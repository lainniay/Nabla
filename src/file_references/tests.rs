use super::*;
use crate::file_references::model::FileReferenceEnvelope;
use crate::file_references::model::MAX_TEXT_FILE;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_segmentation::UnicodeSegmentation;

fn workspace() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("nabla-file-reference-{unique}"));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn parses_tokens_without_triggering_email_or_double_at() {
    let text = "see @src/lib.rs and @{docs/my file.md}; no a@b.com or @@skip";
    let refs = references(text);
    assert_eq!(
        refs.iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>(),
        ["src/lib.rs", "docs/my file.md"]
    );
}

#[test]
fn token_at_unicode_cursor_is_grapheme_safe_and_can_be_open() {
    let text = "你好 @{文档/草稿";
    let token = token_at_cursor(text, text.graphemes(true).count()).unwrap();
    assert_eq!(token.path, "文档/草稿");
    assert!(!token.closed);
}

#[test]
fn completion_braces_paths_with_spaces() {
    assert_eq!(completion_text("docs/a file.md"), "@{docs/a file.md}");
    assert_eq!(completion_text("src/lib.rs"), "@src/lib.rs");
}

#[test]
fn prepares_snapshots_and_deduplicates_normalized_paths() {
    let root = workspace();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "pub mod app;\n").unwrap();
    let service = FileReferenceService::new(root.clone()).unwrap();
    let prepared = service
        .prepare("Review @src/lib.rs and @./src/lib.rs".to_owned())
        .unwrap();
    assert!(prepared.message.starts_with(ENVELOPE_PREFIX));
    let envelope: FileReferenceEnvelope =
        serde_json::from_str(&prepared.message[ENVELOPE_PREFIX.len()..]).unwrap();
    assert_eq!(envelope.message, "Review @src/lib.rs and @./src/lib.rs");
    assert_eq!(envelope.references.len(), 1);
    assert_eq!(envelope.references[0].mode, "snapshot");
    assert_eq!(
        envelope.references[0].content.as_deref(),
        Some("pub mod app;\n")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn binary_and_oversized_text_degrade_to_path_only() {
    let root = workspace();
    fs::write(root.join("binary.dat"), [0xff, 0x00]).unwrap();
    fs::write(
        root.join("large.txt"),
        vec![b'x'; MAX_TEXT_FILE as usize + 1],
    )
    .unwrap();
    let service = FileReferenceService::new(root.clone()).unwrap();
    let prepared = service
        .prepare("Use @binary.dat and @large.txt".to_owned())
        .unwrap();
    let envelope: FileReferenceEnvelope =
        serde_json::from_str(&prepared.message[ENVELOPE_PREFIX.len()..]).unwrap();
    assert_eq!(
        envelope
            .references
            .iter()
            .map(|reference| reference.mode.as_str())
            .collect::<Vec<_>>(),
        ["path", "path"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_files_and_unclosed_braces_block_preparation() {
    let root = workspace();
    let service = FileReferenceService::new(root.clone()).unwrap();
    assert!(service.prepare("Read @missing.txt".to_owned()).is_err());
    assert!(service.prepare("Read @{missing file".to_owned()).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn supported_image_signatures_are_sent_as_pi_images() {
    let root = workspace();
    fs::write(root.join("image.bin"), b"\x89PNG\r\n\x1a\npayload").unwrap();
    let service = FileReferenceService::new(root.clone()).unwrap();
    let prepared = service.prepare("Inspect @image.bin".to_owned()).unwrap();
    assert_eq!(prepared.images.len(), 1);
    assert_eq!(prepared.images[0].mime_type, "image/png");
    assert_eq!(
        serde_json::to_value(&prepared.images[0]).unwrap()["mimeType"],
        "image/png"
    );
    let envelope: FileReferenceEnvelope =
        serde_json::from_str(&prepared.message[ENVELOPE_PREFIX.len()..]).unwrap();
    assert_eq!(envelope.references[0].mode, "image");
    assert!(envelope.references[0].content.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn index_respects_ignores_but_keeps_unignored_hidden_files() {
    let root = workspace();
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(root.join("ignored.txt"), "ignored").unwrap();
    fs::write(root.join(".visible"), "hidden by name only").unwrap();
    fs::write(root.join("shown.txt"), "shown").unwrap();
    let service = FileReferenceService::new(root.clone()).unwrap();
    let all = service.search("").unwrap();
    let paths = all
        .iter()
        .map(|candidate| candidate.path.as_str())
        .collect::<Vec<_>>();
    assert!(!paths.contains(&"ignored.txt"));
    assert!(paths.contains(&".visible"));
    assert!(paths.contains(&"shown.txt"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_envelope_fixture_matches_rust_contract() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../protocol-fixtures/nabla.file-references.v1.json"
    ))
    .unwrap();
    let wire = fixture["wire"].as_str().unwrap();
    assert!(wire.starts_with(ENVELOPE_PREFIX));
    let envelope: FileReferenceEnvelope =
        serde_json::from_str(&wire[ENVELOPE_PREFIX.len()..]).unwrap();
    assert_eq!(envelope.version, 1);
    assert_eq!(envelope.message, fixture["message"]);
    assert_eq!(envelope.references[0].path, "src/lib.rs");
}

#[cfg(unix)]
#[test]
fn preparation_rejects_directories_and_escaping_symlinks() {
    use std::os::unix::fs::symlink;

    let root = workspace();
    fs::create_dir(root.join("folder")).unwrap();
    let outside = root.with_extension("outside");
    fs::write(&outside, "outside").unwrap();
    symlink(&outside, root.join("escape")).unwrap();
    let service = FileReferenceService::new(root.clone()).unwrap();
    assert!(service.prepare("Read @folder".to_owned()).is_err());
    assert!(service.prepare("Read @escape".to_owned()).is_err());
    fs::remove_file(outside).unwrap();
    fs::remove_dir_all(root).unwrap();
}

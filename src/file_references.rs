use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

pub const ENVELOPE_PREFIX: &str = "NABLA_FILE_REFERENCES_V1\n";
const MAX_REFERENCES: usize = 8;
const MAX_INDEX_FILES: usize = 200_000;
const MAX_TEXT_FILE: u64 = 32 * 1024;
const MAX_TEXT_TOTAL: u64 = 128 * 1024;
const MAX_IMAGES: usize = 4;
const MAX_IMAGE_FILE: u64 = 5 * 1024 * 1024;
const MAX_IMAGE_TOTAL: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReferenceToken {
    pub range: std::ops::Range<usize>,
    pub path: String,
    pub braced: bool,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCandidate {
    pub path: String,
    pub basename: String,
    pub parent: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileCompletionState {
    pub query: String,
    pub token_range: std::ops::Range<usize>,
    pub generation: u64,
    pub candidates: Vec<FileCandidate>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDelivery {
    Prompt,
    Steer,
    FollowUp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPrompt {
    pub original_message: String,
    pub message: String,
    pub images: Vec<ImageContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedFileReference {
    pub path: String,
    pub mode: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileReferenceEnvelope {
    version: u8,
    message: String,
    references: Vec<PreparedFileReference>,
}

type FileIndexCache = Arc<Mutex<Option<(Instant, Vec<FileCandidate>)>>>;

#[derive(Clone)]
pub struct FileReferenceService {
    root: PathBuf,
    cache: FileIndexCache,
}

impl FileReferenceService {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("Unable to resolve workspace: {error}"))?;
        Ok(Self {
            root,
            cache: Arc::new(Mutex::new(None)),
        })
    }

    pub fn search(&self, query: &str) -> Result<Vec<FileCandidate>, String> {
        let files = self.index()?;
        let query = query.to_lowercase();
        let mut matches = files
            .into_iter()
            .filter_map(|candidate| match_score(&candidate, &query).map(|score| (score, candidate)))
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            left_score
                .cmp(right_score)
                .then_with(|| path_depth(&left.path).cmp(&path_depth(&right.path)))
                .then_with(|| left.path.to_lowercase().cmp(&right.path.to_lowercase()))
        });
        Ok(matches
            .into_iter()
            .map(|(_, candidate)| candidate)
            .take(50)
            .collect())
    }

    pub fn prepare(&self, message: String) -> Result<PreparedPrompt, String> {
        let tokens = references_including_open(&message);
        if tokens.is_empty() {
            return Ok(PreparedPrompt {
                original_message: message.clone(),
                message,
                images: Vec::new(),
            });
        }
        let mut seen = HashSet::new();
        let mut text_total = 0u64;
        let mut image_total = 0u64;
        let mut images = Vec::new();
        let mut prepared = Vec::new();
        for token in tokens {
            if !token.closed {
                return Err(format!(
                    "Unclosed file reference: {}",
                    &message[token.range]
                ));
            }
            let (relative, canonical, metadata) = self.resolve(&token.path)?;
            if !seen.insert(relative.clone()) {
                continue;
            }
            if prepared.len() >= MAX_REFERENCES {
                return Err(format!(
                    "A prompt may reference at most {MAX_REFERENCES} files"
                ));
            }
            let size = metadata.len();
            if size > MAX_IMAGE_FILE {
                prepared.push(path_only(relative, size, "file size limit exceeded"));
                continue;
            }
            let bytes = fs::read(&canonical)
                .map_err(|error| format!("Unable to read {relative}: {error}"))?;
            if let Some(mime_type) = image_mime(&bytes) {
                if images.len() < MAX_IMAGES
                    && size <= MAX_IMAGE_FILE
                    && image_total.saturating_add(size) <= MAX_IMAGE_TOTAL
                {
                    images.push(ImageContent {
                        kind: "image".to_owned(),
                        data: STANDARD.encode(bytes),
                        mime_type: mime_type.to_owned(),
                    });
                    image_total += size;
                    prepared.push(PreparedFileReference {
                        path: relative,
                        mode: "image".to_owned(),
                        size,
                        reason: None,
                        content: None,
                    });
                } else {
                    prepared.push(path_only(relative, size, "image limit exceeded"));
                }
                continue;
            }
            match String::from_utf8(bytes) {
                Ok(content)
                    if size <= MAX_TEXT_FILE
                        && text_total.saturating_add(size) <= MAX_TEXT_TOTAL =>
                {
                    text_total += size;
                    prepared.push(PreparedFileReference {
                        path: relative,
                        mode: "snapshot".to_owned(),
                        size,
                        reason: None,
                        content: Some(content),
                    });
                }
                Ok(_) => prepared.push(path_only(relative, size, "text limit exceeded")),
                Err(_) => prepared.push(path_only(relative, size, "binary file")),
            }
        }
        let envelope = FileReferenceEnvelope {
            version: 1,
            message: message.clone(),
            references: prepared,
        };
        let json = serde_json::to_string(&envelope)
            .map_err(|error| format!("Unable to encode file references: {error}"))?;
        Ok(PreparedPrompt {
            original_message: message,
            message: format!("{ENVELOPE_PREFIX}{json}"),
            images,
        })
    }

    fn index(&self) -> Result<Vec<FileCandidate>, String> {
        if let Some((created, files)) = self
            .cache
            .lock()
            .map_err(|_| "File index cache is unavailable".to_owned())?
            .as_ref()
            && created.elapsed() < Duration::from_secs(2)
        {
            return Ok(files.clone());
        }
        let mut files = Vec::new();
        let walker = WalkBuilder::new(&self.root)
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        for entry in walker {
            let entry = entry.map_err(|error| format!("Unable to index workspace: {error}"))?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let metadata = entry.metadata().map_err(|error| {
                format!("Unable to inspect {}: {error}", entry.path().display())
            })?;
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| "Indexed path escaped workspace".to_owned())?;
            let path = slash_path(relative);
            let basename = entry.file_name().to_string_lossy().into_owned();
            let parent = relative
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .map(slash_path)
                .unwrap_or_default();
            files.push(FileCandidate {
                path,
                basename,
                parent,
                size: metadata.len(),
            });
            if files.len() >= MAX_INDEX_FILES {
                break;
            }
        }
        *self
            .cache
            .lock()
            .map_err(|_| "File index cache is unavailable".to_owned())? =
            Some((Instant::now(), files.clone()));
        Ok(files)
    }

    fn resolve(&self, input: &str) -> Result<(String, PathBuf, fs::Metadata), String> {
        let path = Path::new(input);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(format!("File reference is outside the workspace: {input}"));
        }
        let candidate = self.root.join(path);
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("Unable to resolve {input}: {error}"))?;
        if !canonical.starts_with(&self.root) {
            return Err(format!("File reference escapes the workspace: {input}"));
        }
        let metadata = canonical
            .metadata()
            .map_err(|error| format!("Unable to inspect {input}: {error}"))?;
        if !metadata.is_file() {
            return Err(format!("File reference is not a regular file: {input}"));
        }
        let relative = canonical
            .strip_prefix(&self.root)
            .map(slash_path)
            .map_err(|_| format!("File reference escapes the workspace: {input}"))?;
        Ok((relative, canonical, metadata))
    }
}

pub fn token_at_cursor(text: &str, grapheme_cursor: usize) -> Option<FileReferenceToken> {
    let cursor = text
        .grapheme_indices(true)
        .nth(grapheme_cursor)
        .map_or(text.len(), |(index, _)| index);
    references_including_open(text)
        .into_iter()
        .find(|token| token.range.start <= cursor && cursor <= token.range.end)
}

pub fn references(text: &str) -> Vec<FileReferenceToken> {
    references_including_open(text)
        .into_iter()
        .filter(|token| token.closed && !token.path.is_empty())
        .collect()
}

fn references_including_open(text: &str) -> Vec<FileReferenceToken> {
    let mut result = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'@'
            || bytes.get(index + 1) == Some(&b'@')
            || (index > 0
                && !text[..index]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace))
        {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        let braced = bytes.get(index) == Some(&b'{');
        if braced {
            index += 1;
            let path_start = index;
            while index < bytes.len() && bytes[index] != b'}' && bytes[index] != b'\n' {
                index += 1;
            }
            let closed = bytes.get(index) == Some(&b'}');
            let path = text[path_start..index].to_owned();
            if closed {
                index += 1;
            }
            result.push(FileReferenceToken {
                range: start..index,
                path,
                braced,
                closed,
            });
        } else {
            let path_start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let path = text[path_start..index].to_owned();
            result.push(FileReferenceToken {
                range: start..index,
                path,
                braced: false,
                closed: true,
            });
        }
    }
    result
}

pub fn reference_tokens(text: &str) -> Vec<FileReferenceToken> {
    references_including_open(text)
}

pub fn completion_text(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("@{{{path}}}")
    } else {
        format!("@{path}")
    }
}

fn path_only(path: String, size: u64, reason: &str) -> PreparedFileReference {
    PreparedFileReference {
        path,
        mode: "path".to_owned(),
        size,
        reason: Some(reason.to_owned()),
        content: None,
    }
}

fn match_score(candidate: &FileCandidate, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(5);
    }
    let basename = candidate.basename.to_lowercase();
    let path = candidate.path.to_lowercase();
    if basename == query {
        Some(0)
    } else if basename.starts_with(query) {
        Some(1)
    } else if path.split('/').any(|segment| segment.starts_with(query)) {
        Some(2)
    } else if path.contains(query) {
        Some(3)
    } else if is_subsequence(query, &path) {
        Some(4)
    } else {
        None
    }
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|needle| chars.by_ref().any(|hay| hay == needle))
}

fn path_depth(path: &str) -> usize {
    path.bytes().filter(|byte| *byte == b'/').count()
}

fn slash_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
            "../protocol-fixtures/nabla.file-references.v1.json"
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
}

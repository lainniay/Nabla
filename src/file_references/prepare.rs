use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::file_references::index::FileReferenceService;
use crate::file_references::matcher::slash_path;
use crate::file_references::model::{
    ENVELOPE_PREFIX, FileReferenceEnvelope, ImageContent, MAX_IMAGE_FILE, MAX_IMAGE_TOTAL,
    MAX_IMAGES, MAX_REFERENCES, MAX_TEXT_FILE, MAX_TEXT_TOTAL, PreparedFileReference,
    PreparedPrompt,
};
use crate::file_references::parser::references_including_open;

impl FileReferenceService {
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
fn path_only(path: String, size: u64, reason: &str) -> PreparedFileReference {
    PreparedFileReference {
        path,
        mode: "path".to_owned(),
        size,
        reason: Some(reason.to_owned()),
        content: None,
    }
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

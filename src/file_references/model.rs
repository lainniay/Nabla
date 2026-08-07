use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

pub const ENVELOPE_PREFIX: &str = "NABLA_FILE_REFERENCES_V1\n";
pub(crate) const MAX_REFERENCES: usize = 8;
pub(crate) const MAX_INDEX_FILES: usize = 200_000;
pub(crate) const MAX_TEXT_FILE: u64 = 32 * 1024;
pub(crate) const MAX_TEXT_TOTAL: u64 = 128 * 1024;
pub(crate) const MAX_IMAGES: usize = 4;
pub(crate) const MAX_IMAGE_FILE: u64 = 5 * 1024 * 1024;
pub(crate) const MAX_IMAGE_TOTAL: u64 = 10 * 1024 * 1024;

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
pub(crate) struct FileReferenceEnvelope {
    pub(crate) version: u8,
    pub(crate) message: String,
    pub(crate) references: Vec<PreparedFileReference>,
}

pub(crate) type FileIndexCache = Arc<Mutex<Option<(Instant, Vec<FileCandidate>)>>>;

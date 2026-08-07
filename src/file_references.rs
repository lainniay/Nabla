pub mod index;
pub mod matcher;
pub mod model;
pub mod parser;
pub mod prepare;

#[cfg(test)]
mod tests;

pub use index::FileReferenceService;
pub use model::{
    ENVELOPE_PREFIX, FileCandidate, FileCompletionState, FileReferenceToken, ImageContent,
    PreparedFileReference, PreparedPrompt, PromptDelivery,
};
pub use parser::{completion_text, reference_tokens, references, token_at_cursor};

//! Talks to the language model. The app only ever uses the local Ollama server (see `ollama.rs`);
//! the `Provider` trait keeps the explainer independent of it and lets tests use a fake.

pub mod ollama;

use std::fmt;

/// Model used until the user picks another one in settings (Task 7).
pub const DEFAULT_MODEL: &str = "qwen3.5:4b";

/// One question to the model: a system prompt and the user's text.
#[derive(Debug, Clone, Copy)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub user: &'a str,
    /// Low for explanations (steady, factual), higher for the greeting (a new one every time).
    pub temperature: f64,
}

pub trait Provider: Send + Sync {
    /// Checks that the server is running and the model is installed.
    fn health(&self, model: &str) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// Streams the answer. `on_chunk` gets each piece of text as it arrives.
    /// Dropping the returned future cancels the request.
    fn stream_chat(
        &self,
        req: ChatRequest<'_>,
        on_chunk: impl FnMut(&str) + Send,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;
}

/// Errors with messages simple enough to show in the popup as they are.
/// They never contain the selected text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    NotRunning,
    ModelMissing(String),
    Timeout,
    /// An error message from Ollama itself.
    Ollama(String),
    BadResponse,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => write!(f, "Ollama is not running. Start Ollama, then try again."),
            Self::ModelMissing(model) => {
                write!(f, "Model {model} is not installed. Run: ollama pull {model}")
            }
            Self::Timeout => write!(f, "Ollama took too long to answer."),
            Self::Ollama(msg) => write!(f, "Ollama error: {msg}"),
            Self::BadResponse => write!(f, "Unexpected answer from Ollama."),
        }
    }
}

impl std::error::Error for ProviderError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_has_a_message() {
        let errors = [
            ProviderError::NotRunning,
            ProviderError::ModelMissing("m".into()),
            ProviderError::Timeout,
            ProviderError::Ollama("x".into()),
            ProviderError::BadResponse,
        ];
        for err in errors {
            assert!(!err.to_string().trim().is_empty(), "{err:?} has no message");
        }
    }

    #[test]
    fn missing_model_message_says_how_to_install_it() {
        let msg = ProviderError::ModelMissing(DEFAULT_MODEL.into()).to_string();
        assert!(msg.contains("ollama pull qwen3.5:4b"), "{msg}");
    }
}

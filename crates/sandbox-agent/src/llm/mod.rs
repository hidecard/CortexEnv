//! Provider-neutral language-model interfaces and provider adapters.

mod openai_compatible;

pub use openai_compatible::OpenAiCompatibleClient;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Role of a message in a chat conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// Instruction supplied by the application.
    System,
    /// Input supplied by the user or an execution tool.
    User,
    /// Text produced by the language model.
    Assistant,
}

/// A single text message exchanged with a chat model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Message speaker.
    pub role: ChatRole,
    /// Verbatim message text.
    pub content: String,
}

impl ChatMessage {
    /// Construct a system instruction.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    /// Construct a user or tool-result message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    /// Construct an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

/// Errors returned by a language-model provider.
///
/// Error values deliberately omit request URLs, response bodies, and credentials.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LlmError {
    /// Client-side provider configuration is invalid.
    #[error("LLM provider configuration is invalid")]
    InvalidConfiguration,
    /// The request could not be completed.
    #[error("LLM provider request failed")]
    RequestFailed,
    /// The provider rejected the request with an HTTP status.
    #[error("LLM provider returned HTTP status {0}")]
    HttpStatus(u16),
    /// The provider response did not contain usable assistant text.
    #[error("LLM provider returned an invalid response")]
    InvalidResponse,
}

/// Provider-neutral interface for a chat completion model.
#[async_trait]
pub trait ChatModel: Send + Sync {
    /// Return one assistant response for the supplied conversation.
    async fn complete(&self, messages: &[ChatMessage]) -> Result<ChatMessage, LlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_messages_use_openai_compatible_role_names() {
        let message = ChatMessage::assistant("answer");
        let encoded = serde_json::to_value(&message).unwrap();
        assert_eq!(encoded["role"], "assistant");
        assert_eq!(encoded["content"], "answer");
        assert_eq!(
            serde_json::from_value::<ChatMessage>(encoded).unwrap(),
            message
        );
    }
}

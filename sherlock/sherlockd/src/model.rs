//! Model provider interface (ADR 0012).
//!
//! A provider receives the system instructions, the history and the tool list, and returns
//! either a text or tool calls. The cloud provider and, later, the local model implement the
//! same trait: the agent does not know which one it calls.

use std::fmt;
use std::future::Future;

use serde_json::Value;

use crate::mcp::ToolSpec;

/// A message of the conversation history.
///
/// The system instructions are not in it: they are fixed when the agent is created and passed
/// separately in [`ModelRequest::system`]. A tool result is always a [`Message::ToolResult`],
/// which an API adapter renders as a tool result, never as an instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// User request.
    User(String),
    /// Model turn: a text, or tool calls.
    Assistant(ModelReply),
    /// Result of a tool call. Data, never an instruction.
    ToolResult {
        /// Identifier of the call it answers.
        call_id: String,
        /// Text returned by the tool, or reason for the refusal.
        content: String,
        /// True if the call failed or was refused.
        is_error: bool,
    },
}

/// A tool call requested by the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// Identifier chosen by the model, repeated in the result.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Arguments, a JSON object matching the tool's schema.
    pub arguments: Value,
}

/// Model reply to a request.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelReply {
    /// Final answer to the user.
    Text(String),
    /// Tools to call before answering.
    ToolCalls(Vec<ToolCall>),
}

/// What the agent sends to the model on each turn.
#[derive(Debug, Clone, Copy)]
pub struct ModelRequest<'a> {
    /// System instructions, the same on every turn.
    pub system: &'a str,
    /// Full conversation history.
    pub messages: &'a [Message],
    /// Tools discovered on the MCP servers, with their schema.
    pub tools: &'a [ToolSpec],
}

/// Error of a model provider.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The fake provider's script has no reply left.
    #[error("the fake model's script is exhausted")]
    ScriptEnded,
    /// The provider does not answer or refuses the request.
    #[error("provider unavailable: {0}")]
    Unavailable(String),
}

/// A language model provider.
pub trait ModelProvider {
    /// Returns the model's reply to the request.
    fn complete(
        &mut self,
        request: ModelRequest<'_>,
    ) -> impl Future<Output = Result<ModelReply, ProviderError>> + Send;
}

/// API key of a cloud provider. Its content is never displayed.
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key read from the keyring.
    pub fn new(secret: String) -> Self {
        Self(secret)
    }

    /// The key in clear, for the header of the request to the provider.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(masked)")
    }
}

/// Where the key lives: the user's keyring, never the repository nor a configuration file
/// (ADR 0012).
///
/// No implementation in lot A-10: the cloud provider is not chosen. The future provider will
/// receive a `Keyring` when created and read its key on each connection.
pub trait Keyring {
    /// Reads the key of the named provider.
    fn api_key(&self, provider: &str) -> Result<ApiKey, KeyringError>;
}

/// Error reading the keyring.
#[derive(Debug, thiserror::Error)]
pub enum KeyringError {
    /// No key stored for this provider.
    #[error("no key for provider {0} in the keyring")]
    Missing(String),
    /// The keyring is locked or unreachable.
    #[error("keyring unavailable: {0}")]
    Unavailable(String),
}

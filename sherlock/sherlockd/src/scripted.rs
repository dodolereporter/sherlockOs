//! Fake model provider, driven by a script, for the tests and `--demo`.

use std::collections::VecDeque;

use crate::model::{Message, ModelProvider, ModelReply, ModelRequest, ProviderError};

type Step = Box<dyn FnMut(&ModelRequest<'_>) -> ModelReply + Send>;

/// A request received by the fake model, kept for the tests to inspect.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// System instructions received.
    pub system: String,
    /// History received.
    pub messages: Vec<Message>,
    /// Names of the offered tools.
    pub tools: Vec<String>,
}

/// Returns, turn after turn, the replies of its script.
#[derive(Default)]
pub struct ScriptedProvider {
    steps: VecDeque<Step>,
    /// Every request received, in order.
    pub requests: Vec<RecordedRequest>,
}

impl ScriptedProvider {
    /// An empty script.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a fixed reply.
    pub fn reply(self, reply: ModelReply) -> Self {
        self.then(move |_| reply.clone())
    }

    /// Adds a reply computed from the request, for example from the last tool result.
    pub fn then(
        mut self,
        step: impl FnMut(&ModelRequest<'_>) -> ModelReply + Send + 'static,
    ) -> Self {
        self.steps.push_back(Box::new(step));
        self
    }
}

impl ModelProvider for ScriptedProvider {
    async fn complete(&mut self, request: ModelRequest<'_>) -> Result<ModelReply, ProviderError> {
        self.requests.push(RecordedRequest {
            system: request.system.to_owned(),
            messages: request.messages.to_vec(),
            tools: request.tools.iter().map(|t| t.name.clone()).collect(),
        });
        let mut step = self.steps.pop_front().ok_or(ProviderError::ScriptEnded)?;
        Ok(step(&request))
    }
}

/// Text of the last tool result in the history, for scripts that reuse it.
pub fn last_tool_result(request: &ModelRequest<'_>) -> Option<String> {
    request.messages.iter().rev().find_map(|m| match m {
        Message::ToolResult { content, .. } => Some(content.clone()),
        _ => None,
    })
}

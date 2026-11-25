//! Sherlock daemon: conversation, model calls, tool chaining.
//!
//! - [`model`]: the model provider interface and where its key lives.
//! - [`scripted`]: a fake provider driven by a script.
//! - [`mcp`]: the MCP client over stdio and tool levels.
//! - [`agent`]: the agent loop and its safety rules.
//! - [`proxy`]: `sherlockd` as an MCP server, in front of Sherlock's servers.
//! - [`claude_code`]: the provider that hands the conversation to Claude Code.
//! - [`test_server`]: a test MCP server, with no effect on the system.
//!
//! Architecture and rules: `docs/sherlockd.md`.

pub mod agent;
pub mod claude_code;
pub mod mcp;
pub mod model;
pub mod proxy;
pub mod scripted;
pub mod test_server;

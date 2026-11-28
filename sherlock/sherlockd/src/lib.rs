//! Sherlock daemon: conversation, model calls, tool chaining.
//!
//! - [`model`]: the model provider interface and where its key lives.
//! - [`scripted`]: a fake provider driven by a script.
//! - [`mcp`]: the MCP client over stdio and tool levels.
//! - [`agent`]: the agent loop and its safety rules.
//! - [`broker`]: the consent broker, its permits and its journal (A-11).
//! - [`proxy`]: `sherlockd` as an MCP server, in front of Sherlock's servers.
//! - [`claude_code`]: the provider that hands the conversation to Claude Code.
//! - [`process`]: started processes never outlive sherlockd.
//! - [`services_server`]: the MCP server of the systemd units and their journal (A-16).
//! - [`settings_server`]: the MCP server of the settings, by Control Room's functions (A-17).
//! - [`test_server`]: a test MCP server, with no effect on the system.
//! - [`window`]: the compositor's consent window, which the broker asks (A-11c).
//! - [`containers_server`], [`docker`]: the MCP server of the containers, on the Docker API (A-14).
//! - [`files_server`], [`folders`]: the MCP server of the files, in the allowed folders (A-13).
//! - [`own_server`]: what Sherlock's own MCP servers share.
//! - [`windows_server`]: the MCP server of the session's windows (A-12).
//!
//! Architecture and rules: `docs/sherlockd.md`.

pub mod agent;
pub mod broker;
pub mod claude_code;
pub mod containers_server;
pub mod docker;
pub mod files_server;
pub mod folders;
pub mod mcp;
pub mod model;
pub mod own_server;
pub mod process;
pub mod proxy;
pub mod scripted;
pub mod services_server;
pub mod settings_server;
pub mod shell;
pub mod test_server;
pub mod window;
pub mod windows_server;

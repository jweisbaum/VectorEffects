//! Versioned local MCP integration contract. Never call installers from an MCP tool.
#![forbid(unsafe_code)]
pub mod bridge;
pub mod endpoint;
pub mod install;
pub mod jobs;
pub mod policy;
mod text;
pub const CONTRACT_VERSION: u32 = 1;

pub mod files;
/// Native launcher for this build's target; packaged with every assistant plugin.
pub const LAUNCHER: &[u8] = include_bytes!(env!("WHIRLWIND_NATIVE_LAUNCHER"));

mod process;

pub mod results;

mod http;

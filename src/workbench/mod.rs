//! Shared serial sessions. Only the device actor reads the physical port.
pub mod client;
pub mod http;
pub mod mcp;
mod model;
pub mod scripts;
mod session;
mod storage;
mod task_history;
mod tasks;

pub use model::*;
pub use session::{Hub, Session};

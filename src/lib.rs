//! Serial Workbench library.
//!
//! Shared serial capture, browser UI, persistent tasks and scripts, CLI and MCP.
//! The `serial_mcp_server` library name and direct-access APIs are retained for compatibility.

pub mod automation;
pub mod cli;
pub mod config;
pub mod error;
pub mod recording;
pub mod serial;
pub mod session;
pub mod tools;
pub mod utils;
pub mod workbench;

// Re-export main types for convenience
pub use config::{Args, Config};
pub use error::{Result, SerialError};
pub use recording::{RecordingManager, RecordingOptions, RecordingStatus, RecordingTail};
pub use serial::{ConnectionManager, PortInfo, SerialConnection};
pub use session::{SerialSession, SessionManager, SessionState};
pub use tools::SerialHandler;
pub use utils::{DataConverter, DataFormat, PortType};

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Library name
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// Library description
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

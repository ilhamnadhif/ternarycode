//! How TernaryCode learns what agents are doing (PRD section 8): the PTY's foreground process,
//! hook messages sent by `ternarycode-hook` over a private Unix socket (a named pipe on
//! Windows), and the hook setup for
//! each agent (flags for one launch, or hooks installed in the agent's configuration).

pub mod client;
pub mod integrate;
pub mod proc;
pub mod protocol;
pub mod server;
pub mod sessions;
pub mod shell;

/// Environment variables every pane gets, read by `ternarycode-hook`.
pub const ENV_SOCK: &str = "TERNARYCODE_SOCK";
pub const ENV_PANE: &str = "TERNARYCODE_PANE";
pub const ENV_TOKEN: &str = "TERNARYCODE_TOKEN";

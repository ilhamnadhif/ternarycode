//! TernaryCode's core model, independent of the UI: projects and panes as saved on disk, agent
//! presets and recognition, the per-pane status state machine, the attention queue, and atomic
//! file writes.

pub mod agent;
pub mod attention;
pub mod model;
pub mod persist;
pub mod status;

pub type PaneId = u64;
pub type ProjectId = u64;

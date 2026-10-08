//! Terminal emulation core shared by all backends (UI independent).

pub mod osc;
pub mod palette;
mod shared;

pub use shared::{
    GridSize, Listener, Prompt, PromptKind, PtyIo, RemoteDir, Shared, Status, TermHandle,
};

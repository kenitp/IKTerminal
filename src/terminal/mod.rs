//! Terminal emulation core shared by all backends (UI independent).

pub mod palette;
mod shared;

pub use shared::{GridSize, Listener, Prompt, PromptKind, PtyIo, Shared, Status, TermHandle};

//! OpenSSH client config: evaluation (`resolve`) and editing (`document`).

mod document;
mod resolve;

pub use document::{Document, Line, find_option, push_option};
pub use resolve::{HostConfig, HostEntry, SshConfig, ssh_dir};

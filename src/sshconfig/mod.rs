//! OpenSSH client config: evaluation (`resolve`) and editing (`document`).

mod command;
mod document;
mod resolve;

pub use command::{HostDraft, SshCommand, find_ssh_command, save_host};
pub use document::{Document, Line, find_option, push_option, write_file};
pub use resolve::{HostConfig, HostEntry, SshConfig, ssh_dir};

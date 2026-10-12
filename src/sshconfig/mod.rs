//! OpenSSH client config: evaluation (`resolve`) and editing (`document`).

mod command;
mod document;
mod resolve;

pub use command::{
    HostDraft, SshCommand, SshOutcome, apply_connected_host, find_ssh_command, save_host,
    ssh_outcome,
};
pub use document::{Document, Line, find_option, push_option, write_file};
pub use resolve::{HostConfig, HostEntry, SshConfig, ssh_dir};

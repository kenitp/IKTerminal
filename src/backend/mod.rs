//! Session I/O backends: local PTY, SSH shell and SFTP.

pub mod local;
pub mod sftp;
pub mod ssh;

use std::sync::OnceLock;

/// Shared async runtime for all network I/O (kept small on purpose).
pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ikterm-io")
            .enable_all()
            .build()
            .expect("failed to start async runtime")
    })
}

//! SFTP over an established SSH connection: browsing and file transfers.
//!
//! All operations run on the async runtime and publish their results into
//! shared state that the UI reads every frame.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::OnceCell;

use super::runtime;
use super::ssh::Link;

const CHUNK: usize = 256 * 1024;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Default)]
pub struct Listing {
    pub cwd: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransferState {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

pub struct Transfer {
    pub label: String,
    pub upload: bool,
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub state: Mutex<TransferState>,
    cancel: AtomicBool,
}

impl Transfer {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn state(&self) -> TransferState {
        self.state.lock().unwrap().clone()
    }

    fn check(&self) -> Result<(), TransferError> {
        if self.cancel.load(Ordering::Relaxed) { Err(TransferError::Cancelled) } else { Ok(()) }
    }
}

enum TransferError {
    Cancelled,
    Failed(String),
}

impl From<String> for TransferError {
    fn from(e: String) -> Self {
        TransferError::Failed(e)
    }
}

#[derive(Clone)]
pub struct SftpClient {
    link: Link,
    session: Arc<OnceCell<Arc<SftpSession>>>,
    pub listing: Arc<Mutex<Listing>>,
    pub transfers: Arc<Mutex<Vec<Arc<Transfer>>>>,
    ctx: egui::Context,
}

impl SftpClient {
    pub fn new(link: Link, ctx: egui::Context) -> Self {
        Self {
            link,
            session: Arc::default(),
            listing: Arc::default(),
            transfers: Arc::default(),
            ctx,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.link.get().is_some()
    }

    async fn sftp(&self) -> Result<Arc<SftpSession>, String> {
        self.session
            .get_or_try_init(|| async {
                let conn = self.link.get().ok_or("SSH 接続が確立していません")?.clone();
                let channel = conn.handle.channel_open_session().await.map_err(|e| e.to_string())?;
                channel.request_subsystem(true, "sftp").await.map_err(|e| e.to_string())?;
                let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| e.to_string())?;
                Ok::<_, String>(Arc::new(sftp))
            })
            .await
            .cloned()
    }

    pub fn cwd(&self) -> String {
        self.listing.lock().unwrap().cwd.clone()
    }

    /// Lists `path` (relative paths are resolved by the server).
    pub fn open_dir(&self, path: String) {
        self.listing.lock().unwrap().loading = true;
        let this = self.clone();
        runtime().spawn(async move {
            let result = this.read_dir(&path).await;
            let mut listing = this.listing.lock().unwrap();
            listing.loading = false;
            match result {
                Ok((cwd, entries)) => {
                    listing.cwd = cwd;
                    listing.entries = entries;
                    listing.error = None;
                }
                Err(e) => listing.error = Some(e),
            }
            drop(listing);
            this.ctx.request_repaint();
        });
    }

    pub fn refresh(&self) {
        let cwd = self.cwd();
        self.open_dir(if cwd.is_empty() { ".".to_owned() } else { cwd });
    }

    async fn read_dir(&self, path: &str) -> Result<(String, Vec<Entry>), String> {
        let sftp = self.sftp().await?;
        let cwd = sftp.canonicalize(path).await.map_err(|e| e.to_string())?;
        let mut entries: Vec<Entry> = sftp
            .read_dir(cwd.as_str())
            .await
            .map_err(|e| e.to_string())?
            .filter(|e| e.file_name() != "." && e.file_name() != "..")
            .map(|e| {
                let meta = e.metadata();
                Entry {
                    name: e.file_name(),
                    is_dir: e.file_type().is_dir(),
                    size: meta.len(),
                }
            })
            .collect();
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok((cwd, entries))
    }

    /// Runs a simple remote operation and refreshes the listing afterwards.
    fn run_op<F>(&self, op: F)
    where
        F: for<'a> FnOnce(&'a SftpSession) -> BoxFuture<'a, Result<(), russh_sftp::client::error::Error>> + Send + 'static,
    {
        let this = self.clone();
        runtime().spawn(async move {
            let result = match this.sftp().await {
                Ok(sftp) => op(&sftp).await.map_err(|e| e.to_string()),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                this.listing.lock().unwrap().error = Some(e);
            }
            this.refresh();
        });
    }

    pub fn mkdir(&self, name: &str) {
        let path = join(&self.cwd(), name);
        self.run_op(move |s| Box::pin(s.create_dir(path)));
    }

    pub fn rename(&self, from: &str, to: &str) {
        let cwd = self.cwd();
        let (from, to) = (join(&cwd, from), join(&cwd, to));
        self.run_op(move |s| Box::pin(s.rename(from, to)));
    }

    pub fn remove(&self, entry: &Entry) {
        let path = join(&self.cwd(), &entry.name);
        let is_dir = entry.is_dir;
        self.run_op(move |s| {
            Box::pin(async move { if is_dir { remove_tree(s, path).await } else { s.remove_file(path).await } })
        });
    }

    fn start_transfer(&self, label: String, upload: bool) -> Arc<Transfer> {
        let t = Arc::new(Transfer {
            label,
            upload,
            total: AtomicU64::new(0),
            done: AtomicU64::new(0),
            state: Mutex::new(TransferState::Running),
            cancel: AtomicBool::new(false),
        });
        self.transfers.lock().unwrap().push(t.clone());
        self.ctx.request_repaint();
        t
    }

    fn finish_transfer(&self, t: &Transfer, result: Result<(), TransferError>) {
        *t.state.lock().unwrap() = match result {
            Ok(()) => TransferState::Done,
            Err(TransferError::Cancelled) => TransferState::Cancelled,
            Err(TransferError::Failed(e)) => TransferState::Failed(e),
        };
        self.ctx.request_repaint();
    }

    pub fn clear_finished(&self) {
        self.transfers.lock().unwrap().retain(|t| t.state() == TransferState::Running);
    }

    /// Uploads local files or directories into the current remote directory.
    pub fn upload(&self, paths: Vec<PathBuf>) {
        let remote_dir = self.cwd();
        for local in paths {
            let Some(name) = local.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            let t = self.start_transfer(name.clone(), true);
            let this = self.clone();
            let remote = join(&remote_dir, &name);
            runtime().spawn(async move {
                t.total.store(local_size(&local), Ordering::Relaxed);
                let result = match this.sftp().await {
                    Ok(sftp) => this.upload_path(&sftp, &local, remote, &t).await,
                    Err(e) => Err(e.into()),
                };
                this.finish_transfer(&t, result);
                this.refresh();
            });
        }
    }

    fn upload_path<'a>(
        &'a self,
        sftp: &'a SftpSession,
        local: &'a Path,
        remote: String,
        t: &'a Transfer,
    ) -> BoxFuture<'a, Result<(), TransferError>> {
        Box::pin(async move {
            t.check()?;
            if local.is_dir() {
                if !sftp.try_exists(remote.as_str()).await.unwrap_or(false) {
                    sftp.create_dir(remote.as_str()).await.map_err(|e| format!("{remote}: {e}"))?;
                }
                let mut children = tokio::fs::read_dir(local).await.map_err(|e| e.to_string())?;
                while let Some(child) = children.next_entry().await.map_err(|e| e.to_string())? {
                    let name = child.file_name().to_string_lossy().into_owned();
                    self.upload_path(sftp, &child.path(), join(&remote, &name), t).await?;
                }
                return Ok(());
            }
            let mut src = tokio::fs::File::open(local).await.map_err(|e| format!("{}: {e}", local.display()))?;
            let mut dst = sftp.create(remote.as_str()).await.map_err(|e| format!("{remote}: {e}"))?;
            self.pump(&mut src, &mut dst, t).await?;
            Ok(dst.shutdown().await.map_err(|e| e.to_string())?)
        })
    }

    /// Downloads remote entries of the current directory into `local_dir`.
    pub fn download(&self, entries: Vec<Entry>, local_dir: PathBuf) {
        let remote_dir = self.cwd();
        for entry in entries {
            let t = self.start_transfer(entry.name.clone(), false);
            let this = self.clone();
            let remote = join(&remote_dir, &entry.name);
            let local = local_dir.join(&entry.name);
            runtime().spawn(async move {
                let result = match this.sftp().await {
                    Ok(sftp) => {
                        t.total.store(remote_size(&sftp, remote.clone(), entry.is_dir).await, Ordering::Relaxed);
                        this.download_path(&sftp, remote, entry.is_dir, &local, &t).await
                    }
                    Err(e) => Err(e.into()),
                };
                this.finish_transfer(&t, result);
            });
        }
    }

    fn download_path<'a>(
        &'a self,
        sftp: &'a SftpSession,
        remote: String,
        is_dir: bool,
        local: &'a Path,
        t: &'a Transfer,
    ) -> BoxFuture<'a, Result<(), TransferError>> {
        Box::pin(async move {
            t.check()?;
            if is_dir {
                tokio::fs::create_dir_all(local).await.map_err(|e| format!("{}: {e}", local.display()))?;
                let children = sftp.read_dir(remote.as_str()).await.map_err(|e| format!("{remote}: {e}"))?;
                for child in children.filter(|c| c.file_name() != "." && c.file_name() != "..") {
                    let name = child.file_name();
                    let child_local = local.join(&name);
                    self.download_path(sftp, join(&remote, &name), child.file_type().is_dir(), &child_local, t).await?;
                }
                return Ok(());
            }
            let mut src = sftp.open(remote.as_str()).await.map_err(|e| format!("{remote}: {e}"))?;
            let mut dst =
                tokio::fs::File::create(local).await.map_err(|e| format!("{}: {e}", local.display()))?;
            self.pump(&mut src, &mut dst, t).await?;
            Ok(dst.flush().await.map_err(|e| e.to_string())?)
        })
    }

    async fn pump<R, W>(&self, src: &mut R, dst: &mut W, t: &Transfer) -> Result<(), TransferError>
    where
        R: tokio::io::AsyncRead + Unpin,
        W: tokio::io::AsyncWrite + Unpin,
    {
        let mut buf = vec![0u8; CHUNK];
        loop {
            t.check()?;
            let n = src.read(&mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(());
            }
            dst.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
            t.done.fetch_add(n as u64, Ordering::Relaxed);
            self.ctx.request_repaint();
        }
    }
}

/// POSIX path join for remote paths.
pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() || name.starts_with('/') {
        name.to_owned()
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Parent of a remote POSIX path.
pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".to_owned(),
        Some((p, _)) => p.to_owned(),
    }
}

fn local_size(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path)
            .map(|rd| rd.flatten().map(|e| local_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn remote_size(sftp: &SftpSession, path: String, is_dir: bool) -> BoxFuture<'_, u64> {
    Box::pin(async move {
        if !is_dir {
            return sftp.metadata(path.as_str()).await.map(|m| m.len()).unwrap_or(0);
        }
        let Ok(children) = sftp.read_dir(path.as_str()).await else { return 0 };
        let mut total = 0;
        for c in children.filter(|c| c.file_name() != "." && c.file_name() != "..") {
            total += if c.file_type().is_dir() {
                remote_size(sftp, join(&path, &c.file_name()), true).await
            } else {
                c.metadata().len()
            };
        }
        total
    })
}

fn remove_tree(sftp: &SftpSession, path: String) -> BoxFuture<'_, Result<(), russh_sftp::client::error::Error>> {
    Box::pin(async move {
        for c in sftp.read_dir(path.as_str()).await? {
            let name = c.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child = join(&path, &name);
            if c.file_type().is_dir() {
                remove_tree(sftp, child).await?;
            } else {
                sftp.remove_file(child).await?;
            }
        }
        sftp.remove_dir(path).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_paths() {
        assert_eq!(join("/home/a", "b"), "/home/a/b");
        assert_eq!(join("/", "b"), "/b");
        assert_eq!(parent("/home/a"), "/home");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/"), "/");
    }
}

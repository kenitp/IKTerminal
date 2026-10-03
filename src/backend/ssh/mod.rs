//! SSH sessions: connection (with ProxyJump), interactive shell channel.

mod auth;
mod handler;

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::vte::ansi::Processor;
use russh::client::{self, Handle};
use russh::{ChannelMsg, Disconnect};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::sshconfig::HostConfig;
use crate::terminal::{PtyIo, Shared, Status, TermHandle};
use handler::Client;

const DEFAULT_CONNECT_TIMEOUT: u64 = 15;

/// An authenticated SSH connection and the jump hosts it is tunneled through.
pub struct Connection {
    pub handle: Handle<Client>,
    _jumps: Vec<Handle<Client>>,
}

/// Set once the connection is established; used to open SFTP channels.
pub type Link = Arc<OnceLock<Arc<Connection>>>;

enum Command {
    Input(Vec<u8>),
    Resize(WindowSize),
    Shutdown,
}

struct SshIo(mpsc::UnboundedSender<Command>);

impl PtyIo for SshIo {
    fn write(&self, data: Cow<'static, [u8]>) {
        let _ = self.0.send(Command::Input(data.into_owned()));
    }

    fn resize(&self, size: WindowSize) {
        let _ = self.0.send(Command::Resize(size));
    }

    fn shutdown(&self) {
        let _ = self.0.send(Command::Shutdown);
    }
}

/// Connects in the background and runs an interactive shell in `term`.
pub fn spawn(shared: Arc<Shared>, term: TermHandle, target: HostConfig, jumps: Vec<HostConfig>) -> Link {
    let link = Link::default();
    let (tx, rx) = mpsc::unbounded_channel();
    shared.set_io(Box::new(SshIo(tx)));
    let task_link = link.clone();
    super::runtime().spawn(async move {
        let result = match connect(&shared, &target, &jumps).await {
            Ok(conn) => {
                let conn = Arc::new(conn);
                let _ = task_link.set(conn.clone());
                run_shell(&shared, &term, &conn, rx).await.map_err(|e| e.to_string())
            }
            Err(e) => Err(e),
        };
        let message = match result {
            Ok(()) => "接続が終了しました".to_owned(),
            Err(e) => format!("接続エラー: {e}"),
        };
        shared.set_status(Status::Exited(message));
    });
    link
}

fn client_config(host: &HostConfig) -> Arc<client::Config> {
    Arc::new(client::Config {
        keepalive_interval: host.server_alive_interval.map(Duration::from_secs),
        keepalive_max: 3,
        ..Default::default()
    })
}

/// Opens a TCP (or tunneled) connection to `host` and authenticates.
async fn open(shared: &Arc<Shared>, host: &HostConfig, via: Option<&Handle<Client>>) -> Result<Handle<Client>, String> {
    let client = Client::new(shared.clone(), &host.hostname, host.port);
    let rejection = client.rejection.clone();
    let config = client_config(host);
    let target = format!("{}:{}", host.hostname, host.port);
    let fail = |e: &dyn std::fmt::Display| format!("{target}: {e}");
    // The timeout covers only transport setup; the handshake may wait on the user (host key prompt).
    let timeout = Duration::from_secs(host.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT));
    let timed_out = || format!("{target} への接続がタイムアウトしました");
    let handshake = match via {
        Some(jump) => {
            let opening = jump.channel_open_direct_tcpip(host.hostname.as_str(), host.port as u32, "127.0.0.1", 0);
            let channel = tokio::time::timeout(timeout, opening)
                .await
                .map_err(|_| timed_out())?
                .map_err(|e| fail(&e))?;
            client::connect_stream(config, channel.into_stream(), client).await
        }
        None => {
            let stream = tokio::time::timeout(timeout, TcpStream::connect((host.hostname.as_str(), host.port)))
                .await
                .map_err(|_| timed_out())?
                .map_err(|e| fail(&e))?;
            let _ = stream.set_nodelay(true);
            client::connect_stream(config, stream, client).await
        }
    };
    let mut handle = handshake.map_err(|e| rejection.lock().unwrap().take().unwrap_or_else(|| fail(&e)))?;
    auth::authenticate(&mut handle, shared, host).await?;
    Ok(handle)
}

async fn connect(shared: &Arc<Shared>, target: &HostConfig, jumps: &[HostConfig]) -> Result<Connection, String> {
    let mut chain: Vec<Handle<Client>> = Vec::with_capacity(jumps.len());
    for hop in jumps.iter().chain(std::iter::once(target)) {
        let handle = open(shared, hop, chain.last()).await?;
        chain.push(handle);
    }
    let handle = chain.pop().expect("target handle");
    Ok(Connection { handle, _jumps: chain })
}

async fn run_shell(
    shared: &Shared,
    term: &TermHandle,
    conn: &Connection,
    mut commands: mpsc::UnboundedReceiver<Command>,
) -> Result<(), russh::Error> {
    let size = shared.size();
    let channel = conn.handle.channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", size.num_cols as u32, size.num_lines as u32, 0, 0, &[])
        .await?;
    channel.request_shell(false).await?;
    let (mut reader, writer) = channel.split();
    shared.set_status(Status::Running);

    let mut parser: Processor = Processor::new();
    loop {
        let sync_deadline = parser.sync_timeout().sync_timeout();
        tokio::select! {
            msg = reader.wait() => match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    parser.advance(&mut *term.lock(), &data);
                    shared.repaint();
                }
                Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
            cmd = commands.recv() => match cmd {
                Some(Command::Input(bytes)) => writer.data_bytes(bytes).await?,
                Some(Command::Resize(ws)) => {
                    writer.window_change(ws.num_cols as u32, ws.num_lines as u32, 0, 0).await?
                }
                Some(Command::Shutdown) | None => break,
            },
            _ = sleep_until(sync_deadline), if sync_deadline.is_some() => {
                parser.stop_sync(&mut *term.lock());
                shared.repaint();
            }
        }
    }
    let _ = conn.handle.disconnect(Disconnect::ByApplication, "", "en").await;
    Ok(())
}

async fn sleep_until(deadline: Option<std::time::Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d.into()).await;
    }
}

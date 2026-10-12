//! Password-only check that a host accepts an account. No shell, and known_hosts is left unchanged.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::Disconnect;
use russh::client::{self, Handler};
use russh::keys::{self, HashAlg, PublicKeyOrCertificate};
use tokio::net::TcpStream;

const TIMEOUT: Duration = Duration::from_secs(8);

struct ProbeHandler {
    host: String,
    port: u16,
    rejection: Arc<Mutex<Option<String>>>,
}

impl ProbeHandler {
    fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_owned(),
            port,
            rejection: Arc::default(),
        }
    }
}

impl Handler for ProbeHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_key.public_key();
        match keys::check_known_hosts(&self.host, self.port, &key) {
            Ok(true) => Ok(true),
            Err(keys::Error::KeyChanged { .. }) => {
                let fingerprint = key.fingerprint(HashAlg::Sha256);
                *self.rejection.lock().unwrap() = Some(format!(
                    "ホスト鍵が known_hosts と一致しません ({fingerprint})"
                ));
                Ok(false)
            }
            _ => Ok(true),
        }
    }
}

/// Tries password authentication and disconnects. An unknown host key is accepted for this
/// attempt only. A key that differs from known_hosts is rejected.
pub async fn probe_password(
    addr: Ipv4Addr,
    port: u16,
    user: &str,
    password: &str,
) -> Result<(), String> {
    let host = addr.to_string();
    let handler = ProbeHandler::new(&host, port);
    let rejection = handler.rejection.clone();
    let config = Arc::new(client::Config::default());
    let stream = tokio::time::timeout(TIMEOUT, TcpStream::connect((addr, port)))
        .await
        .map_err(|_| "時間内に応答しません".to_owned())?
        .map_err(|error| error.to_string())?;
    let _ = stream.set_nodelay(true);
    let handshake = tokio::time::timeout(TIMEOUT, client::connect_stream(config, stream, handler))
        .await
        .map_err(|_| "時間内に応答しません".to_owned())?;
    let mut handle = handshake.map_err(|error| {
        rejection
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| error.to_string())
    })?;
    let authed = tokio::time::timeout(TIMEOUT, handle.authenticate_password(user, password))
        .await
        .map_err(|_| "時間内に応答しません".to_owned())?
        .map_err(|error| error.to_string())?;
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        handle.disconnect(Disconnect::ByApplication, "", "en"),
    )
    .await;
    if authed.success() {
        Ok(())
    } else {
        Err("ユーザーまたはパスワードが違います".to_owned())
    }
}

//! russh client handler: server host key verification against `known_hosts`.

use std::sync::{Arc, Mutex};

use russh::client;
use russh::keys::{self, HashAlg, PublicKeyOrCertificate};

use crate::terminal::{PromptKind, Shared};

pub struct Client {
    shared: Arc<Shared>,
    host: String,
    port: u16,
    /// Why the host key was rejected, reported instead of the generic connect error.
    pub rejection: Arc<Mutex<Option<String>>>,
}

impl Client {
    pub fn new(shared: Arc<Shared>, host: &str, port: u16) -> Self {
        Self {
            shared,
            host: host.to_owned(),
            port,
            rejection: Arc::default(),
        }
    }

    fn reject(&self, reason: String) -> bool {
        *self.rejection.lock().unwrap() = Some(reason);
        false
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_key.public_key();
        let fingerprint = key.fingerprint(HashAlg::Sha256);
        match keys::check_known_hosts(&self.host, self.port, &key) {
            Ok(true) => Ok(true),
            Err(keys::Error::KeyChanged { line }) => Ok(self.reject(format!(
                "{} のホスト鍵が known_hosts ({} 行目) と一致しません。中間者攻撃の可能性があります。\n受信した鍵: {fingerprint}",
                self.host, line
            ))),
            _ => {
                let message = format!(
                    "{}:{} は未登録のホストです。\n{} 鍵のフィンガープリント:\n{fingerprint}\n\n信頼して known_hosts に登録しますか?",
                    self.host,
                    self.port,
                    key.algorithm()
                );
                let accepted =
                    self.shared.ask("ホスト鍵の確認".to_owned(), message, PromptKind::Confirm).await.is_some();
                if !accepted {
                    return Ok(self.reject("ホスト鍵が承認されませんでした".to_owned()));
                }
                if let Err(e) = keys::known_hosts::learn_known_hosts(&self.host, self.port, &key) {
                    eprintln!("known_hosts への登録に失敗: {e}");
                }
                Ok(true)
            }
        }
    }
}

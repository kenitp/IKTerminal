//! User authentication: agent, identity files, keyboard-interactive, password.

use std::path::PathBuf;
use std::sync::Arc;

use russh::client::{Handle, KeyboardInteractiveAuthResponse};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg};

use super::handler::Client;
use crate::sshconfig::{HostConfig, ssh_dir};
use crate::terminal::{PromptKind, Shared};

const PASSWORD_ATTEMPTS: usize = 3;
const DEFAULT_KEYS: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

pub async fn authenticate(
    handle: &mut Handle<Client>,
    shared: &Shared,
    host: &HostConfig,
) -> Result<(), String> {
    let user = host.user.as_str();
    let rsa_hash = handle
        .best_supported_rsa_hash()
        .await
        .ok()
        .flatten()
        .flatten();

    if !host.identities_only && try_agent(handle, user, rsa_hash).await {
        return Ok(());
    }
    if try_identity_files(handle, shared, host, rsa_hash).await? {
        return Ok(());
    }
    if try_keyboard_interactive(handle, shared, host).await? {
        return Ok(());
    }
    try_password(handle, shared, host).await
}

async fn try_agent(handle: &mut Handle<Client>, user: &str, rsa_hash: Option<HashAlg>) -> bool {
    #[cfg(windows)]
    let agent = match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(a) => Some(a.dynamic()),
        Err(_) => AgentClient::connect_pageant()
            .await
            .ok()
            .map(|a| a.dynamic()),
    };
    #[cfg(not(windows))]
    let agent = AgentClient::connect_env().await.ok().map(|a| a.dynamic());

    let Some(mut agent) = agent else { return false };
    let Ok(identities) = agent.request_identities().await else {
        return false;
    };
    for identity in identities {
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        let hash = if key.algorithm().is_rsa() {
            rsa_hash
        } else {
            None
        };
        if let Ok(res) = handle
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
            && res.success()
        {
            return true;
        }
    }
    false
}

fn identity_candidates(host: &HostConfig) -> Vec<PathBuf> {
    if !host.identity_files.is_empty() {
        return host.identity_files.clone();
    }
    ssh_dir()
        .map(|d| DEFAULT_KEYS.iter().map(|k| d.join(k)).collect())
        .unwrap_or_default()
}

async fn try_identity_files(
    handle: &mut Handle<Client>,
    shared: &Shared,
    host: &HostConfig,
    rsa_hash: Option<HashAlg>,
) -> Result<bool, String> {
    for path in identity_candidates(host)
        .into_iter()
        .filter(|p| p.is_file())
    {
        let Some(key) = load_key(shared, &path).await else {
            continue;
        };
        let hash = if key.algorithm().is_rsa() {
            rsa_hash
        } else {
            None
        };
        let res = handle
            .authenticate_publickey(
                host.user.as_str(),
                PrivateKeyWithHashAlg::new(Arc::new(key), hash),
            )
            .await
            .map_err(|e| e.to_string())?;
        if res.success() {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn load_key(shared: &Shared, path: &std::path::Path) -> Option<keys::PrivateKey> {
    match keys::load_secret_key(path, None) {
        Ok(k) => return Some(k),
        Err(keys::Error::KeyIsEncrypted) => {}
        Err(_) => return None,
    }
    for _ in 0..PASSWORD_ATTEMPTS {
        let message = format!("{} のパスフレーズ", path.display());
        let pass = shared
            .ask("鍵のパスフレーズ".to_owned(), message, PromptKind::Secret)
            .await?;
        if let Ok(k) = keys::load_secret_key(path, Some(&pass)) {
            return Some(k);
        }
    }
    None
}

async fn try_keyboard_interactive(
    handle: &mut Handle<Client>,
    shared: &Shared,
    host: &HostConfig,
) -> Result<bool, String> {
    let mut reply = handle
        .authenticate_keyboard_interactive_start(host.user.as_str(), None)
        .await
        .map_err(|e| e.to_string())?;
    // Bounded so a misbehaving server cannot keep us prompting forever.
    for _ in 0..PASSWORD_ATTEMPTS * 2 {
        match reply {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                let mut responses = Vec::with_capacity(prompts.len());
                for p in prompts {
                    let title = if name.is_empty() {
                        format!("{}@{}", host.user, host.hostname)
                    } else {
                        name.clone()
                    };
                    let message = [instructions.trim(), p.prompt.trim()]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    let kind = if p.echo {
                        PromptKind::Text
                    } else {
                        PromptKind::Secret
                    };
                    let answer = shared
                        .ask(title, message, kind)
                        .await
                        .ok_or("認証がキャンセルされました")?;
                    responses.push(answer);
                }
                reply = handle
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(false)
}

async fn try_password(
    handle: &mut Handle<Client>,
    shared: &Shared,
    host: &HostConfig,
) -> Result<(), String> {
    let title = format!("{}@{}", host.user, host.hostname);
    for attempt in 0..PASSWORD_ATTEMPTS {
        let message = if attempt == 0 {
            "パスワード".to_owned()
        } else {
            "パスワードが違います。再入力してください".to_owned()
        };
        let password = shared
            .ask(title.clone(), message, PromptKind::Secret)
            .await
            .ok_or("認証がキャンセルされました")?;
        let res = handle
            .authenticate_password(host.user.as_str(), password)
            .await
            .map_err(|e| e.to_string())?;
        if res.success() {
            return Ok(());
        }
    }
    Err("認証に失敗しました".to_owned())
}

use aes::cipher::{BlockEncrypt, KeyInit, KeyIvInit, StreamCipher};
use aes::Aes128;
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::console::{p_print, Color};
use crate::crypto::{a32_to_bytes, bytes_to_a32, mega_b64_decode, mega_b64_encode};
use crate::hashcash;
use crate::util::random_alnum;

use super::crypto::{
    decrypt_attr, decrypt_csid, decrypt_key, encrypt_attr, encrypt_key, get_chunks, parse_private_key,
    pbkdf2_password, prepare_key, stringhash,
};
use super::error::MegaApiError;

pub const API_BASE: &str = "https://g.api.mega.co.nz/cs";
pub const MEGA_BASE: &str = "https://mega.nz";

#[derive(Debug, Clone)]
pub struct MegaNode {
    pub handle: String,
    pub parent: String,
    pub node_type: u8,
    pub size: u64,
    pub name: String,
    pub key: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct UploadResult {
    pub handle: String,
    pub key_b64: String,
    pub public_url: Option<String>,
}

pub struct MegaClient {
    http: reqwest::Client,
    seq: AtomicU64,
    sid: Option<String>,
    master_key: Vec<u32>,
    user_id: String,
    root_id: String,
    trash_id: String,
    request_id: String,
    email: String,
}

impl MegaClient {
    pub fn new(proxy: Option<&str>) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .user_agent("MegaTemp")
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(20));
        if let Some(proxy) = proxy {
            if !proxy.is_empty() {
                builder = builder.proxy(reqwest::Proxy::all(proxy)?);
            }
        }
        Ok(Self {
            http: builder.build()?,
            seq: AtomicU64::new(0),
            sid: None,
            master_key: Vec::new(),
            user_id: String::new(),
            root_id: String::new(),
            trash_id: String::new(),
            request_id: random_alnum(10),
            email: String::new(),
        })
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    pub fn root_id(&self) -> &str {
        &self.root_id
    }

    pub fn trash_id(&self) -> &str {
        &self.trash_id
    }

    /// Low-level MEGA API call with `-3` retry and 402 hashcash handling.
    pub async fn api_request(&self, data: Value) -> Result<Value> {
        let mut attempt = 0u32;
        let mut hashcash_attempted = false;
        let mut hashcash_solution: Option<String> = None;
        loop {
            attempt += 1;
            let id = self.seq.fetch_add(1, Ordering::Relaxed);
            let mut url = format!("{API_BASE}?id={id}");
            if let Some(sid) = &self.sid {
                url.push_str(&format!("&sid={sid}"));
            }
            let mut request = self.http.post(&url).json(&[data.clone()]);
            if hashcash_attempted {
                if let Some(solution) = &hashcash_solution {
                    request = request.header("X-Hashcash", solution);
                }
            }

            let response = match request.send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    return Err(anyhow!("MEGA request failed: {e}"));
                }
            };

            if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
                if hashcash_attempted {
                    bail!("MEGA rejected the hashcash solution");
                }
                let header = response
                    .headers()
                    .get("X-Hashcash")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let Some(header) = header else {
                    bail!("MEGA requested hashcash but sent no challenge");
                };
                let solved = tokio::task::spawn_blocking(move || {
                    hashcash::solve_challenge(&header, Duration::from_secs(60))
                })
                .await
                .map_err(|e| anyhow!("hashcash worker failed: {e}"))?;
                let Some(solution) = solved else {
                    bail!("could not solve MEGA hashcash challenge");
                };
                hashcash_solution = Some(solution);
                hashcash_attempted = true;
                continue;
            }

            let text = response.text().await?;
            let value: Value = serde_json::from_str(&text)
                .with_context(|| format!("invalid MEGA response: {}", text.chars().take(200).collect::<String>()))?;

            let first = match &value {
                Value::Array(items) => items.first().cloned(),
                other => Some(other.clone()),
            };
            let Some(first) = first else {
                bail!("empty MEGA response");
            };

            if let Some(code) = first.as_i64() {
                if code == 0 {
                    return Ok(Value::from(0));
                }
                if code == -3 && attempt < 6 {
                    tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                    continue;
                }
                return Err(MegaApiError::new(code).into());
            }
            return Ok(first);
        }
    }

    pub async fn login(&mut self, email: &str, password: &str) -> Result<()> {
        let email = email.to_lowercase();
        self.email = email.clone();

        let salt_resp = self
            .api_request(json!({"a": "us0", "user": email}))
            .await
            .context("requesting user salt")?;

        let (password_aes, user_hash) = if let Some(salt) = salt_resp.get("s").and_then(|v| v.as_str()) {
            let salt_bytes = a32_to_bytes(&bytes_to_a32(&mega_b64_decode(salt)?));
            let (aes, hash) = pbkdf2_password(password, &salt_bytes);
            (bytes_to_a32(&aes), mega_b64_encode(&hash))
        } else {
            let key = prepare_key(&bytes_to_a32(password.as_bytes()))?;
            let hash = stringhash(&email, &key)?;
            (key, hash)
        };

        let login = self
            .api_request(json!({"a": "us", "user": email, "uh": user_hash}))
            .await
            .context("logging in")?;

        let encrypted_key = login
            .get("k")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("login response has no key"))?;
        self.master_key = decrypt_key(&bytes_to_a32(&mega_b64_decode(encrypted_key)?), &password_aes)?;
        self.user_id = login.get("u").and_then(|v| v.as_str()).unwrap_or("").to_string();

        if let Some(tsid) = login.get("tsid").and_then(|v| v.as_str()) {
            let raw = mega_b64_decode(tsid)?;
            if raw.len() >= 32 {
                let first = bytes_to_a32(&raw[..16]);
                let encrypted = encrypt_key(&first, &self.master_key)?;
                if a32_to_bytes(&encrypted) == raw[raw.len() - 16..] {
                    self.sid = Some(tsid.to_string());
                }
            }
        } else if let Some(csid) = login.get("csid").and_then(|v| v.as_str()) {
            let privk = login
                .get("privk")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("login response has no private key"))?;
            let decrypted = decrypt_key(
                &bytes_to_a32(&mega_b64_decode(privk)?),
                &self.master_key,
            )?;
            let parts = parse_private_key(&a32_to_bytes(&decrypted))?;
            self.sid = Some(decrypt_csid(csid, &parts)?);
        } else {
            bail!("login response has neither tsid nor csid");
        }

        let nodes = self.get_files().await?;
        self.root_id = nodes
            .iter()
            .find(|n| n.node_type == 2)
            .map(|n| n.handle.clone())
            .unwrap_or_default();
        self.trash_id = nodes
            .iter()
            .find(|n| n.node_type == 4)
            .map(|n| n.handle.clone())
            .unwrap_or_default();
        Ok(())
    }

    pub async fn get_files(&self) -> Result<Vec<MegaNode>> {
        let resp = self.api_request(json!({"a": "f", "c": 1, "r": 1})).await?;
        let Some(files) = resp.get("f").and_then(|v| v.as_array()) else {
            return Ok(Vec::new());
        };
        let mut nodes = Vec::new();
        for file in files {
            let handle = file.get("h").and_then(|v| v.as_str()).unwrap_or("");
            let node_type = file.get("t").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
            if handle.is_empty() || !matches!(node_type, 0 | 1) {
                continue;
            }
            let parent = file.get("p").and_then(|v| v.as_str()).unwrap_or("");
            let size = file.get("s").and_then(|v| v.as_u64()).unwrap_or(0);
            let keys = file.get("k").and_then(|v| v.as_str()).unwrap_or("");
            let mut key_map: Vec<(String, String)> = Vec::new();
            for part in keys.split('/') {
                if let Some((uid, k)) = part.split_once(':') {
                    key_map.push((uid.to_string(), k.to_string()));
                }
            }
            let uid = file.get("u").and_then(|v| v.as_str()).unwrap_or("");
            let chosen = key_map
                .iter()
                .find(|(k, _)| k == uid || k == &self.user_id)
                .or_else(|| key_map.first());
            let Some((_, key_b64)) = chosen else {
                continue;
            };
            let Ok(encrypted_key) = mega_b64_decode(key_b64) else {
                continue;
            };
            let key = match decrypt_key(&bytes_to_a32(&encrypted_key), &self.master_key) {
                Ok(k) => k,
                Err(_) => continue,
            };
            if key.len() < 4 {
                continue;
            }
            let attrs = file.get("a").and_then(|v| v.as_str()).unwrap_or("");
            let name = match decrypt_attr(attrs, &key[..4]) {
                Ok(v) => v.get("n").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                Err(_) => continue,
            };
            nodes.push(MegaNode {
                handle: handle.to_string(),
                parent: parent.to_string(),
                node_type,
                size,
                name,
                key,
            });
        }
        Ok(nodes)
    }

    pub async fn list_root_files(&self) -> Result<Vec<MegaNode>> {
        let mut files: Vec<MegaNode> = self
            .get_files()
            .await?
            .into_iter()
            .filter(|n| n.node_type == 0 && n.parent == self.root_id)
            .collect();
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(files)
    }

    pub async fn quota(&self) -> Result<(u64, u64)> {
        let resp = self
            .api_request(json!({"a": "uq", "xfer": 1, "strg": 1, "v": 1}))
            .await?;
        let used = resp.get("cstrg").and_then(|v| v.as_u64()).unwrap_or(0);
        let total = resp.get("mstrg").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok((used, total))
    }

    pub async fn public_link(&self, handle: &str, key_b64: &str) -> Result<String> {
        let public = self.api_request(json!({"a": "l", "n": handle})).await?;
        let public = public
            .as_str()
            .ok_or_else(|| anyhow!("could not create public link"))?;
        Ok(format!("{MEGA_BASE}/#!{public}!{key_b64}"))
    }

    /// Upload a local file and, when `public` is set, return its public link.
    pub async fn upload(
        &self,
        path: &Path,
        dest: Option<&str>,
        dest_name: Option<&str>,
        public: bool,
    ) -> Result<UploadResult> {
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() {
            bail!("{} is not a file", path.display());
        }
        let size = metadata.len();
        let dest = dest.unwrap_or(&self.root_id);
        if dest.is_empty() {
            bail!("no destination folder");
        }
        let upload_resp = self.api_request(json!({"a": "u", "s": size})).await?;
        let upload_url = upload_resp
            .get("p")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("MEGA did not return an upload URL"))?;

        let ul_key: Vec<u32> = (0..6).map(|_| rand::random::<u32>()).collect();
        let key_bytes = a32_to_bytes(&ul_key[..4]);
        let counter = a32_to_bytes(&[ul_key[4], ul_key[5], 0, 0]);
        let mut cipher = ctr::Ctr128BE::<Aes128>::new_from_slices(&key_bytes, &counter)?;
        let block_cipher = Aes128::new_from_slice(&key_bytes)?;
        let iv = a32_to_bytes(&[ul_key[4], ul_key[5], ul_key[4], ul_key[5]]);
        let mut chain = [0u8; 16];

        let mut file = tokio::fs::File::open(path).await?;
        let chunks = get_chunks(size);
        let mut completion_handle = String::new();

        if size == 0 {
            let resp = self
                .http
                .post(format!("{}/0", upload_url.trim_end_matches('/')))
                .body(Vec::new())
                .send()
                .await?
                .text()
                .await?;
            completion_handle = clean_handle(&resp);
        }

        for (offset, chunk_size) in chunks {
            if chunk_size == 0 && size > 0 {
                continue;
            }
            let mut chunk = vec![0u8; chunk_size];
            tokio::io::AsyncReadExt::read_exact(&mut file, &mut chunk).await?;

            // CBC-MAC over the final block of this chunk (MEGA's scheme).
            let start = if chunk.len() > 16 {
                ((chunk.len() - 1) / 16) * 16
            } else {
                0
            };
            let mut block = [0u8; 16];
            let available = chunk.len().saturating_sub(start).min(16);
            block[..available].copy_from_slice(&chunk[start..start + available]);
            for i in 0..16 {
                block[i] ^= iv[i];
            }
            block_cipher.encrypt_block((&mut block).into());
            for i in 0..16 {
                block[i] ^= chain[i];
            }
            block_cipher.encrypt_block((&mut block).into());
            chain = block;

            cipher.apply_keystream(&mut chunk);
            let resp = self
                .http
                .post(format!("{}/{}", upload_url.trim_end_matches('/'), offset))
                .body(chunk)
                .send()
                .await?;
            if !resp.status().is_success() {
                bail!("upload chunk failed: HTTP {}", resp.status());
            }
            completion_handle = clean_handle(&resp.text().await?);
        }

        if completion_handle.is_empty() {
            bail!("MEGA did not return a completion handle");
        }

        let mac = [
            u32::from_be_bytes([chain[0], chain[1], chain[2], chain[3]]),
            u32::from_be_bytes([chain[4], chain[5], chain[6], chain[7]]),
            u32::from_be_bytes([chain[8], chain[9], chain[10], chain[11]]),
            u32::from_be_bytes([chain[12], chain[13], chain[14], chain[15]]),
        ];
        let meta_mac = [mac[0] ^ mac[1], mac[2] ^ mac[3]];
        let key = vec![
            ul_key[0] ^ ul_key[4],
            ul_key[1] ^ ul_key[5],
            ul_key[2] ^ meta_mac[0],
            ul_key[3] ^ meta_mac[1],
            ul_key[4],
            ul_key[5],
            meta_mac[0],
            meta_mac[1],
        ];

        let name = dest_name
            .map(str::to_string)
            .or_else(|| path.file_name().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| "upload.bin".to_string());
        let attrs = encrypt_attr(&json!({"n": name}), &ul_key[..4])?;
        let encrypted_key = mega_b64_encode(&a32_to_bytes(&encrypt_key(&key, &self.master_key)?));

        self.api_request(json!({
            "a": "p",
            "t": dest,
            "i": self.request_id,
            "n": [{
                "h": completion_handle,
                "t": 0,
                "a": attrs,
                "k": encrypted_key
            }]
        }))
        .await?;

        let key_b64 = mega_b64_encode(&a32_to_bytes(&key));
        let public_url = if public {
            Some(self.public_link(&completion_handle, &key_b64).await?)
        } else {
            None
        };

        Ok(UploadResult {
            handle: completion_handle,
            key_b64,
            public_url,
        })
    }

    /// Download and decrypt a node to `dest`.
    pub async fn download(&self, node: &MegaNode, dest: &Path) -> Result<()> {
        if node.key.len() < 6 {
            bail!("node key is incomplete");
        }
        let resp = self
            .api_request(json!({"a": "g", "g": 1, "n": node.handle}))
            .await?;
        let url = resp
            .get("g")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("MEGA did not return a download URL"))?;

        let key_bytes = a32_to_bytes(&node.key[..4]);
        let counter = a32_to_bytes(&[node.key[4], node.key[5], 0, 0]);
        let mut cipher = ctr::Ctr128BE::<Aes128>::new_from_slices(&key_bytes, &counter)?;

        let part = dest.with_extension(format!(
            "{}part",
            dest.extension()
                .and_then(|e| e.to_str())
                .map(|e| format!("{e}."))
                .unwrap_or_default()
        ));
        let mut file = tokio::fs::File::create(&part).await?;
        let response = self.http.get(url).send().await?.error_for_status()?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let mut chunk = chunk?.to_vec();
            cipher.apply_keystream(&mut chunk);
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        drop(file);
        tokio::fs::rename(&part, dest).await?;
        Ok(())
    }

    pub async fn delete_node(&self, handle: &str) -> Result<()> {
        self.api_request(json!({"a": "d", "n": handle, "i": self.request_id}))
            .await?;
        Ok(())
    }

    pub async fn empty_trash(&self) -> Result<()> {
        let nodes = self.get_files().await?;
        let trash = self.trash_id.clone();
        let mut requests = Vec::new();
        for node in nodes.into_iter().filter(|n| n.parent == trash) {
            requests.push(json!({"a": "d", "n": node.handle, "i": self.request_id}));
        }
        if !requests.is_empty() {
            self.api_request(Value::Array(requests)).await?;
        }
        Ok(())
    }

    pub async fn delete_account(&self) -> Result<()> {
        let nodes = self.get_files().await?;
        let mut requests = Vec::new();
        for node in &nodes {
            if node.node_type == 0 || node.node_type == 1 {
                requests.push(json!({"a": "d", "n": node.handle, "i": self.request_id}));
            }
        }
        if !requests.is_empty() {
            self.api_request(Value::Array(requests)).await?;
        }
        self.empty_trash().await?;
        self.api_request(json!({"a": "uc"})).await?;
        Ok(())
    }

    pub async fn create_folder(&self, name: &str, dest: Option<&str>) -> Result<MegaNode> {
        let parent = dest.unwrap_or(&self.root_id).to_string();
        let parts: Vec<&str> = name.split('/').filter(|p| !p.trim().is_empty()).collect();
        if parts.is_empty() {
            bail!("folder name is empty");
        }
        let mut current = parent.clone();
        for part in parts {
            let existing = self
                .get_files()
                .await?
                .into_iter()
                .find(|n| n.node_type == 1 && n.parent == current && n.name == part);
            if let Some(node) = existing {
                current = node.handle;
                continue;
            }
            let ul_key: Vec<u32> = (0..6).map(|_| rand::random::<u32>()).collect();
            let attrs = encrypt_attr(&json!({"n": part}), &ul_key[..4])?;
            let encrypted_key = mega_b64_encode(&a32_to_bytes(&encrypt_key(
                &ul_key[..4],
                &self.master_key,
            )?));
            let resp = self
                .api_request(json!({
                    "a": "p",
                    "t": current,
                    "n": [{"h": "xxxxxxxx", "t": 1, "a": attrs, "k": encrypted_key}],
                    "i": self.request_id
                }))
                .await?;
            let handle = resp
                .get("f")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.get("h"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_default();
            current = handle;
        }
        Ok(MegaNode {
            handle: current,
            parent,
            node_type: 1,
            size: 0,
            name: name.to_string(),
            key: Vec::new(),
        })
    }

    /// Remove MEGA's default "welcome" root file.
    pub async fn delete_welcome_file(&self) -> Result<usize> {
        let files = self.list_root_files().await?;
        let mut removed = 0;
        for file in files {
            let name = file.name.to_lowercase();
            if name.contains("welcome") || name.contains("getting-started") {
                if self.delete_node(&file.handle).await.is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}

fn clean_handle(text: &str) -> String {
    text.trim().trim_matches('"').trim().to_string()
}

pub fn auth_failure(err: &anyhow::Error) -> bool {
    err.downcast_ref::<MegaApiError>()
        .map(MegaApiError::is_auth_failure)
        .unwrap_or(false)
}

pub fn transient(err: &anyhow::Error) -> bool {
    err.downcast_ref::<MegaApiError>()
        .map(MegaApiError::is_transient)
        .unwrap_or(false)
}

pub fn log_error(err: &anyhow::Error) {
    p_print(&format!("MEGA error: {err}"), Color::Fail);
}

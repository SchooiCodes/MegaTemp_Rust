use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const ENC2_PREFIX: &str = "ENC2:";
pub const ENC_PREFIX: &str = "ENC:";
pub const OBS_PREFIX: &str = "OBS:";

/// MEGA uses URL-safe base64 without padding.
pub fn mega_b64_encode(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

pub fn mega_b64_decode(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| *c != '=' && *c != ',').collect();
    URL_SAFE_NO_PAD
        .decode(cleaned.as_bytes())
        .or_else(|_| URL_SAFE.decode(cleaned.as_bytes()))
        .or_else(|_| STANDARD.decode(cleaned.as_bytes()))
        .map_err(|e| anyhow!("invalid base64: {e}"))
}

pub fn bytes_to_a32(data: &[u8]) -> Vec<u32> {
    let mut padded = data.to_vec();
    while padded.len() % 4 != 0 {
        padded.push(0);
    }
    padded
        .chunks_exact(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn a32_to_bytes(a: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.len() * 4);
    for v in a {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out
}

fn key_from_password(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hasher.finalize().into()
}

pub fn is_encrypted_value(value: &str) -> bool {
    value.starts_with(ENC2_PREFIX) || value.starts_with(ENC_PREFIX) || value.starts_with(OBS_PREFIX)
}

/// Modern `ENC2:` field encryption: AES-256-GCM.
pub fn encrypt_field(value: &str, password: &str) -> Result<String> {
    if value.is_empty() {
        return Ok(value.to_string());
    }
    let key = key_from_password(password);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("cipher init: {e}"))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let sealed = cipher
        .encrypt(nonce, value.as_bytes())
        .map_err(|e| anyhow!("encrypt: {e}"))?;
    if sealed.len() < 16 {
        bail!("unexpected ciphertext length");
    }
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    let mut out = Vec::with_capacity(12 + 16 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(tag);
    out.extend_from_slice(ct);
    Ok(format!("{ENC2_PREFIX}{}", STANDARD.encode(out)))
}

pub fn decrypt_field(value: &str, password: &str) -> Result<String> {
    if let Some(rest) = value.strip_prefix(ENC2_PREFIX) {
        let raw = STANDARD
            .decode(rest)
            .or_else(|_| URL_SAFE.decode(rest))
            .map_err(|e| anyhow!("invalid ENC2 payload: {e}"))?;
        if raw.len() < 12 + 16 {
            bail!("ENC2 payload too short");
        }
        let key = key_from_password(password);
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("cipher init: {e}"))?;
        let nonce = Nonce::from_slice(&raw[..12]);
        let (tag, ct) = raw[12..].split_at(16);
        let mut sealed = Vec::with_capacity(ct.len() + 16);
        sealed.extend_from_slice(ct);
        sealed.extend_from_slice(tag);
        let plain = cipher
            .decrypt(nonce, sealed.as_ref())
            .map_err(|_| anyhow!("ENC2 decryption failed"))?;
        return String::from_utf8(plain).map_err(|e| anyhow!("invalid utf-8: {e}"));
    }

    if let Some(rest) = value.strip_prefix(ENC_PREFIX) {
        return decrypt_fernet(rest, password);
    }

    if let Some(rest) = value.strip_prefix(OBS_PREFIX) {
        return decrypt_obfuscated(rest, password);
    }

    Ok(value.to_string())
}

/// Legacy Fernet (AES-128-CBC + HMAC-SHA256) format used by the Python client.
fn decrypt_fernet(token: &str, password: &str) -> Result<String> {
    let cleaned: String = token.trim().trim_end_matches('=').to_string();
    let raw = URL_SAFE_NO_PAD
        .decode(cleaned.as_bytes())
        .or_else(|_| URL_SAFE.decode(cleaned.as_bytes()))
        .map_err(|e| anyhow!("invalid legacy token: {e}"))?;
    if raw.len() < 1 + 8 + 16 + 16 + 32 {
        bail!("legacy token too short");
    }
    if raw[0] != 0x80 {
        bail!("unsupported legacy version");
    }
    let hash = key_from_password(password);
    let (mac_key, enc_key) = hash.split_at(16);
    let (body, sig) = raw.split_at(raw.len() - 32);

    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(mac_key).unwrap();
    mac.update(body);
    mac.verify_slice(sig)
        .map_err(|_| anyhow!("legacy token signature mismatch"))?;

    let iv = &body[9..25];
    let ciphertext = &body[25..];
    let mut buf = ciphertext.to_vec();
    let decryptor = cbc::Decryptor::<aes::Aes128>::new(enc_key.into(), iv.into());
    let plain = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| anyhow!("legacy decryption failed"))?;
    String::from_utf8(plain.to_vec()).map_err(|e| anyhow!("invalid utf-8: {e}"))
}

/// Legacy `OBS:` XOR obfuscation.
fn decrypt_obfuscated(value: &str, password: &str) -> Result<String> {
    let raw = STANDARD
        .decode(value)
        .or_else(|_| URL_SAFE.decode(value))
        .map_err(|e| anyhow!("invalid OBS payload: {e}"))?;
    let key = key_from_password(password);
    let mut data = raw;
    if data.len() < 64 {
        data.resize(64, 0);
    }
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= key[i % key.len()];
    }
    while data.last() == Some(&0) {
        data.pop();
    }
    String::from_utf8(data).map_err(|e| anyhow!("invalid utf-8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enc2_roundtrip() {
        let enc = encrypt_field("hunter2", "pw").unwrap();
        assert!(enc.starts_with(ENC2_PREFIX));
        assert_eq!(decrypt_field(&enc, "pw").unwrap(), "hunter2");
        assert!(decrypt_field(&enc, "wrong").is_err());
    }

    #[test]
    fn empty_value_untouched() {
        assert_eq!(encrypt_field("", "pw").unwrap(), "");
        assert_eq!(decrypt_field("plain", "pw").unwrap(), "plain");
    }

    #[test]
    fn mega_base64_roundtrip() {
        let data = vec![0u8, 1, 2, 250, 251, 252];
        let encoded = mega_b64_encode(&data);
        assert!(!encoded.contains('='));
        assert_eq!(mega_b64_decode(&encoded).unwrap(), data);
    }

    #[test]
    fn a32_roundtrip() {
        let data = b"hello world";
        assert_eq!(a32_to_bytes(&bytes_to_a32(data)), b"hello world\0".to_vec());
    }
}

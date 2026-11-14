use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use anyhow::{bail, Result};
use num_bigint::{BigInt, BigUint, Sign};
use num_integer::{ExtendedGcd, Integer};
use num_traits::One;
use sha2::{Digest, Sha256, Sha512};

use crate::crypto::{a32_to_bytes, bytes_to_a32, mega_b64_decode, mega_b64_encode};

pub fn aes_cbc_encrypt(data: &[u8], key: &[u32]) -> Result<Vec<u8>> {
    if data.len() % 16 != 0 {
        bail!("aes_cbc_encrypt length must be a multiple of 16");
    }
    let key_bytes = a32_to_bytes(key);
    let cipher = Aes128::new_from_slice(&key_bytes).map_err(|e| anyhow::anyhow!("bad key: {e}"))?;
    let zero = [0u8; 16];
    let mut out = data.to_vec();
    let mut prev = zero;
    for block in out.chunks_exact_mut(16) {
        for (b, p) in block.iter_mut().zip(prev.iter()) {
            *b ^= p;
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(block);
        cipher.encrypt_block((&mut arr).into());
        block.copy_from_slice(&arr);
        prev = arr;
    }
    Ok(out)
}

pub fn aes_cbc_decrypt(data: &[u8], key: &[u32]) -> Result<Vec<u8>> {
    if data.len() % 16 != 0 {
        bail!("aes_cbc_decrypt length must be a multiple of 16");
    }
    let key_bytes = a32_to_bytes(key);
    let cipher = Aes128::new_from_slice(&key_bytes).map_err(|e| anyhow::anyhow!("bad key: {e}"))?;
    let mut out = data.to_vec();
    let mut prev = [0u8; 16];
    for block in out.chunks_exact_mut(16) {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(block);
        let cipher_block = arr;
        cipher.decrypt_block((&mut arr).into());
        for (b, p) in arr.iter_mut().zip(prev.iter()) {
            *b ^= p;
        }
        block.copy_from_slice(&arr);
        prev = cipher_block;
    }
    Ok(out)
}

fn aes_cbc_encrypt_a32(data: &[u32], key: &[u32]) -> Result<Vec<u32>> {
    Ok(bytes_to_a32(&aes_cbc_encrypt(&a32_to_bytes(data), key)?))
}

fn aes_cbc_decrypt_a32(data: &[u32], key: &[u32]) -> Result<Vec<u32>> {
    Ok(bytes_to_a32(&aes_cbc_decrypt(&a32_to_bytes(data), key)?))
}

/// MEGA's legacy password key derivation.
pub fn prepare_key(password: &[u32]) -> Result<Vec<u32>> {
    let mut pkey = vec![0x93C4_67E3u32, 0x7DB0_C7A4, 0xD1BE_3F81, 0x0152_CB56];
    for _ in 0..0x10000 {
        for j in (0..password.len()).step_by(4) {
            let mut key = [0u32; 4];
            for i in 0..4 {
                if i + j < password.len() {
                    key[i] = password[i + j];
                }
            }
            pkey = aes_cbc_encrypt_a32(&pkey, &key)?;
        }
    }
    Ok(pkey)
}

/// MEGA's v1 email hash.
pub fn stringhash(email: &str, key: &[u32]) -> Result<String> {
    let mut s32 = bytes_to_a32(email.as_bytes());
    let mut h32 = [0u32; 4];
    for (i, v) in s32.iter().enumerate() {
        h32[i % 4] ^= *v;
    }
    for _ in 0..0x4000 {
        let out = aes_cbc_encrypt_a32(&h32, key)?;
        h32.copy_from_slice(&out[..4]);
    }
    s32 = vec![h32[0], h32[2]];
    Ok(mega_b64_encode(&a32_to_bytes(&s32)))
}

pub fn decrypt_key(data: &[u32], key: &[u32]) -> Result<Vec<u32>> {
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(4) {
        out.extend(aes_cbc_decrypt_a32(chunk, key)?);
    }
    Ok(out)
}

pub fn encrypt_key(data: &[u32], key: &[u32]) -> Result<Vec<u32>> {
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(4) {
        out.extend(aes_cbc_encrypt_a32(chunk, key)?);
    }
    Ok(out)
}

pub fn encrypt_attr(attr: &serde_json::Value, key: &[u32]) -> Result<String> {
    let mut raw = b"MEGA".to_vec();
    raw.extend_from_slice(python_json(attr).as_bytes());
    while raw.len() % 16 != 0 {
        raw.push(0);
    }
    Ok(mega_b64_encode(&aes_cbc_encrypt(&raw, key)?))
}

/// Match Python's `json.dumps` spacing so attribute bytes are identical.
fn python_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(k).unwrap_or_default(),
                        python_json(v)
                    )
                })
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        serde_json::Value::Array(items) => {
            let entries: Vec<String> = items.iter().map(python_json).collect();
            format!("[{}]", entries.join(", "))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub fn decrypt_attr(attr: &str, key: &[u32]) -> Result<serde_json::Value> {
    let raw = mega_b64_decode(attr)?;
    let decrypted = aes_cbc_decrypt(&raw, key)?;
    let end = decrypted.iter().position(|b| *b == 0).unwrap_or(decrypted.len());
    let text = String::from_utf8_lossy(&decrypted[..end]);
    let Some(json_part) = text.strip_prefix("MEGA") else {
        bail!("invalid MEGA attributes");
    };
    Ok(serde_json::from_str(json_part)?)
}

/// MEGA file chunk boundaries.
pub fn get_chunks(size: u64) -> Vec<(u64, usize)> {
    let mut chunks = Vec::new();
    let mut p = 0u64;
    let mut s = 0x20000u64;
    while p + s < size {
        chunks.push((p, s as usize));
        p += s;
        if s < 0x100000 {
            s += 0x20000;
        }
    }
    chunks.push((p, (size - p) as usize));
    chunks
}

pub fn pbkdf2_password(password: &str, salt: &[u8]) -> ([u8; 16], [u8; 16]) {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha512>(password.as_bytes(), salt, 100_000, &mut out);
    let mut aes = [0u8; 16];
    let mut hash = [0u8; 16];
    aes.copy_from_slice(&out[..16]);
    hash.copy_from_slice(&out[16..]);
    (aes, hash)
}

/// Parse the four MPI integers from MEGA's RSA private key blob.
pub fn parse_private_key(blob: &[u8]) -> Result<Vec<BigUint>> {
    let mut rest = blob;
    let mut parts = Vec::new();
    for _ in 0..4 {
        if rest.len() < 2 {
            bail!("truncated RSA private key");
        }
        let bits = (rest[0] as usize) * 256 + rest[1] as usize;
        let bytes = bits.div_ceil(8) + 2;
        if rest.len() < bytes {
            bail!("truncated RSA private key component");
        }
        parts.push(BigUint::from_bytes_be(&rest[2..bytes]));
        rest = &rest[bytes..];
    }
    Ok(parts)
}

pub fn rsa_private_components(parts: &[BigUint]) -> Result<(BigUint, BigUint, BigUint)> {
    if parts.len() < 3 {
        bail!("missing RSA components");
    }
    let p = parts[0].clone();
    let q = parts[1].clone();
    let d = parts[2].clone();
    let n = &p * &q;
    let phi = (&p - BigUint::one()) * (&q - BigUint::one());
    let d_int = BigInt::from_biguint(Sign::Plus, d.clone());
    let phi_int = BigInt::from_biguint(Sign::Plus, phi);
    let ExtendedGcd { gcd, x, .. } = d_int.extended_gcd(&phi_int);
    if !gcd.is_one() {
        bail!("RSA public exponent is not invertible");
    }
    let e = if x.sign() == Sign::Minus {
        (x + &phi_int).to_biguint().unwrap()
    } else {
        x.to_biguint().unwrap()
    };
    Ok((n, e, d))
}

pub fn rsa_raw_decrypt(c: &BigUint, d: &BigUint, n: &BigUint) -> Vec<u8> {
    c.modpow(d, n).to_bytes_be()
}

/// Decrypt the `csid` challenge into a session id.
pub fn decrypt_csid(csid: &str, parts: &[BigUint]) -> Result<String> {
    let (n, _e, d) = rsa_private_components(parts)?;
    let raw = mega_b64_decode(csid)?;
    let c = BigUint::from_bytes_be(&raw);
    let mut plain = rsa_raw_decrypt(&c, &d, &n);
    if plain.len() > 43 {
        plain.truncate(43);
    }
    Ok(mega_b64_encode(&plain))
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_match_mega() {
        assert_eq!(get_chunks(0), vec![(0, 0)]);
        assert_eq!(get_chunks(100), vec![(0, 100)]);
        let big = get_chunks(0x400000);
        assert_eq!(big.iter().map(|(_, s)| *s).sum::<usize>(), 0x400000);
        assert_eq!(big[0].1, 0x20000);
    }

    #[test]
    fn attr_roundtrip() {
        let key = vec![1u32, 2, 3, 4];
        let value = serde_json::json!({"n": "hello.bin"});
        let enc = encrypt_attr(&value, &key).unwrap();
        let dec = decrypt_attr(&enc, &key).unwrap();
        assert_eq!(dec["n"], "hello.bin");
    }

    #[test]
    fn key_roundtrip() {
        let key = vec![9u32, 8, 7, 6];
        let data = vec![1u32, 2, 3, 4, 5, 6, 7, 8];
        let enc = encrypt_key(&data, &key).unwrap();
        assert_eq!(decrypt_key(&enc, &key).unwrap(), data);
    }

    #[test]
    fn python_compatibility_vectors() {
        // Generated with mega.py's crypto.py.
        let key = prepare_key(&bytes_to_a32(b"test123")).unwrap();
        assert_eq!(key, vec![1760830136, 2035647396, 3097494260, 2788914512]);
        assert_eq!(stringhash("user@example.com", &key).unwrap(), "dIcC4tyJwAU");
        assert_eq!(
            encrypt_attr(&serde_json::json!({"n": "hello.bin"}), &[1, 2, 3, 4]).unwrap(),
            "TQ6Djbscnu7yNI0KdFCjCTuTu3lAQfSXXTUtcWL0zw8"
        );
        assert_eq!(
            mega_b64_encode(&a32_to_bytes(&[1, 2, 3, 4])),
            "AAAAAQAAAAIAAAADAAAABA"
        );
    }
}

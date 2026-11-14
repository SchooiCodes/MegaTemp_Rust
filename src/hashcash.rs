use crate::crypto::mega_b64_encode;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const TOKEN_BYTES: usize = 48;
pub const PREFIX_BYTES: usize = 4;
pub const REPEAT: usize = 262_144;
pub const BUF_SIZE: usize = PREFIX_BYTES + REPEAT * TOKEN_BYTES;

/// MEGA's hashcash difficulty conversion.
pub fn threshold_from_easiness(easiness: u32) -> u64 {
    let low = ((easiness & 63) << 1) as u64 + 1;
    let shift = ((easiness >> 6) * 7 + 3) as u32;
    low.checked_shl(shift).unwrap_or(u64::MAX)
}

fn make_buffer(token: &str) -> Vec<u8> {
    let bytes = token.as_bytes();
    let mut buffer = vec![0u8; BUF_SIZE];
    if bytes.is_empty() {
        return buffer;
    }
    let body = &mut buffer[PREFIX_BYTES..];
    for (i, slot) in body.iter_mut().enumerate() {
        *slot = bytes[i % bytes.len()];
    }
    buffer
}

fn matches(buffer: &mut [u8], nonce: u32, threshold: u64) -> bool {
    buffer[..4].copy_from_slice(&nonce.to_be_bytes());
    let digest = Sha256::digest(&buffer[..]);
    let value = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) as u64;
    value <= threshold
}

/// Find a hashcash solution for `token` at the given easiness.
/// Returns MEGA-base64 of the 4-byte big-endian nonce.
pub fn solve(token: &str, easiness: u32, timeout: Duration) -> Option<String> {
    let threshold = threshold_from_easiness(easiness);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8);
    let found = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + timeout;
    let mut handles = Vec::new();

    for worker in 0..workers {
        let token = token.to_string();
        let found = Arc::clone(&found);
        handles.push(std::thread::spawn(move || -> Option<u32> {
            let mut buffer = make_buffer(&token);
            let mut nonce = worker as u32;
            let step = workers as u32;
            while !found.load(Ordering::Relaxed) && Instant::now() < deadline {
                if matches(&mut buffer, nonce, threshold) {
                    found.store(true, Ordering::Relaxed);
                    return Some(nonce);
                }
                nonce = nonce.wrapping_add(step);
            }
            None
        }));
    }

    for handle in handles {
        if let Ok(Some(nonce)) = handle.join() {
            return Some(mega_b64_encode(&nonce.to_be_bytes()));
        }
    }
    None
}

/// Parse a server `X-Hashcash` challenge (`1:easiness:timestamp:token`).
pub fn parse_challenge(header: &str) -> Option<(u32, String)> {
    let parts: Vec<&str> = header.splitn(4, ':').collect();
    if parts.len() != 4 {
        return None;
    }
    Some((parts[1].trim().parse().ok()?, parts[3].to_string()))
}

pub fn build_header(token: &str, solution: &str) -> String {
    format!("1:{token}:{solution}")
}

pub fn solve_challenge(header: &str, timeout: Duration) -> Option<String> {
    let (easiness, token) = parse_challenge(header)?;
    let solution = solve(&token, easiness, timeout)?;
    Some(build_header(&token, &solution))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_192() {
        assert_eq!(threshold_from_easiness(192), 16_777_216);
    }

    #[test]
    fn parse_and_build_header() {
        let (e, token) = parse_challenge("1:192:1700000000:abc").unwrap();
        assert_eq!(e, 192);
        assert_eq!(token, "abc");
        assert_eq!(build_header(&token, "sol"), "1:abc:sol");
    }

    #[test]
    fn solves_easy_challenge() {
        let token = "a".repeat(TOKEN_BYTES);
        let solution = solve(&token, 255, Duration::from_secs(30)).expect("solution");
        let raw = crate::crypto::mega_b64_decode(&solution).unwrap();
        assert_eq!(raw.len(), 4);
    }
}

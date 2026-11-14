use std::fmt;

/// Error returned by the MEGA API (`-N` codes).
#[derive(Debug, Clone)]
pub struct MegaApiError {
    pub code: i64,
}

impl fmt::Display for MegaApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MEGA API error {}: {}", self.code, describe(self.code))
    }
}

impl std::error::Error for MegaApiError {}

impl MegaApiError {
    pub fn new(code: i64) -> Self {
        Self { code }
    }

    pub fn is_auth_failure(&self) -> bool {
        matches!(self.code, -9 | -15 | -16)
    }

    pub fn is_transient(&self) -> bool {
        matches!(self.code, -3 | -4)
    }
}

pub fn describe(code: i64) -> &'static str {
    match code {
        -1 => "internal error",
        -2 => "invalid argument",
        -3 => "request failed, retrying",
        -4 => "over quota",
        -5 => "invalid session",
        -6 => "invalid request",
        -7 => "invalid key",
        -8 => "not found",
        -9 => "object does not exist",
        -10 => "circular linkage",
        -11 => "access denied",
        -12 => "already exists",
        -13 => "incomplete request",
        -14 => "invalid crypto key",
        -15 => "session expired",
        -16 => "invalid account",
        -17 => "blocked",
        0 => "ok",
        _ => "unknown error",
    }
}

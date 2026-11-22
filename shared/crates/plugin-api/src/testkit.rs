//! Test support (feature `testkit`): lets a plugin's unit tests drive its typed functions
//! through a byte-level request/response shape, so one test can check what a message looks like
//! once serialised. Nothing in the program uses this; it is not part of any binary.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A request body, as bytes.
#[derive(Debug, Clone)]
pub struct RcSlice(Vec<u8>);

impl RcSlice {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.to_vec())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    Ok = 0,
    Error = 1,
}

#[derive(Debug, Clone, Copy)]
pub struct RcStatus {
    pub code: u32,
}

impl RcStatus {
    #[must_use]
    pub fn is_ok(self) -> bool {
        self.code == StatusCode::Ok as u32
    }
}

/// The bytes of a response.
#[derive(Debug, Clone)]
pub struct Payload(Vec<u8>);

impl Payload {
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

/// A response: a status and a payload (the bytes of the value, or the error text).
#[derive(Debug, Clone)]
pub struct RcCallResult {
    pub status: RcStatus,
    pub payload: Payload,
}

impl RcCallResult {
    /// Wraps a typed result.
    pub fn of<T: Serialize>(result: Result<T, String>) -> Self {
        match result {
            Ok(value) => Self {
                status: RcStatus { code: 0 },
                payload: Payload(encode_message(&value).expect("encode response")),
            },
            Err(error) => Self {
                status: RcStatus { code: 1 },
                payload: Payload(error.into_bytes()),
            },
        }
    }
}

/// Serialises a value.
///
/// # Errors
/// When the value cannot be encoded.
pub fn encode_message<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    bincode::serde::encode_to_vec(value, bincode::config::standard()).map_err(|e| e.to_string())
}

/// Reads a value back.
///
/// # Errors
/// When the bytes are not a `T`.
pub fn decode_message<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        .map(|(value, _)| value)
        .map_err(|e| e.to_string())
}

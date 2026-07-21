//! Blocking JSON-over-HTTP transport abstraction.
//!
//! The whole crate is generic over [`JsonHttpTransport`] so that host tests
//! mock the network and never touch it. Inside a wasm component the shim
//! passes [`WakiJsonTransport`], which delegates to the host's `wasi:http`
//! through the blocking `waki` client (TLS and DNS are performed host-side,
//! outside the sandbox).

use serde_json::Value;

use crate::error::CoreError;

/// Default connect timeout in seconds for outbound RPC calls. waki exposes a
/// connect timeout only (no read timeout); the host's own execution limits
/// bound the total call time (sourceRef: docs/book/src/plugins/
/// writing-a-tool-plugin.md, fuel and memory ceilings).
pub const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;

/// A blocking transport that POSTs a JSON body and returns the JSON response.
pub trait JsonHttpTransport {
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, CoreError>;
}

/// References delegate, so a caller can keep ownership of its transport
/// (tests inspect recorded requests through the original after the call).
impl<Transport: JsonHttpTransport> JsonHttpTransport for &Transport {
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, CoreError> {
        (*self).post_json(url, body)
    }
}

/// `wasi:http` transport for wasm components, backed by `waki`.
#[cfg(target_family = "wasm")]
pub struct WakiJsonTransport {
    pub connect_timeout_secs: u64,
}

#[cfg(target_family = "wasm")]
impl Default for WakiJsonTransport {
    fn default() -> Self {
        Self {
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
        }
    }
}

#[cfg(target_family = "wasm")]
impl JsonHttpTransport for WakiJsonTransport {
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, CoreError> {
        let response = waki::Client::new()
            .post(url)
            .connect_timeout(std::time::Duration::from_secs(self.connect_timeout_secs))
            .json(body)
            .send()
            .map_err(|send_error| CoreError::TransportFailed(send_error.to_string()))?;

        let status = response.status_code();
        if !(200..300).contains(&status) {
            return Err(CoreError::HttpStatus(status));
        }

        response
            .json::<Value>()
            .map_err(|decode_error| CoreError::MalformedResponse(decode_error.to_string()))
    }
}

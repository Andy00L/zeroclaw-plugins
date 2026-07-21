//! Shared mock transport for host tests. No test in this crate touches the
//! network: every RPC interaction replays canned responses, most of them
//! captured verbatim from mainnet (see tests/fixtures/).

use std::cell::RefCell;
use std::collections::VecDeque;

use serde_json::Value;
use solana_wasip2_core::error::CoreError;
use solana_wasip2_core::http::JsonHttpTransport;

pub struct MockTransport {
    queued_responses: RefCell<VecDeque<Result<Value, CoreError>>>,
    pub recorded_requests: RefCell<Vec<Value>>,
}

impl MockTransport {
    pub fn with_responses(responses: Vec<Result<Value, CoreError>>) -> Self {
        Self {
            queued_responses: RefCell::new(responses.into()),
            recorded_requests: RefCell::new(Vec::new()),
        }
    }

    /// Queue fixture files (raw JSON text) as successful responses, in order.
    pub fn from_fixtures(fixture_texts: &[&str]) -> Self {
        let parsed_responses = fixture_texts
            .iter()
            .map(|fixture_text| {
                Ok(serde_json::from_str::<Value>(fixture_text)
                    .expect("fixture file must be valid JSON"))
            })
            .collect();
        Self::with_responses(parsed_responses)
    }
}

impl JsonHttpTransport for MockTransport {
    fn post_json(&self, _url: &str, body: &Value) -> Result<Value, CoreError> {
        self.recorded_requests.borrow_mut().push(body.clone());
        self.queued_responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| {
                Err(CoreError::TransportFailed(
                    "mock transport: no queued response left".to_string(),
                ))
            })
    }
}

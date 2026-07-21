//! A ZeroClaw WIT tool plugin: `solana_pay_request`.
//!
//! Turns "charge table 4 for 25 USDC" into a Solana Pay transfer request URL
//! with a fresh reference key for payment tracking. The recipient wallet is
//! operator-configured only: the model has no argument to redirect funds.
//! Zero secrets, zero RPC calls, custody tier T1 (it builds a request a
//! human pays from their own wallet).
//!
//! The pure request core lives in [`pay_request`] with no wasm dependency,
//! so it compiles and tests on the host with a plain `cargo test`; the wasm
//! component reuses the exact same logic through this shim.
//!
//! Build:  rustup target add wasm32-wasip2
//!         cargo build --target wasm32-wasip2 --release

pub mod pay_request;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "../../wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });

    use crate::pay_request::execute_pay_request;
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use zeroclaw::plugin::logging::{
        log_record, LogLevel, PluginAction, PluginEvent, PluginOutcome,
    };

    struct SolanaPayRequest;

    const PLUGIN_NAME: &str = "solana-pay-request";
    const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
    const TOOL_NAME: &str = "solana_pay_request";

    /// Fill a reference key from the WASI 0.2 random interface. A failure to
    /// gather entropy aborts the call rather than issuing a predictable
    /// reference.
    fn generate_reference_bytes() -> [u8; 32] {
        let mut reference_bytes = [0u8; 32];
        getrandom::fill(&mut reference_bytes).expect("WASI random interface must be available");
        reference_bytes
    }

    impl PluginInfo for SolanaPayRequest {
        fn plugin_name() -> String {
            PLUGIN_NAME.to_string()
        }

        fn plugin_version() -> String {
            PLUGIN_VERSION.to_string()
        }
    }

    impl Tool for SolanaPayRequest {
        fn name() -> String {
            TOOL_NAME.to_string()
        }

        fn description() -> String {
            "Create a Solana Pay payment request URL so a customer can pay the operator's \
             configured wallet. Give the amount and optionally the token symbol (default \
             USDC), a label, a message, and a memo. Returns the solana: URL plus a unique \
             reference address for confirming the payment later. The receiving wallet is \
             fixed by the operator's configuration and cannot be chosen here."
                .to_string()
        }

        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "amount": {
                        "type": "string",
                        "description": "Decimal amount to request, for example \"25\" or \"0.5\"."
                    },
                    "token": {
                        "type": "string",
                        "description": "Configured token symbol, for example USDC (default) or SOL."
                    },
                    "label": {
                        "type": "string",
                        "description": "Short label the payer's wallet displays, for example the shop name."
                    },
                    "message": {
                        "type": "string",
                        "description": "One-line description of what is being paid for."
                    },
                    "memo": {
                        "type": "string",
                        "description": "Memo recorded on chain with the payment, for example an invoice id."
                    }
                },
                "required": ["amount"]
            })
            .to_string()
        }

        fn execute(args: String) -> Result<ToolResult, String> {
            let outcome = execute_pay_request(&args, generate_reference_bytes);
            let (action, wit_outcome, message) = if outcome.success {
                (PluginAction::Complete, PluginOutcome::Success, "payment request created")
            } else {
                (PluginAction::Fail, PluginOutcome::Failure, "payment request rejected")
            };
            log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "solana_pay_request::tool::execute".to_string(),
                    action,
                    outcome: Some(wit_outcome),
                    duration_ms: None,
                    attrs: None,
                    message: message.to_string(),
                },
            );
            Ok(ToolResult {
                success: outcome.success,
                output: outcome.output,
                error: outcome.error,
            })
        }
    }

    export!(SolanaPayRequest);
}

//! A ZeroClaw WIT tool plugin: `payment_watch`.
//!
//! Watches a Solana Pay reference address and verifies settlement honestly:
//! by the operator-configured recipient's balance delta inside each
//! transaction, never by the mere existence of a transaction touching the
//! reference. Designed to be polled by a cron-triggered SOP (see
//! examples/sop/). Read-only, custody tier T0.
//!
//! The pure watch core lives in [`payment_watch`] with no wasm dependency,
//! so it compiles and tests on the host with a plain `cargo test`; the wasm
//! component reuses the exact same logic through this shim.
//!
//! Build:  rustup target add wasm32-wasip2
//!         cargo build --target wasm32-wasip2 --release

pub mod payment_watch;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "../../wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });

    use crate::payment_watch::execute_payment_watch;
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use solana_wasip2_core::http::WakiJsonTransport;
    use zeroclaw::plugin::logging::{
        log_record, LogLevel, PluginAction, PluginEvent, PluginOutcome,
    };

    struct PaymentWatch;

    const PLUGIN_NAME: &str = "payment-watch";
    const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
    const TOOL_NAME: &str = "payment_watch";

    impl PluginInfo for PaymentWatch {
        fn plugin_name() -> String {
            PLUGIN_NAME.to_string()
        }

        fn plugin_version() -> String {
            PLUGIN_VERSION.to_string()
        }
    }

    impl Tool for PaymentWatch {
        fn name() -> String {
            TOOL_NAME.to_string()
        }

        fn description() -> String {
            "Check whether a Solana Pay payment request has been paid. Give the reference \
             address and the expected amount (and optionally the token symbol, default \
             USDC). Returns PAID, PARTIAL, or PENDING with on-chain evidence, verified by \
             the operator wallet's actual balance change, not by transactions merely \
             touching the reference. Read-only. Call it again later (or from a cron SOP) \
             while a payment is pending."
                .to_string()
        }

        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "reference": {
                        "type": "string",
                        "description": "Base58 reference address from the payment request."
                    },
                    "amount": {
                        "type": "string",
                        "description": "Expected decimal amount, for example \"25\"."
                    },
                    "token": {
                        "type": "string",
                        "description": "Configured token symbol, for example USDC (default) or SOL."
                    },
                    "cursor": {
                        "type": "string",
                        "description": "Cursor line from a previous call, to scan only newer transactions."
                    }
                },
                "required": ["reference", "amount"]
            })
            .to_string()
        }

        fn execute(args: String) -> Result<ToolResult, String> {
            let outcome = execute_payment_watch(WakiJsonTransport::default(), &args);
            let (action, wit_outcome, message) = if outcome.success {
                (
                    PluginAction::Complete,
                    PluginOutcome::Success,
                    "settlement check completed",
                )
            } else {
                (
                    PluginAction::Fail,
                    PluginOutcome::Failure,
                    "settlement check failed",
                )
            };
            log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "payment_watch::tool::execute".to_string(),
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

    export!(PaymentWatch);
}

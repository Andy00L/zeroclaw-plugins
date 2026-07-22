//! A ZeroClaw WIT tool plugin: `spl_transfer_build`.
//!
//! Builds an unsigned SPL token transfer (base64 v0 transaction) from the
//! operator's wallet to an allowlisted recipient, with a hard per-call cap,
//! an on-chain mint risk gate, automatic recipient-account creation, and
//! optional durable nonce lifetime so an approval queue cannot outlive the
//! blockhash. The plugin holds no key and cannot send anything: custody
//! tier T1. A human (or the host's signing flow) signs elsewhere.
//!
//! The pure builder core lives in [`transfer_build`] with no wasm
//! dependency, so it compiles and tests on the host with a plain
//! `cargo test`; the wasm component reuses the exact same logic through
//! this shim.
//!
//! Build:  rustup target add wasm32-wasip2
//!         cargo build --target wasm32-wasip2 --release

pub mod transfer_build;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "../../wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });

    use crate::transfer_build::execute_transfer_build;
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use solana_wasip2_core::http::WakiJsonTransport;
    use zeroclaw::plugin::logging::{
        log_record, LogLevel, PluginAction, PluginEvent, PluginOutcome,
    };

    struct SplTransferBuild;

    const PLUGIN_NAME: &str = "spl-transfer-build";
    const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
    const TOOL_NAME: &str = "spl_transfer_build";

    impl PluginInfo for SplTransferBuild {
        fn plugin_name() -> String {
            PLUGIN_NAME.to_string()
        }

        fn plugin_version() -> String {
            PLUGIN_VERSION.to_string()
        }
    }

    impl Tool for SplTransferBuild {
        fn name() -> String {
            TOOL_NAME.to_string()
        }

        fn description() -> String {
            "Build an UNSIGNED Solana SPL token transfer transaction from the operator's \
             wallet to a recipient on the operator's allowlist. Returns a base64 \
             transaction plus a human-readable summary for the operator to verify and \
             sign in their own wallet; this tool cannot sign or send anything. Amounts \
             are capped per call. Use for requests like \"send 20 USDC to <allowlisted \
             address>\"."
                .to_string()
        }

        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "Base58 wallet address of the recipient. Must be on the operator's allowlist."
                    },
                    "amount": {
                        "type": "string",
                        "description": "Decimal amount to send, for example \"20\" or \"0.5\"."
                    },
                    "token": {
                        "type": "string",
                        "description": "Configured token symbol, for example USDC (default)."
                    },
                    "memo": {
                        "type": "string",
                        "description": "Optional memo recorded on chain, for example an invoice id."
                    }
                },
                "required": ["recipient", "amount"]
            })
            .to_string()
        }

        fn execute(args: String) -> Result<ToolResult, String> {
            let outcome = execute_transfer_build(WakiJsonTransport::default(), &args);
            let (action, wit_outcome, message) = if outcome.success {
                (
                    PluginAction::Complete,
                    PluginOutcome::Success,
                    "unsigned transfer built",
                )
            } else {
                (
                    PluginAction::Reject,
                    PluginOutcome::Failure,
                    "transfer refused",
                )
            };
            log_record(
                LogLevel::Info,
                &PluginEvent {
                    function_name: "spl_transfer_build::tool::execute".to_string(),
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

    export!(SplTransferBuild);
}

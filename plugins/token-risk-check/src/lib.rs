//! A ZeroClaw WIT tool plugin: `token_risk_check`.
//!
//! Red/amber/green risk report for any Solana token mint: mint and freeze
//! authorities, Token-2022 extensions (permanent delegate, transfer hooks,
//! transfer fees, frozen-by-default accounts), and holder concentration.
//! Read-only, custody tier T0: it holds no key and can move nothing.
//!
//! The pure risk core lives in [`risk_check`] with no wasm dependency, so it
//! compiles and tests on the host with a plain `cargo test`; the wasm
//! component reuses the exact same logic through this shim.
//!
//! Build:  rustup target add wasm32-wasip2
//!         cargo build --target wasm32-wasip2 --release

pub mod risk_check;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "../../wit/v0",
        world: "tool-plugin",
        features: ["plugins-wit-v0"],
    });

    use crate::risk_check::execute_risk_check;
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use exports::zeroclaw::plugin::tool::{Guest as Tool, ToolResult};
    use solana_wasip2_core::http::WakiJsonTransport;
    use zeroclaw::plugin::logging::{
        log_record, LogLevel, PluginAction, PluginEvent, PluginOutcome,
    };

    struct TokenRiskCheck;

    const PLUGIN_NAME: &str = "token-risk-check";
    const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
    const TOOL_NAME: &str = "token_risk_check";

    impl PluginInfo for TokenRiskCheck {
        fn plugin_name() -> String {
            PLUGIN_NAME.to_string()
        }

        fn plugin_version() -> String {
            PLUGIN_VERSION.to_string()
        }
    }

    impl Tool for TokenRiskCheck {
        fn name() -> String {
            TOOL_NAME.to_string()
        }

        fn description() -> String {
            "Assess the on-chain risk of a Solana token mint before recommending, \
             valuing, or transferring it. Returns a RED, AMBER, or GREEN level with \
             the reasons: mint and freeze authorities, Token-2022 extensions such as \
             permanent delegates, transfer hooks, and transfer fees, plus holder \
             concentration. Read-only; it cannot move funds."
                .to_string()
        }

        fn parameters_schema() -> String {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "mint": {
                        "type": "string",
                        "description": "Base58 address of the token mint to inspect."
                    }
                },
                "required": ["mint"]
            })
            .to_string()
        }

        fn execute(args: String) -> Result<ToolResult, String> {
            let outcome = execute_risk_check(WakiJsonTransport::default(), &args);
            emit(&outcome);
            Ok(ToolResult {
                success: outcome.success,
                output: outcome.output,
                error: outcome.error,
            })
        }
    }

    fn emit(outcome: &crate::risk_check::ToolOutcome) {
        let (action, wit_outcome, message) = if outcome.success {
            (
                PluginAction::Complete,
                PluginOutcome::Success,
                "risk report produced",
            )
        } else {
            (
                PluginAction::Fail,
                PluginOutcome::Failure,
                "risk check failed",
            )
        };
        log_record(
            LogLevel::Info,
            &PluginEvent {
                function_name: "token_risk_check::tool::execute".to_string(),
                action,
                outcome: Some(wit_outcome),
                duration_ms: None,
                attrs: None,
                message: message.to_string(),
            },
        );
    }

    export!(TokenRiskCheck);
}

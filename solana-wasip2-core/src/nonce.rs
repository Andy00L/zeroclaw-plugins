//! Durable nonce account parsing.
//!
//! Durable nonces remove the ~90 second blockhash lifetime, which is what an
//! approval-gated agent payment needs: the human may sign minutes or hours
//! after the transaction was built. The nonce account's stored hash goes in
//! the transaction's recent_blockhash field and `AdvanceNonceAccount` must be
//! the first instruction (sourceRef:
//! https://solana.com/docs/core/transactions/durable-nonces).

use solana_hash::Hash;
use solana_nonce::state::State;
use solana_nonce::versions::Versions;
use solana_pubkey::Pubkey;

use crate::error::CoreError;

/// The useful content of an initialized durable nonce account.
#[derive(Debug)]
pub struct NonceInfo {
    /// The account that must sign `AdvanceNonceAccount`.
    pub authority: Pubkey,
    /// The stored durable nonce hash, used as the transaction's
    /// recent_blockhash.
    pub nonce_value: Hash,
}

/// Parse raw nonce account data (bincode, as stored on chain by the System
/// program). Only `Versions::Current` + `State::Initialized` is accepted:
/// legacy nonce state no longer validates durable transactions (sourceRef:
/// solana-nonce-3.2.1/src/versions.rs, verify_recent_blockhash returns None
/// for Legacy).
pub fn parse_nonce_account_data(account_data: &[u8]) -> Result<NonceInfo, CoreError> {
    let versions: Versions = bincode::deserialize(account_data).map_err(|decode_error| {
        CoreError::InvalidNonceAccount(format!("undecodable nonce state: {decode_error}"))
    })?;
    match versions {
        Versions::Legacy(_) => Err(CoreError::InvalidNonceAccount(
            "legacy nonce version; the account must be re-initialized".to_string(),
        )),
        Versions::Current(state) => match *state {
            State::Uninitialized => Err(CoreError::InvalidNonceAccount(
                "nonce account is uninitialized".to_string(),
            )),
            State::Initialized(nonce_data) => Ok(NonceInfo {
                authority: nonce_data.authority,
                nonce_value: nonce_data.blockhash(),
            }),
        },
    }
}

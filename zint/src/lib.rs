//! Zink testing framework.
#![deny(missing_docs)]

mod bytes;
mod contract;
mod evm;
mod lookup;

/// Shared utilities for the `zint` crate.
pub mod utils;

pub use self::{
    bytes::Bytes32,
    contract::Contract,
    evm::{Info, EVM},
};

pub use hex;
pub use revm::primitives::{HaltReason, OutOfGasError, U256};
pub use tracing as log;
pub use zabi::selector::keccak256;

/// Set up the logger.
pub fn setup_logger() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .compact()
        .try_init()
        .ok();
}

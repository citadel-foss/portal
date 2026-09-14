//! Domain operations. Each module holds the behavior a command performs; hosts supply only
//! the transport, the authenticated caller and their own native concerns.

pub mod chain_backend;
pub mod maker;
pub mod maker_wallet;
pub mod taker_swap;
pub mod taker_wallet;
pub mod logs;
pub mod maker_reports;
pub mod maker_settings;
pub mod market;
pub mod setup;
pub mod taker_reports;

//! Transport-free Portal services.
//!
//! This crate owns wallet, swap and router behavior; the desktop and web hosts own their own
//! transports, sessions and native integration. Nothing here may depend on Tauri, Axum, HTTP or
//! a native window — that boundary is what lets one set of wallet fixes serve both hosts.

pub mod error;
pub mod events;
pub mod logging;
pub mod operations;
pub mod ops;
pub mod security;
pub mod state;
pub mod storage;
pub mod tor;
pub mod types;

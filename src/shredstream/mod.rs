//! Raw ShredStream-compatible client API.
//!
//! This module intentionally does not connect to Jito ShredStream. It keeps the
//! high-level subscription shape from `sol-parser-sdk::shredstream`, but the
//! transport is raw UDP Solana shreds decoded by [`crate::shred`].

mod client;
mod config;
pub mod dex;
pub mod pump_ix;

pub mod proto {
    tonic::include_proto!("shredstream");
}

pub use client::{dropped_events, ShredStreamClient};
pub use config::ShredStreamConfig;
pub use dex::{parse_transaction_dex_events, parse_transaction_dex_events_with_filter};
pub use proto::*;

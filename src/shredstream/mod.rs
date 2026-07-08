//! Multi-source ShredStream-compatible client API.
//!
//! The high-level subscription shape matches `sol-parser-sdk::shredstream`.
//! Decode/source selection is controlled by [`ShredDecodeMode`]: native raw UDP
//! shreds are decoded locally by [`crate::shred`], while Jito-style gRPC entries
//! are retained as a compatibility source.

mod client;
mod config;
pub mod dex;
pub mod pump_ix;

pub mod proto {
    tonic::include_proto!("shredstream");
}

pub use client::{dropped_events, ShredStreamClient};
pub use config::{JitoShredStreamConfig, ShredDecodeMode, ShredStreamConfig};
pub use dex::{parse_transaction_dex_events, parse_transaction_dex_events_with_filter};
pub use proto::*;

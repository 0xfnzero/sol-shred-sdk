//! Multi-DEX transaction parser entry points for shred/entry ingestion.
//!
//! The implementation currently lives in `pump_ix` for compatibility with the
//! migrated `sol-parser-sdk` shredstream code. New code should use this module.

pub use super::pump_ix::{parse_transaction_dex_events, parse_transaction_dex_events_with_filter};

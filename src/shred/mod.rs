//! Raw Solana shred decoding.
//!
//! This module is the low-latency path of the SDK:
//! UDP payload -> Agave `Shred` -> optional FEC recovery -> deshred -> `Entry`
//! -> `VersionedTransaction`.

mod client;
mod config;
mod decoder;
mod error;
mod reassembler;

pub use client::RawShredClient;
pub use config::RawShredConfig;
pub use decoder::{entries_to_transactions, entries_to_tx_batch, ShredEntryBatch, ShredTxBatch};
pub use error::{ShredDecodeError, ShredResult};
pub use reassembler::{RawShredDecoder, ShredDecoderStats};

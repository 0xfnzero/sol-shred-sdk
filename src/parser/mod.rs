//! Transaction-level event parsers.

mod pumpfun;

use solana_sdk::transaction::VersionedTransaction;

use crate::common::AnyResult;

pub use pumpfun::{PumpfunEventParser, PumpfunParserConfig};

/// Parser plug-in used by raw shred ingestion.
///
/// `sol-shred-sdk` owns shred networking/reassembly. Higher-level crates can
/// implement this trait for their own event enum without reimplementing raw
/// shred decoding.
pub trait TransactionEventParser {
    type Event: Send + 'static;

    fn parse_transaction_events<F>(
        &mut self,
        transaction: &VersionedTransaction,
        slot: u64,
        tx_index: u64,
        recv_us: i64,
        emit: F,
    ) -> AnyResult<usize>
    where
        F: FnMut(Self::Event);
}

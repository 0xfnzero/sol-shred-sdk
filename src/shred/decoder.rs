use solana_entry::entry::Entry;
use solana_sdk::transaction::VersionedTransaction;

/// A contiguous decoded Entry segment from one slot.
#[derive(Debug, Clone)]
pub struct ShredEntryBatch {
    pub slot: u64,
    pub entries: Vec<Entry>,
}

/// Flattened transactions from decoded entries.
#[derive(Debug, Clone)]
pub struct ShredTxBatch {
    pub slot: u64,
    pub transactions: Vec<VersionedTransaction>,
}

#[inline]
pub fn entries_to_transactions(entries: &[Entry]) -> Vec<&VersionedTransaction> {
    entries
        .iter()
        .flat_map(|entry| entry.transactions.iter())
        .collect()
}

#[inline]
pub fn entries_to_tx_batch(batch: ShredEntryBatch) -> ShredTxBatch {
    let transactions = batch
        .entries
        .into_iter()
        .flat_map(|entry| entry.transactions)
        .collect();

    ShredTxBatch {
        slot: batch.slot,
        transactions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_entry_batch_flattens_to_empty_tx_batch() {
        let batch = entries_to_tx_batch(ShredEntryBatch {
            slot: 7,
            entries: Vec::new(),
        });

        assert_eq!(batch.slot, 7);
        assert!(batch.transactions.is_empty());
    }
}

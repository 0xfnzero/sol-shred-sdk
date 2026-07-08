use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use solana_entry::entry::Entry;
use solana_ledger::shred::{ReedSolomonCache, Shred, ShredId, Shredder};

use super::config::RawShredConfig;
use super::decoder::ShredEntryBatch;
use super::error::{ShredDecodeError, ShredResult};

const MERKLE_SHRED_SERIALIZED_LEN: usize = 1203;

#[derive(Debug, Default, Clone, Copy)]
pub struct ShredDecoderStats {
    pub udp_packets: u64,
    pub parse_errors: u64,
    pub data_shreds: u64,
    pub coding_shreds: u64,
    pub stale_shreds: u64,
    pub fec_recover_attempts: u64,
    pub fec_recover_failures: u64,
    pub fec_recovered_data_shreds: u64,
    pub deshred_failures: u64,
    pub entry_decode_failures: u64,
    pub emitted_entry_batches: u64,
    pub emitted_entries: u64,
    pub emitted_transactions: u64,
}

struct DataShred {
    shred: Shred,
    data_complete: bool,
}

struct SlotBuffer {
    data: BTreeMap<u32, DataShred>,
    next_data_index: u32,
    last_activity: Instant,
}

impl SlotBuffer {
    fn new(now: Instant) -> Self {
        Self {
            data: BTreeMap::new(),
            next_data_index: 0,
            last_activity: now,
        }
    }
}

/// Stateful raw shred decoder.
///
/// The decoder owns all incomplete slot state. It is intentionally synchronous
/// so callers can keep it on a pinned OS thread or inside a tight UDP receive
/// loop without async scheduling on the hot path.
pub struct RawShredDecoder {
    config: RawShredConfig,
    forward_watermark: Option<u64>,
    slots: BTreeMap<u64, SlotBuffer>,
    coding: BTreeMap<(u64, u32), HashMap<ShredId, Shred>>,
    fec_recovery_inputs: HashMap<(u64, u32), usize>,
    rs_cache: ReedSolomonCache,
    stats: ShredDecoderStats,
}

impl RawShredDecoder {
    pub fn new(config: RawShredConfig) -> Self {
        Self {
            config,
            forward_watermark: None,
            slots: BTreeMap::new(),
            coding: BTreeMap::new(),
            fec_recovery_inputs: HashMap::new(),
            rs_cache: ReedSolomonCache::default(),
            stats: ShredDecoderStats::default(),
        }
    }

    #[inline]
    pub fn stats(&self) -> ShredDecoderStats {
        self.stats
    }

    /// Push one UDP payload and return all entry batches newly completed by it.
    pub fn push_packet(&mut self, packet: &[u8], now: Instant) -> Vec<ShredEntryBatch> {
        self.stats.udp_packets += 1;

        let shred = match self.parse_udp_packet(packet) {
            Ok(shred) => shred,
            Err(_) => {
                self.stats.parse_errors += 1;
                return Vec::new();
            }
        };

        let slot = shred.slot();
        if self.config.forward_slot_watermark {
            if let Some(watermark) = self.forward_watermark {
                if slot < watermark {
                    self.stats.stale_shreds += 1;
                    return Vec::new();
                }
            }
        }

        if !self.slots.contains_key(&slot) {
            self.evict_excess_slots(slot);
        }
        let fec_set = shred.fec_set_index();
        let buf = self
            .slots
            .entry(slot)
            .or_insert_with(|| SlotBuffer::new(now));
        buf.last_activity = now;

        if shred.is_code() {
            self.stats.coding_shreds += 1;
            self.coding
                .entry((slot, fec_set))
                .or_default()
                .insert(shred.id(), shred);
        } else {
            self.stats.data_shreds += 1;
            let index = shred.index();
            let data_complete = shred.data_complete();
            buf.data.insert(
                index,
                DataShred {
                    shred,
                    data_complete,
                },
            );
        }

        self.try_fec_recover(slot, fec_set, now);
        let out = self.drain_ready_segments(slot);
        self.note_emitted(&out);
        out
    }

    pub fn evict_stale_slots(&mut self, now: Instant) -> usize {
        let timeout = self.config.reassembly_gap_timeout;
        let mut removed = 0usize;

        self.slots.retain(|_, buffer| {
            let keep = now.duration_since(buffer.last_activity) <= timeout;
            if !keep {
                removed += 1;
            }
            keep
        });
        self.coding
            .retain(|(slot, _), _| self.slots.contains_key(slot));
        self.fec_recovery_inputs
            .retain(|(slot, _), _| self.slots.contains_key(slot));

        removed
    }

    fn parse_udp_packet(&self, packet: &[u8]) -> ShredResult<Shred> {
        if self.config.udp_payload_prefix_skip > 0 {
            let Some(slice) = packet.get(self.config.udp_payload_prefix_skip..) else {
                return Err(ShredDecodeError::Parse(format!(
                    "packet length {} is shorter than configured prefix skip {}",
                    packet.len(),
                    self.config.udp_payload_prefix_skip
                )));
            };
            return Ok(Shred::new_from_serialized_shred(slice.to_vec())?);
        }

        match Shred::new_from_serialized_shred(packet.to_vec()) {
            Ok(shred) => Ok(shred),
            Err(error) => {
                if packet.len() >= 64 + MERKLE_SHRED_SERIALIZED_LEN
                    && packet[..64].iter().all(|&byte| byte == 0)
                {
                    return Ok(Shred::new_from_serialized_shred(packet[64..].to_vec())?);
                }
                Err(error.into())
            }
        }
    }

    fn note_emitted(&mut self, batches: &[ShredEntryBatch]) {
        if batches.is_empty() {
            return;
        }

        let mut max_slot = self.forward_watermark.unwrap_or_default();
        for batch in batches {
            max_slot = max_slot.max(batch.slot);
            self.stats.emitted_entry_batches += 1;
            self.stats.emitted_entries += batch.entries.len() as u64;
            self.stats.emitted_transactions += batch
                .entries
                .iter()
                .map(|entry| entry.transactions.len() as u64)
                .sum::<u64>();
        }

        if self.config.forward_slot_watermark {
            self.forward_watermark = Some(max_slot);
        }
    }

    fn evict_excess_slots(&mut self, touch_slot: u64) {
        while self.slots.len() >= self.config.max_tracked_slots.max(1) {
            let Some(min_slot) = self.slots.keys().next().copied() else {
                break;
            };
            if min_slot == touch_slot && self.slots.len() == 1 {
                break;
            }
            self.slots.remove(&min_slot);
            self.coding.retain(|(slot, _), _| *slot != min_slot);
            self.fec_recovery_inputs
                .retain(|(slot, _), _| *slot != min_slot);
        }
    }

    fn try_fec_recover(&mut self, slot: u64, fec_set: u32, now: Instant) {
        let coding_count = self
            .coding
            .get(&(slot, fec_set))
            .map(HashMap::len)
            .unwrap_or_default();
        if coding_count == 0 {
            return;
        }

        let data_count = self
            .slots
            .get(&slot)
            .map(|buffer| {
                buffer
                    .data
                    .values()
                    .filter(|data| data.shred.fec_set_index() == fec_set)
                    .count()
            })
            .unwrap_or_default();
        let input_count = data_count + coding_count;
        if input_count < 2 {
            return;
        }

        let input_key = (slot, fec_set);
        if self
            .fec_recovery_inputs
            .get(&input_key)
            .is_some_and(|last_input_count| *last_input_count == input_count)
        {
            return;
        }
        self.fec_recovery_inputs.insert(input_key, input_count);

        let mut shreds = Vec::new();

        if let Some(buffer) = self.slots.get(&slot) {
            for data in buffer.data.values() {
                if data.shred.fec_set_index() == fec_set {
                    shreds.push(data.shred.clone());
                }
            }
        }
        if let Some(coding) = self.coding.get(&(slot, fec_set)) {
            shreds.extend(coding.values().cloned());
        }

        self.stats.fec_recover_attempts += 1;
        match Shredder::try_recovery(shreds, &self.rs_cache) {
            Ok(recovered) => {
                for shred in recovered {
                    if shred.is_data() {
                        let index = shred.index();
                        let data_complete = shred.data_complete();
                        let buffer = self
                            .slots
                            .entry(slot)
                            .or_insert_with(|| SlotBuffer::new(now));
                        buffer.last_activity = now;
                        if buffer
                            .data
                            .insert(
                                index,
                                DataShred {
                                    shred,
                                    data_complete,
                                },
                            )
                            .is_none()
                        {
                            self.stats.fec_recovered_data_shreds += 1;
                        }
                    }
                }
            }
            Err(_) => {
                self.stats.fec_recover_failures += 1;
            }
        }
    }

    fn drain_ready_segments(&mut self, slot: u64) -> Vec<ShredEntryBatch> {
        let mut out = Vec::new();
        let buffer_empty;

        {
            let Some(buffer) = self.slots.get_mut(&slot) else {
                return Vec::new();
            };

            loop {
                let mut chunk = Vec::new();
                let mut index = buffer.next_data_index;
                let complete_index = loop {
                    let Some(data) = buffer.data.get(&index) else {
                        break None;
                    };
                    chunk.push(index);
                    if data.data_complete {
                        break Some(index);
                    }
                    let Some(next_index) = index.checked_add(1) else {
                        break None;
                    };
                    index = next_index;
                };

                let Some(complete_index) = complete_index else {
                    break;
                };

                match Self::decode_chunk(slot, &chunk, buffer, self.config.max_deshred_bytes) {
                    Ok(Some(entries)) => out.push(ShredEntryBatch { slot, entries }),
                    Ok(None) => {}
                    Err(ChunkDecodeError::Deshred) => {
                        self.stats.deshred_failures += 1;
                    }
                    Err(ChunkDecodeError::EntryDecode) => {
                        self.stats.entry_decode_failures += 1;
                    }
                }

                for index in &chunk {
                    buffer.data.remove(index);
                }
                buffer.next_data_index = complete_index.saturating_add(1);
            }

            buffer_empty = buffer.data.is_empty();
        }

        if buffer_empty {
            let has_coding_pending = self
                .coding
                .keys()
                .any(|(coding_slot, _)| *coding_slot == slot);
            if !has_coding_pending {
                self.slots.remove(&slot);
            }
            self.coding
                .retain(|(coding_slot, _), _| self.slots.contains_key(coding_slot));
            self.fec_recovery_inputs
                .retain(|(coding_slot, _), _| self.slots.contains_key(coding_slot));
        }

        out
    }

    fn decode_chunk(
        slot: u64,
        chunk: &[u32],
        buffer: &SlotBuffer,
        max_deshred_bytes: usize,
    ) -> Result<Option<Vec<Entry>>, ChunkDecodeError> {
        let payloads: Vec<&[u8]> = chunk
            .iter()
            .filter_map(|index| {
                buffer
                    .data
                    .get(index)
                    .map(|data| data.shred.payload().as_ref())
            })
            .collect();

        if payloads.len() != chunk.len() {
            return Ok(None);
        }

        let bytes = match Shredder::deshred(payloads) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::trace!("raw shred deshred failed slot={slot}: {error}");
                return Err(ChunkDecodeError::Deshred);
            }
        };

        if bytes.len() > max_deshred_bytes {
            log::trace!(
                "raw shred deshred payload too large slot={slot} len={} max={}",
                bytes.len(),
                max_deshred_bytes
            );
            return Err(ChunkDecodeError::EntryDecode);
        }

        match bincode::deserialize::<Vec<Entry>>(&bytes) {
            Ok(entries) => Ok(Some(entries)),
            Err(error) => {
                log::trace!(
                    "raw shred entry decode failed slot={slot} bytes_len={}: {error}",
                    bytes.len()
                );
                Err(ChunkDecodeError::EntryDecode)
            }
        }
    }
}

enum ChunkDecodeError {
    Deshred,
    EntryDecode,
}

impl Default for RawShredDecoder {
    fn default() -> Self {
        Self::new(RawShredConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_ledger::shred::{ProcessShredsStats, ReedSolomonCache, Shredder};
    use solana_sdk::{
        hash::Hash,
        signature::{Keypair, Signer},
        system_transaction,
    };

    #[test]
    fn bincode_entry_decode_roundtrips_empty_vec() {
        let bytes = bincode::serialize(&Vec::<Entry>::new()).unwrap();
        let entries: Vec<Entry> = bincode::deserialize(&bytes).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn stale_slot_eviction_removes_old_buffers() {
        let mut decoder = RawShredDecoder::new(RawShredConfig {
            reassembly_gap_timeout: std::time::Duration::from_millis(1),
            ..RawShredConfig::default()
        });
        let now = Instant::now();
        decoder.slots.insert(1, SlotBuffer::new(now));

        let removed = decoder.evict_stale_slots(now + std::time::Duration::from_millis(2));
        assert_eq!(removed, 1);
        assert!(decoder.slots.is_empty());
    }

    #[test]
    fn official_solana_shreds_decode_back_to_entries() {
        let slot = 42;
        let parent_slot = 41;
        let shredder = Shredder::new(slot, parent_slot, 0, 0).unwrap();
        let leader_keypair = Keypair::new();
        let from_keypair = Keypair::new();
        let to_keypair = Keypair::new();
        let tx =
            system_transaction::transfer(&from_keypair, &to_keypair.pubkey(), 1, Hash::default());
        let entries = vec![Entry::new(&Hash::default(), 1, vec![tx.clone()])];

        let (data_shreds, _coding_shreds) = shredder.entries_to_shreds(
            &leader_keypair,
            &entries,
            true,
            None,
            0,
            0,
            true,
            &ReedSolomonCache::default(),
            &mut ProcessShredsStats::default(),
        );
        assert!(!data_shreds.is_empty());

        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        let mut batches = Vec::new();
        for shred in data_shreds {
            batches.extend(decoder.push_packet(shred.payload().as_ref(), now));
        }

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].slot, slot);
        assert_eq!(batches[0].entries.len(), 1);
        assert_eq!(batches[0].entries[0].transactions.len(), 1);
        assert_eq!(
            batches[0].entries[0].transactions[0].signatures,
            tx.signatures
        );
        assert_eq!(decoder.stats().emitted_transactions, 1);
    }

    #[test]
    fn default_decoder_allows_out_of_order_completed_slots() {
        fn make_shreds(slot: u64, parent_slot: u64) -> (Vec<Shred>, Vec<Entry>) {
            let shredder = Shredder::new(slot, parent_slot, 0, 0).unwrap();
            let leader_keypair = Keypair::new();
            let from_keypair = Keypair::new();
            let to_keypair = Keypair::new();
            let tx = system_transaction::transfer(
                &from_keypair,
                &to_keypair.pubkey(),
                1,
                Hash::default(),
            );
            let entries = vec![Entry::new(&Hash::default(), 1, vec![tx])];
            let (data_shreds, _coding_shreds) = shredder.entries_to_shreds(
                &leader_keypair,
                &entries,
                true,
                None,
                0,
                0,
                true,
                &ReedSolomonCache::default(),
                &mut ProcessShredsStats::default(),
            );
            (data_shreds, entries)
        }

        let (later_shreds, later_entries) = make_shreds(43, 42);
        let (earlier_shreds, earlier_entries) = make_shreds(42, 41);
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();

        let mut later_batches = Vec::new();
        for shred in later_shreds {
            later_batches.extend(decoder.push_packet(shred.payload().as_ref(), now));
        }
        assert_eq!(later_batches.len(), 1);
        assert_eq!(later_batches[0].slot, 43);
        assert_eq!(later_batches[0].entries.len(), later_entries.len());

        let mut earlier_batches = Vec::new();
        for shred in earlier_shreds {
            earlier_batches.extend(decoder.push_packet(shred.payload().as_ref(), now));
        }
        assert_eq!(earlier_batches.len(), 1);
        assert_eq!(earlier_batches[0].slot, 42);
        assert_eq!(earlier_batches[0].entries.len(), earlier_entries.len());
    }

    #[test]
    fn out_of_order_shreds_wait_for_missing_prefix() {
        let slot = 42;
        let parent_slot = 41;
        let shredder = Shredder::new(slot, parent_slot, 0, 0).unwrap();
        let leader_keypair = Keypair::new();
        let entries: Vec<_> = (0..32)
            .map(|_| {
                let from_keypair = Keypair::new();
                let to_keypair = Keypair::new();
                let tx = system_transaction::transfer(
                    &from_keypair,
                    &to_keypair.pubkey(),
                    1,
                    Hash::default(),
                );
                Entry::new(&Hash::default(), 1, vec![tx])
            })
            .collect();

        let (data_shreds, _coding_shreds) = shredder.entries_to_shreds(
            &leader_keypair,
            &entries,
            true,
            None,
            0,
            0,
            true,
            &ReedSolomonCache::default(),
            &mut ProcessShredsStats::default(),
        );

        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        for shred in data_shreds.iter().skip(1) {
            assert!(decoder
                .push_packet(shred.payload().as_ref(), now)
                .is_empty());
        }

        let batches = decoder.push_packet(data_shreds[0].payload().as_ref(), now);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entries.len(), entries.len());
    }

    #[test]
    #[ignore = "manual release-mode decoder microbench"]
    fn bench_decode_generated_shreds() {
        let slot = 42;
        let parent_slot = 41;
        let shredder = Shredder::new(slot, parent_slot, 0, 0).unwrap();
        let leader_keypair = Keypair::new();
        let entries: Vec<_> = (0..32)
            .map(|_| {
                let from_keypair = Keypair::new();
                let to_keypair = Keypair::new();
                let tx = system_transaction::transfer(
                    &from_keypair,
                    &to_keypair.pubkey(),
                    1,
                    Hash::default(),
                );
                Entry::new(&Hash::default(), 1, vec![tx])
            })
            .collect();

        let (data_shreds, _coding_shreds) = shredder.entries_to_shreds(
            &leader_keypair,
            &entries,
            true,
            None,
            0,
            0,
            true,
            &ReedSolomonCache::default(),
            &mut ProcessShredsStats::default(),
        );
        let packets: Vec<Vec<u8>> = data_shreds
            .iter()
            .map(|shred| shred.payload().as_ref().to_vec())
            .collect();
        let iterations = std::env::var("RAW_SHRED_BENCH_ITERS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(10_000);

        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        let started = Instant::now();
        let mut decoded_batches = 0usize;
        let mut decoded_entries = 0usize;
        let mut decoded_transactions = 0usize;
        for _ in 0..iterations {
            for packet in &packets {
                for batch in decoder.push_packet(packet, now) {
                    decoded_batches += 1;
                    decoded_entries += batch.entries.len();
                    decoded_transactions += batch
                        .entries
                        .iter()
                        .map(|entry| entry.transactions.len())
                        .sum::<usize>();
                }
            }
        }
        let elapsed = started.elapsed();
        let packets_total = iterations * packets.len();
        let packets_per_second = packets_total as f64 / elapsed.as_secs_f64();
        let slots_per_second = decoded_batches as f64 / elapsed.as_secs_f64();
        let tx_per_second = decoded_transactions as f64 / elapsed.as_secs_f64();

        assert_eq!(decoded_batches, iterations);
        assert_eq!(decoded_entries, iterations * entries.len());
        eprintln!(
            "raw_shred_decoder packets={} batches={} transactions={} elapsed_ms={:.3} packets_per_sec={:.0} slots_per_sec={:.0} tx_per_sec={:.0}",
            packets_total,
            decoded_batches,
            decoded_transactions,
            elapsed.as_secs_f64() * 1_000.0,
            packets_per_second,
            slots_per_second,
            tx_per_second,
        );
    }

    #[test]
    fn shred_decode_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}

        assert_send_sync::<ShredDecodeError>();
    }
}

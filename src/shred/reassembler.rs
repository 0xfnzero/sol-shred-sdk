use std::collections::{
    btree_map::Entry as BTreeEntry, hash_map::Entry as HashEntry, BTreeMap, HashMap,
};
use std::num::NonZeroUsize;
use std::time::Instant;

use lru::LruCache;
use reed_solomon_erasure::galois_8::ReedSolomon;
use solana_entry::entry::Entry;
use solana_ledger::shred::{Payload, Shred, ShredId, Shredder};
use solana_sdk::hash::{hashv, Hash};

use super::config::RawShredConfig;
use super::decoder::ShredEntryBatch;
use super::error::{ShredDecodeError, ShredResult};

const MERKLE_SHRED_SERIALIZED_LEN: usize = 1203;
const CODE_SHRED_SERIALIZED_LEN: usize = 1228;
const SHRED_VARIANT_OFFSET: usize = 64;
const CODING_HEADER_OFFSET: usize = 83;
const DATA_SIZE_OFFSET: usize = 86;
const SHRED_DATA_HEADER_LEN: usize = 88;
const SHRED_CODE_HEADER_LEN: usize = 89;
const MERKLE_ROOT_LEN: usize = 32;
const MERKLE_PROOF_ENTRY_LEN: usize = 20;
const RETRANSMITTER_SIGNATURE_LEN: usize = 64;
const RS_CACHE_CAPACITY: usize = 32;
const MAX_SHREDS_PER_FEC_SET: usize = 256;
// Bounds wire-payload retention to roughly 80 MiB plus map overhead.
const MAX_BUFFERED_SHREDS: usize = 65_536;
const MERKLE_HASH_PREFIX_LEAF: &[u8] = b"\x00SOLANA_MERKLE_SHREDS_LEAF";
const MERKLE_HASH_PREFIX_NODE: &[u8] = b"\x01SOLANA_MERKLE_SHREDS_NODE";

#[derive(Debug, Default, Clone, Copy)]
pub struct ShredDecoderStats {
    pub udp_packets: u64,
    pub parse_errors: u64,
    pub data_shreds: u64,
    pub coding_shreds: u64,
    pub stale_shreds: u64,
    pub duplicate_shreds: u64,
    pub conflicting_shreds: u64,
    pub invalid_fec_shreds: u64,
    pub oversized_payloads: u64,
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
    payload: Payload,
    fec_set_index: u32,
    version: u16,
    variant: Option<MerkleVariant>,
    data_complete: bool,
    last_in_slot: bool,
}

struct CodingSet {
    shreds: HashMap<ShredId, Shred>,
    num_data_shreds: usize,
    num_coding_shreds: usize,
    first_coding_index: u32,
    version: u16,
    variant: MerkleVariant,
    merkle_root: Hash,
    recovery_failed: bool,
}

struct SlotBuffer {
    data: BTreeMap<u32, DataShred>,
    next_data_index: u32,
    terminal_complete: bool,
    last_activity: Instant,
}

impl SlotBuffer {
    fn new(now: Instant) -> Self {
        Self {
            data: BTreeMap::new(),
            next_data_index: 0,
            terminal_complete: false,
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
    coding: BTreeMap<(u64, u32), CodingSet>,
    buffered_shreds: usize,
    merkle_rs_cache: LruCache<(usize, usize), ReedSolomon>,
    stats: ShredDecoderStats,
}

impl RawShredDecoder {
    pub fn new(config: RawShredConfig) -> Self {
        Self {
            config,
            forward_watermark: None,
            slots: BTreeMap::new(),
            coding: BTreeMap::new(),
            buffered_shreds: 0,
            merkle_rs_cache: LruCache::new(NonZeroUsize::new(RS_CACHE_CAPACITY).unwrap()),
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

        let new_slot = !self.slots.contains_key(&slot);
        if new_slot && !self.admit_slot(slot) {
            self.stats.stale_shreds += 1;
            return Vec::new();
        }
        let fec_set = shred.fec_set_index();
        let buffer = self
            .slots
            .entry(slot)
            .or_insert_with(|| SlotBuffer::new(now));
        if buffer.terminal_complete {
            self.stats.duplicate_shreds += 1;
            return Vec::new();
        }

        let changed = if shred.is_code() {
            self.stats.coding_shreds += 1;
            self.insert_coding_shred(slot, fec_set, shred)
        } else {
            self.stats.data_shreds += 1;
            self.insert_data_shred(slot, shred)
        };

        if !changed {
            if new_slot
                && self.slots[&slot].data.is_empty()
                && !self
                    .coding
                    .keys()
                    .any(|(coding_slot, _)| *coding_slot == slot)
            {
                self.slots.remove(&slot);
            }
            return Vec::new();
        }
        self.slots
            .get_mut(&slot)
            .expect("slot buffer exists")
            .last_activity = now;

        let recovered_data = self.try_fec_recover(slot, fec_set, now);
        let out = if recovered_data
            || self.slots[&slot]
                .data
                .contains_key(&self.slots[&slot].next_data_index)
        {
            self.drain_ready_segments(slot)
        } else {
            Vec::new()
        };
        self.note_emitted(&out);
        self.enforce_buffer_limit(MAX_BUFFERED_SHREDS);
        self.debug_assert_buffered_count();
        out
    }

    pub fn evict_stale_slots(&mut self, now: Instant) -> usize {
        let timeout = self.config.reassembly_gap_timeout;
        let mut removed = 0usize;

        let stale_slots: Vec<u64> = self
            .slots
            .iter()
            .filter_map(|(&slot, buffer)| {
                (!buffer.terminal_complete
                    && now.saturating_duration_since(buffer.last_activity) > timeout)
                    .then_some(slot)
            })
            .collect();
        for slot in stale_slots {
            removed += usize::from(self.remove_slot_state(slot));
        }
        self.debug_assert_buffered_count();

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

        if packet.len() >= 64 + MERKLE_SHRED_SERIALIZED_LEN
            && packet[..64].iter().all(|&byte| byte == 0)
        {
            return Ok(Shred::new_from_serialized_shred(packet[64..].to_vec())?);
        }

        Ok(Shred::new_from_serialized_shred(packet.to_vec())?)
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

    fn admit_slot(&mut self, touch_slot: u64) -> bool {
        while self.slots.len() >= self.config.max_tracked_slots.max(1) {
            let Some(min_slot) = self.slots.keys().next().copied() else {
                break;
            };
            if touch_slot <= min_slot {
                return false;
            }
            self.remove_slot_state(min_slot);
        }
        true
    }

    fn remove_slot_state(&mut self, slot: u64) -> bool {
        let Some(buffer) = self.slots.remove(&slot) else {
            return false;
        };
        let mut removed_shreds = buffer.data.len();
        self.coding.retain(|(coding_slot, _), set| {
            if *coding_slot == slot {
                removed_shreds = removed_shreds.saturating_add(set.shreds.len());
                false
            } else {
                true
            }
        });
        self.buffered_shreds = self.buffered_shreds.saturating_sub(removed_shreds);
        true
    }

    fn remove_coding_set(&mut self, slot: u64, fec_set: u32) {
        if let Some(set) = self.coding.remove(&(slot, fec_set)) {
            self.buffered_shreds = self.buffered_shreds.saturating_sub(set.shreds.len());
        }
    }

    fn enforce_buffer_limit(&mut self, limit: usize) {
        while self.buffered_shreds > limit {
            let Some(oldest_slot) = self
                .slots
                .iter()
                .filter(|(_, buffer)| !buffer.terminal_complete)
                .min_by_key(|(slot, buffer)| (buffer.last_activity, **slot))
                .map(|(&slot, _)| slot)
            else {
                break;
            };
            if !self.remove_slot_state(oldest_slot) {
                break;
            }
        }
    }

    #[inline]
    fn debug_assert_buffered_count(&self) {
        debug_assert_eq!(
            self.buffered_shreds,
            self.slots
                .values()
                .map(|buffer| buffer.data.len())
                .sum::<usize>()
                .saturating_add(
                    self.coding
                        .values()
                        .map(|set| set.shreds.len())
                        .sum::<usize>()
                )
        );
    }

    fn insert_data_shred(&mut self, slot: u64, shred: Shred) -> bool {
        let index = shred.index();
        let buffer = self.slots.get_mut(&slot).expect("slot buffer exists");
        if index < buffer.next_data_index {
            self.stats.duplicate_shreds += 1;
            return false;
        }

        let data_complete = shred.data_complete();
        let last_in_slot = shred.last_in_slot();
        let fec_set_index = shred.fec_set_index();
        let version = shred.version();
        let variant = merkle_variant(shred.payload().as_ref());
        let payload = shred.into_payload();
        match buffer.data.entry(index) {
            BTreeEntry::Vacant(entry) => {
                entry.insert(DataShred {
                    payload,
                    fec_set_index,
                    version,
                    variant,
                    data_complete,
                    last_in_slot,
                });
                self.buffered_shreds = self.buffered_shreds.saturating_add(1);
                true
            }
            BTreeEntry::Occupied(entry) if entry.get().payload == payload => {
                self.stats.duplicate_shreds += 1;
                false
            }
            BTreeEntry::Occupied(_) => {
                self.stats.conflicting_shreds += 1;
                false
            }
        }
    }

    fn insert_coding_shred(&mut self, slot: u64, fec_set: u32, shred: Shred) -> bool {
        let Some((num_data_shreds, num_coding_shreds, position)) = coding_header(&shred) else {
            self.stats.invalid_fec_shreds += 1;
            return false;
        };
        let Some(first_coding_index) = shred.index().checked_sub(u32::from(position)) else {
            self.stats.invalid_fec_shreds += 1;
            return false;
        };
        let fec_shred_count =
            usize::from(num_data_shreds).checked_add(usize::from(num_coding_shreds));
        if num_data_shreds == 0
            || num_coding_shreds == 0
            || fec_shred_count.is_none_or(|count| count > MAX_SHREDS_PER_FEC_SET)
            || position >= num_coding_shreds
        {
            self.stats.invalid_fec_shreds += 1;
            return false;
        }
        if fec_set.checked_add(u32::from(num_data_shreds)).is_none() {
            self.stats.invalid_fec_shreds += 1;
            return false;
        }
        if self.slots[&slot].next_data_index > fec_set {
            self.stats.duplicate_shreds += 1;
            return false;
        }
        let Some(variant) = merkle_variant(shred.payload().as_ref()).filter(|variant| variant.code)
        else {
            self.stats.invalid_fec_shreds += 1;
            return false;
        };
        let Ok(merkle_root) = shred.merkle_root() else {
            self.stats.invalid_fec_shreds += 1;
            return false;
        };

        let key = (slot, fec_set);
        let set = self.coding.entry(key).or_insert_with(|| CodingSet {
            shreds: HashMap::new(),
            num_data_shreds: usize::from(num_data_shreds),
            num_coding_shreds: usize::from(num_coding_shreds),
            first_coding_index,
            version: shred.version(),
            variant,
            merkle_root,
            recovery_failed: false,
        });
        if set.num_data_shreds != usize::from(num_data_shreds)
            || set.num_coding_shreds != usize::from(num_coding_shreds)
            || set.first_coding_index != first_coding_index
            || set.version != shred.version()
            || set.variant != variant
            || set.merkle_root != merkle_root
        {
            self.stats.invalid_fec_shreds += 1;
            return false;
        }

        let id = shred.id();
        if let Some(existing) = set.shreds.get(&id) {
            if existing == &shred {
                self.stats.duplicate_shreds += 1;
            } else {
                self.stats.conflicting_shreds += 1;
            }
            return false;
        }
        if let Some(existing) = set.shreds.values().next() {
            if existing.signature() != shred.signature() {
                self.stats.invalid_fec_shreds += 1;
                return false;
            }
        }
        match set.shreds.entry(id) {
            HashEntry::Vacant(entry) => {
                entry.insert(shred);
                self.buffered_shreds = self.buffered_shreds.saturating_add(1);
                true
            }
            HashEntry::Occupied(_) => unreachable!("coding shred ID checked above"),
        }
    }

    fn try_fec_recover(&mut self, slot: u64, fec_set: u32, now: Instant) -> bool {
        let Some(coding_set) = self.coding.get(&(slot, fec_set)) else {
            return false;
        };
        if coding_set.recovery_failed {
            return false;
        }
        let num_data_shreds = coding_set.num_data_shreds;
        let Some(fec_end) = fec_set.checked_add(num_data_shreds as u32) else {
            self.stats.invalid_fec_shreds += 1;
            self.remove_coding_set(slot, fec_set);
            return false;
        };
        let data_count = self
            .slots
            .get(&slot)
            .map(|buffer| {
                buffer
                    .data
                    .values()
                    .filter(|data| {
                        data.fec_set_index == fec_set
                            && solana_ledger::shred::layout::get_index(&data.payload)
                                .is_some_and(|index| index < fec_end)
                    })
                    .count()
            })
            .unwrap_or_default();
        if data_count >= num_data_shreds {
            self.remove_coding_set(slot, fec_set);
            return false;
        }
        if data_count + coding_set.shreds.len() < num_data_shreds {
            return false;
        }

        self.stats.fec_recover_attempts += 1;
        let recovered = self.recover_merkle_data(slot, fec_set, fec_end);

        match recovered {
            Ok(recovered) => {
                let mut inserted = false;
                for recovered in recovered {
                    let buffer = self
                        .slots
                        .entry(slot)
                        .or_insert_with(|| SlotBuffer::new(now));
                    buffer.last_activity = now;
                    if recovered.index < buffer.next_data_index {
                        continue;
                    }
                    if let BTreeEntry::Vacant(entry) = buffer.data.entry(recovered.index) {
                        entry.insert(DataShred {
                            payload: recovered.payload,
                            fec_set_index: fec_set,
                            version: recovered.version,
                            variant: Some(recovered.variant),
                            data_complete: recovered.data_complete,
                            last_in_slot: recovered.last_in_slot,
                        });
                        self.buffered_shreds = self.buffered_shreds.saturating_add(1);
                        self.stats.fec_recovered_data_shreds += 1;
                        inserted = true;
                    }
                }
                let recovered_all_data = self
                    .slots
                    .get(&slot)
                    .map(|buffer| {
                        buffer
                            .data
                            .values()
                            .filter(|data| {
                                data.fec_set_index == fec_set
                                    && solana_ledger::shred::layout::get_index(&data.payload)
                                        .is_some_and(|index| index < fec_end)
                            })
                            .count()
                            >= num_data_shreds
                    })
                    .unwrap_or_default();
                if recovered_all_data {
                    self.remove_coding_set(slot, fec_set);
                }
                inserted
            }
            Err(_) => {
                self.stats.fec_recover_failures += 1;
                if let Some(coding_set) = self.coding.get_mut(&(slot, fec_set)) {
                    coding_set.recovery_failed = true;
                }
                false
            }
        }
    }

    fn recover_merkle_data(
        &mut self,
        slot: u64,
        fec_set: u32,
        fec_end: u32,
    ) -> Result<Vec<RecoveredDataShred>, ()> {
        let coding_set = self.coding.get(&(slot, fec_set)).ok_or(())?;
        let num_data = coding_set.num_data_shreds;
        let num_coding = coding_set.num_coding_shreds;
        let total = num_data.checked_add(num_coding).ok_or(())?;
        let variant = coding_set.variant;
        let shard_len = variant.erasure_shard_len().ok_or(())?;
        let mut shards = vec![None; total];
        let expected_signature = coding_set
            .shreds
            .values()
            .next()
            .map(Shred::signature)
            .ok_or(())?;
        let expected_version = coding_set.version;
        let expected_root = coding_set.merkle_root;
        let chained_root: Option<[u8; MERKLE_ROOT_LEN]> = if variant.chained {
            let offset = SHRED_CODE_HEADER_LEN.checked_add(shard_len).ok_or(())?;
            Some(
                coding_set
                    .shreds
                    .values()
                    .next()
                    .and_then(|shred| shred.payload().get(offset..offset + MERKLE_ROOT_LEN))
                    .ok_or(())?
                    .try_into()
                    .map_err(|_| ())?,
            )
        } else {
            None
        };
        let chained_root_slice = chained_root.as_ref().map(<[u8; MERKLE_ROOT_LEN]>::as_slice);

        for shred in coding_set.shreds.values() {
            let payload = shred.payload().as_ref();
            if shred.signature() != expected_signature
                || shred.version() != expected_version
                || shred.fec_set_index() != fec_set
                || merkle_variant(payload) != Some(variant)
                || shred.merkle_root().ok().as_ref() != Some(&expected_root)
            {
                return Err(());
            }
            if variant.chained {
                let offset = SHRED_CODE_HEADER_LEN.checked_add(shard_len).ok_or(())?;
                if shred.payload().get(offset..offset + MERKLE_ROOT_LEN) != chained_root_slice {
                    return Err(());
                }
            }
            let (_, _, position) = coding_header(shred).ok_or(())?;
            let shard = payload
                .get(SHRED_CODE_HEADER_LEN..SHRED_CODE_HEADER_LEN + shard_len)
                .ok_or(())?;
            let shard_index = num_data.checked_add(usize::from(position)).ok_or(())?;
            let target = shards.get_mut(shard_index).ok_or(())?;
            if target.is_some() {
                return Err(());
            }
            *target = Some(shard.to_vec());
        }

        if let Some(buffer) = self.slots.get(&slot) {
            for data in buffer.data.values() {
                if data.fec_set_index != fec_set {
                    continue;
                }
                let index = solana_ledger::shred::layout::get_index(&data.payload).ok_or(())?;
                if index >= fec_end {
                    continue;
                }
                if solana_ledger::shred::layout::get_slot(&data.payload) != Some(slot)
                    || data.version != expected_version
                    || data.fec_set_index != fec_set
                    || data.variant != Some(variant.as_data())
                {
                    return Err(());
                }
                if variant.chained {
                    let offset = RETRANSMITTER_SIGNATURE_LEN
                        .checked_add(shard_len)
                        .ok_or(())?;
                    if data.payload.get(offset..offset + MERKLE_ROOT_LEN) != chained_root_slice {
                        return Err(());
                    }
                }
                if data.payload.get(..RETRANSMITTER_SIGNATURE_LEN)
                    != Some(expected_signature.as_ref())
                {
                    return Err(());
                }
                let position =
                    usize::try_from(index.checked_sub(fec_set).ok_or(())?).map_err(|_| ())?;
                let shard = data
                    .payload
                    .get(RETRANSMITTER_SIGNATURE_LEN..RETRANSMITTER_SIGNATURE_LEN + shard_len)
                    .ok_or(())?;
                let target = shards.get_mut(position).ok_or(())?;
                if target.is_some() {
                    return Err(());
                }
                *target = Some(shard.to_vec());
            }
        }

        let present_data: Vec<bool> = shards[..num_data].iter().map(Option::is_some).collect();
        let cache_key = (num_data, num_coding);
        if self.merkle_rs_cache.get(&cache_key).is_none() {
            self.merkle_rs_cache.put(
                cache_key,
                ReedSolomon::new(num_data, num_coding).map_err(|_| ())?,
            );
        }
        let rs = self.merkle_rs_cache.get(&cache_key).ok_or(())?;
        rs.reconstruct(&mut shards).map_err(|_| ())?;
        let recovered_root = merkle_root_from_shards(
            &shards,
            num_data,
            coding_set.first_coding_index,
            coding_set
                .shreds
                .values()
                .next()
                .and_then(|shred| {
                    shred
                        .payload()
                        .get(RETRANSMITTER_SIGNATURE_LEN..SHRED_CODE_HEADER_LEN)
                })
                .ok_or(())?,
            chained_root_slice,
        )?;
        if recovered_root != expected_root {
            return Err(());
        }

        let mut recovered =
            Vec::with_capacity(present_data.iter().filter(|present| !**present).count());
        for (position, was_present) in present_data.into_iter().enumerate() {
            if was_present {
                continue;
            }
            let shard = shards.get_mut(position).and_then(Option::take).ok_or(())?;
            if shard.len() != shard_len {
                return Err(());
            }
            let mut payload = vec![0u8; MERKLE_SHRED_SERIALIZED_LEN];
            payload[..RETRANSMITTER_SIGNATURE_LEN].copy_from_slice(expected_signature.as_ref());
            payload[RETRANSMITTER_SIGNATURE_LEN..RETRANSMITTER_SIGNATURE_LEN + shard_len]
                .copy_from_slice(&shard);
            if let Some(chained_root) = chained_root_slice {
                let offset = RETRANSMITTER_SIGNATURE_LEN + shard_len;
                payload[offset..offset + MERKLE_ROOT_LEN].copy_from_slice(chained_root);
            }
            let index = solana_ledger::shred::layout::get_index(&payload).ok_or(())?;
            if index != fec_set.checked_add(position as u32).ok_or(())? {
                return Err(());
            }
            if solana_ledger::shred::layout::get_slot(&payload) != Some(slot)
                || merkle_variant(&payload) != Some(variant.as_data())
            {
                return Err(());
            }
            let payload: Payload = payload.into();
            let parsed = Shred::new_from_serialized_shred(payload.clone()).map_err(|_| ())?;
            if parsed.fec_set_index() != fec_set || parsed.version() != expected_version {
                return Err(());
            }
            shred_data_len(&payload).ok_or(())?;
            let flags = *payload.get(85).ok_or(())?;
            recovered.push(RecoveredDataShred {
                index,
                payload,
                version: expected_version,
                variant: variant.as_data(),
                data_complete: flags & 0x40 != 0,
                last_in_slot: flags & 0xc0 == 0xc0,
            });
        }
        Ok(recovered)
    }

    fn drain_ready_segments(&mut self, slot: u64) -> Vec<ShredEntryBatch> {
        let mut out = Vec::new();
        let mut terminal_complete = false;
        let mut removed_data_shreds = 0usize;

        {
            let Some(buffer) = self.slots.get_mut(&slot) else {
                return Vec::new();
            };

            loop {
                let start_index = buffer.next_data_index;
                let mut index = start_index;
                let complete_index = loop {
                    let Some(data) = buffer.data.get(&index) else {
                        break None;
                    };
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
                let segment_terminal = buffer
                    .data
                    .get(&complete_index)
                    .is_some_and(|data| data.last_in_slot);

                let decoded = match Self::decode_chunk(
                    slot,
                    start_index,
                    complete_index,
                    buffer,
                    self.config.max_deshred_bytes,
                ) {
                    Ok(entries) => {
                        out.push(ShredEntryBatch { slot, entries });
                        true
                    }
                    Err(ChunkDecodeError::Deshred) => {
                        self.stats.deshred_failures += 1;
                        false
                    }
                    Err(ChunkDecodeError::EntryDecode) => {
                        self.stats.entry_decode_failures += 1;
                        false
                    }
                    Err(ChunkDecodeError::PayloadTooLarge) => {
                        self.stats.oversized_payloads += 1;
                        false
                    }
                };

                buffer.next_data_index = complete_index.saturating_add(1);
                if !decoded {
                    for index in start_index..=complete_index {
                        removed_data_shreds += usize::from(buffer.data.remove(&index).is_some());
                    }
                } else {
                    let previous_len = buffer.data.len();
                    buffer.data = buffer.data.split_off(&buffer.next_data_index);
                    removed_data_shreds += previous_len.saturating_sub(buffer.data.len());
                }
                if segment_terminal {
                    terminal_complete = true;
                    buffer.terminal_complete = true;
                    removed_data_shreds = removed_data_shreds.saturating_add(buffer.data.len());
                    buffer.data.clear();
                    break;
                }
            }
        }
        self.buffered_shreds = self.buffered_shreds.saturating_sub(removed_data_shreds);

        let next_data_index = self.slots[&slot].next_data_index;
        let mut removed_coding_shreds = 0usize;
        self.coding.retain(|(coding_slot, fec_set), set| {
            let keep = *coding_slot != slot
                || (!terminal_complete && *fec_set >= next_data_index && set.num_data_shreds > 0);
            if !keep {
                removed_coding_shreds = removed_coding_shreds.saturating_add(set.shreds.len());
            }
            keep
        });
        self.buffered_shreds = self.buffered_shreds.saturating_sub(removed_coding_shreds);
        if terminal_complete {
            self.slots
                .get_mut(&slot)
                .expect("slot buffer exists")
                .terminal_complete = true;
        }

        out
    }

    fn decode_chunk(
        slot: u64,
        start_index: u32,
        complete_index: u32,
        buffer: &SlotBuffer,
        max_deshred_bytes: usize,
    ) -> Result<Vec<Entry>, ChunkDecodeError> {
        let mut deshred_len = 0usize;
        for index in start_index..=complete_index {
            let data = buffer.data.get(&index).ok_or(ChunkDecodeError::Deshred)?;
            let data_len = shred_data_len(&data.payload).ok_or(ChunkDecodeError::Deshred)?;
            deshred_len = deshred_len
                .checked_add(data_len)
                .ok_or(ChunkDecodeError::PayloadTooLarge)?;
            if deshred_len > max_deshred_bytes {
                log::trace!(
                    "raw shred deshred payload too large slot={slot} len={deshred_len} max={max_deshred_bytes}"
                );
                return Err(ChunkDecodeError::PayloadTooLarge);
            }
        }

        let payloads = (start_index..=complete_index).map(|index| {
            buffer
                .data
                .get(&index)
                .expect("contiguous data range checked")
                .payload
                .as_ref()
        });
        let bytes = match Shredder::deshred(payloads) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::trace!("raw shred deshred failed slot={slot}: {error}");
                return Err(ChunkDecodeError::Deshred);
            }
        };

        debug_assert_eq!(bytes.len(), deshred_len);

        match wincode::deserialize_exact::<Vec<Entry>>(&bytes) {
            Ok(entries) => Ok(entries),
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
    PayloadTooLarge,
}

struct RecoveredDataShred {
    index: u32,
    payload: Payload,
    version: u16,
    variant: MerkleVariant,
    data_complete: bool,
    last_in_slot: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MerkleVariant {
    code: bool,
    proof_size: u8,
    chained: bool,
    resigned: bool,
}

impl MerkleVariant {
    fn as_data(self) -> Self {
        Self {
            code: false,
            ..self
        }
    }

    fn erasure_shard_len(self) -> Option<usize> {
        CODE_SHRED_SERIALIZED_LEN.checked_sub(
            SHRED_CODE_HEADER_LEN
                + usize::from(self.chained) * MERKLE_ROOT_LEN
                + usize::from(self.proof_size) * MERKLE_PROOF_ENTRY_LEN
                + usize::from(self.resigned) * RETRANSMITTER_SIGNATURE_LEN,
        )
    }
}

fn merkle_variant(payload: &[u8]) -> Option<MerkleVariant> {
    let variant = *payload.get(SHRED_VARIANT_OFFSET)?;
    let (code, chained, resigned) = match variant & 0xf0 {
        0x40 => (true, false, false),
        0x60 => (true, true, false),
        0x70 => (true, true, true),
        0x80 => (false, false, false),
        0x90 => (false, true, false),
        0xb0 => (false, true, true),
        _ => return None,
    };
    Some(MerkleVariant {
        code,
        proof_size: variant & 0x0f,
        chained,
        resigned,
    })
}

fn merkle_root_from_shards(
    shards: &[Option<Vec<u8>>],
    num_data: usize,
    first_coding_index: u32,
    coding_header_template: &[u8],
    chained_root: Option<&[u8]>,
) -> Result<Hash, ()> {
    if num_data == 0 || num_data >= shards.len() {
        return Err(());
    }
    let chained_len = chained_root.map_or(0, <[u8]>::len);
    let mut nodes = Vec::with_capacity(shards.len());

    for (position, shard) in shards.iter().enumerate() {
        let shard = shard.as_ref().ok_or(())?;
        let mut leaf = Vec::with_capacity(
            if position < num_data {
                shard.len()
            } else {
                SHRED_CODE_HEADER_LEN - RETRANSMITTER_SIGNATURE_LEN + shard.len()
            } + chained_len,
        );
        if position < num_data {
            leaf.extend_from_slice(shard);
        } else {
            let coding_position = position - num_data;
            let coding_index = first_coding_index
                .checked_add(u32::try_from(coding_position).map_err(|_| ())?)
                .ok_or(())?;
            let mut header = coding_header_template.to_vec();
            header
                .get_mut(9..13)
                .ok_or(())?
                .copy_from_slice(&coding_index.to_le_bytes());
            header.get_mut(23..25).ok_or(())?.copy_from_slice(
                &u16::try_from(coding_position)
                    .map_err(|_| ())?
                    .to_le_bytes(),
            );
            leaf.extend_from_slice(&header);
            leaf.extend_from_slice(shard);
        }
        if let Some(chained_root) = chained_root {
            leaf.extend_from_slice(chained_root);
        }
        nodes.push(hashv(&[MERKLE_HASH_PREFIX_LEAF, &leaf]));
    }

    while nodes.len() > 1 {
        let mut parents = Vec::with_capacity(nodes.len().div_ceil(2));
        for pair in nodes.chunks(2) {
            let left = &pair[0];
            let right = pair.get(1).unwrap_or(left);
            parents.push(hashv(&[
                MERKLE_HASH_PREFIX_NODE,
                &left.as_ref()[..MERKLE_PROOF_ENTRY_LEN],
                &right.as_ref()[..MERKLE_PROOF_ENTRY_LEN],
            ]));
        }
        nodes = parents;
    }
    nodes.pop().ok_or(())
}

fn coding_header(shred: &Shred) -> Option<(u16, u16, u16)> {
    if !shred.is_code() {
        return None;
    }
    let payload = shred.payload().as_ref();
    let num_data = u16::from_le_bytes(
        payload
            .get(CODING_HEADER_OFFSET..CODING_HEADER_OFFSET + 2)?
            .try_into()
            .ok()?,
    );
    let num_coding = u16::from_le_bytes(
        payload
            .get(CODING_HEADER_OFFSET + 2..CODING_HEADER_OFFSET + 4)?
            .try_into()
            .ok()?,
    );
    let position = u16::from_le_bytes(
        payload
            .get(CODING_HEADER_OFFSET + 4..CODING_HEADER_OFFSET + 6)?
            .try_into()
            .ok()?,
    );
    Some((num_data, num_coding, position))
}

fn shred_data_len(payload: &[u8]) -> Option<usize> {
    let size = usize::from(u16::from_le_bytes(
        payload
            .get(DATA_SIZE_OFFSET..DATA_SIZE_OFFSET + 2)?
            .try_into()
            .ok()?,
    ));
    let max_size = merkle_variant(payload)
        .and_then(|variant| variant.erasure_shard_len())
        .and_then(|shard_len| RETRANSMITTER_SIGNATURE_LEN.checked_add(shard_len))
        .unwrap_or(payload.len());
    size.checked_sub(SHRED_DATA_HEADER_LEN)
        .filter(|_| size <= max_size)
}

impl Default for RawShredDecoder {
    fn default() -> Self {
        Self::new(RawShredConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_keypair::{Keypair, Signer};
    use solana_ledger::shred::{ProcessShredsStats, ReedSolomonCache, Shredder};
    use solana_sdk::hash::Hash;
    use solana_system_transaction as system_transaction;

    fn make_entries(count: usize) -> Vec<Entry> {
        (0..count)
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
            .collect()
    }

    fn make_shreds(
        slot: u64,
        entries: &[Entry],
        is_last_in_slot: bool,
        next_data_index: u32,
        next_code_index: u32,
    ) -> (Vec<Shred>, Vec<Shred>) {
        Shredder::new(slot, slot.saturating_sub(1), 0, 0)
            .unwrap()
            .entries_to_merkle_shreds_for_tests(
                &Keypair::new(),
                entries,
                is_last_in_slot,
                Hash::default(),
                next_data_index,
                next_code_index,
                &ReedSolomonCache::default(),
                &mut ProcessShredsStats::default(),
            )
    }

    fn make_chained_shreds(
        slot: u64,
        entries: &[Entry],
        is_last_in_slot: bool,
    ) -> (Vec<Shred>, Vec<Shred>) {
        Shredder::new(slot, slot.saturating_sub(1), 0, 0)
            .unwrap()
            .entries_to_merkle_shreds_for_tests(
                &Keypair::new(),
                entries,
                is_last_in_slot,
                Hash::new_unique(),
                0,
                0,
                &ReedSolomonCache::default(),
                &mut ProcessShredsStats::default(),
            )
    }

    #[test]
    fn wincode_entry_decode_roundtrips_empty_vec() {
        let bytes = wincode::serialize(&Vec::<Entry>::new()).unwrap();
        let entries: Vec<Entry> = wincode::deserialize(&bytes).unwrap();
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
    fn stale_eviction_handles_non_monotonic_instants() {
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        decoder
            .slots
            .insert(1, SlotBuffer::new(now + std::time::Duration::from_secs(1)));

        assert_eq!(decoder.evict_stale_slots(now), 0);
        assert!(decoder.slots.contains_key(&1));
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

        let (data_shreds, _coding_shreds) = shredder.entries_to_merkle_shreds_for_tests(
            &leader_keypair,
            &entries,
            true,
            Hash::default(),
            0,
            0,
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
    fn duplicate_completed_slot_shreds_do_not_emit_twice() {
        let entries = make_entries(4);
        let (data_shreds, coding_shreds) = make_shreds(42, &entries, true, 0, 0);
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();

        let mut batches = Vec::new();
        for shred in &data_shreds {
            batches.extend(decoder.push_packet(shred.payload().as_ref(), now));
        }
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entries.len(), entries.len());

        for shred in data_shreds.iter().chain(&coding_shreds) {
            assert!(decoder
                .push_packet(shred.payload().as_ref(), now)
                .is_empty());
        }
        assert_eq!(decoder.stats().emitted_entry_batches, 1);
        assert_eq!(
            decoder.stats().duplicate_shreds,
            (data_shreds.len() + coding_shreds.len()) as u64
        );
        let completed = decoder.slots.get(&42).expect("completed tombstone");
        assert!(completed.terminal_complete);
        assert!(completed.data.is_empty());
        assert!(!decoder
            .coding
            .keys()
            .any(|(coding_slot, _)| *coding_slot == 42));
        assert_eq!(decoder.buffered_shreds, 0);
    }

    #[test]
    fn fec_recovery_waits_for_threshold_and_restores_missing_data() {
        let entries = make_entries(12);
        let (data_shreds, coding_shreds) = make_shreds(42, &entries, true, 0, 0);
        assert_eq!(data_shreds.len(), 32);
        assert!(coding_shreds.len() >= 32);

        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        for shred in data_shreds.iter().skip(1) {
            assert!(decoder
                .push_packet(shred.payload().as_ref(), now)
                .is_empty());
        }
        assert_eq!(decoder.stats().fec_recover_attempts, 0);

        let batches = decoder.push_packet(coding_shreds[0].payload().as_ref(), now);
        assert_eq!(decoder.stats().fec_recover_attempts, 1);
        assert_eq!(decoder.stats().fec_recover_failures, 0);
        assert_eq!(decoder.stats().fec_recovered_data_shreds, 1);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entries.len(), entries.len());
        assert_eq!(decoder.buffered_shreds, 0);
    }

    #[test]
    fn buffered_shred_limit_evicts_oldest_incomplete_slot() {
        let entries = make_entries(1);
        let (mut old_shreds, _) = make_shreds(41, &entries, false, 0, 0);
        let (mut new_shreds, _) = make_shreds(42, &entries, false, 0, 0);
        let now = Instant::now();
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        decoder.slots.insert(41, SlotBuffer::new(now));
        decoder.slots.insert(
            42,
            SlotBuffer::new(now + std::time::Duration::from_millis(1)),
        );

        assert!(decoder.insert_data_shred(41, old_shreds.remove(0)));
        assert!(decoder.insert_data_shred(42, new_shreds.remove(0)));
        assert_eq!(decoder.buffered_shreds, 2);

        decoder.enforce_buffer_limit(1);

        assert!(!decoder.slots.contains_key(&41));
        assert!(decoder.slots.contains_key(&42));
        assert_eq!(decoder.buffered_shreds, 1);
    }

    #[test]
    fn fec_recovery_does_not_run_on_every_early_coding_shred() {
        let entries = make_entries(12);
        let (data_shreds, coding_shreds) = make_shreds(42, &entries, true, 0, 0);
        assert_eq!(data_shreds.len(), 32);

        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        assert!(decoder
            .push_packet(data_shreds[0].payload().as_ref(), now)
            .is_empty());
        for shred in coding_shreds.iter().take(30) {
            assert!(decoder
                .push_packet(shred.payload().as_ref(), now)
                .is_empty());
        }
        assert_eq!(decoder.stats().fec_recover_attempts, 0);

        let batches = decoder.push_packet(coding_shreds[30].payload().as_ref(), now);
        assert_eq!(decoder.stats().fec_recover_attempts, 1);
        assert_eq!(decoder.stats().fec_recovered_data_shreds, 31);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entries.len(), entries.len());
    }

    #[test]
    fn chained_resigned_merkle_recovery_restores_missing_data() {
        let entries = make_entries(12);
        let (data_shreds, coding_shreds) = make_chained_shreds(42, &entries, true);
        assert!(merkle_variant(data_shreds[0].payload().as_ref()).is_some_and(|v| v.resigned));

        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        for shred in data_shreds.iter().skip(1) {
            assert!(decoder
                .push_packet(shred.payload().as_ref(), now)
                .is_empty());
        }
        let batches = decoder.push_packet(coding_shreds[0].payload().as_ref(), now);

        assert_eq!(decoder.stats().fec_recover_failures, 0);
        assert_eq!(decoder.stats().fec_recovered_data_shreds, 1);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entries.len(), entries.len());
    }

    #[test]
    fn mixed_merkle_fec_sets_are_rejected_before_recovery() {
        let entries = make_entries(12);
        let (data_shreds, coding_shreds) = make_shreds(42, &entries, true, 0, 0);
        let (other_data, other_coding) = make_shreds(42, &entries, true, 0, 0);
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        for shred in data_shreds.iter().take(30) {
            decoder.push_packet(shred.payload().as_ref(), now);
        }
        decoder.push_packet(coding_shreds[0].payload().as_ref(), now);

        assert!(decoder
            .push_packet(other_coding[1].payload().as_ref(), now)
            .is_empty());
        assert_eq!(decoder.stats().invalid_fec_shreds, 1);
        assert_eq!(decoder.stats().fec_recover_attempts, 0);

        let batches = decoder.push_packet(other_data[0].payload().as_ref(), now);
        assert!(batches.is_empty());
        assert_eq!(decoder.stats().conflicting_shreds, 1);
    }

    #[test]
    fn merkle_root_mismatch_rejects_recovery() {
        let entries = make_entries(12);
        let (data_shreds, coding_shreds) = make_shreds(42, &entries, true, 0, 0);
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        for shred in data_shreds.iter().skip(1) {
            decoder.push_packet(shred.payload().as_ref(), now);
        }

        let mut corrupted = coding_shreds[0].payload().as_ref().to_vec();
        corrupted[SHRED_CODE_HEADER_LEN] ^= 1;
        let corrupted = Shred::new_from_serialized_shred(corrupted).expect("sanitized shred");
        decoder.push_packet(corrupted.payload().as_ref(), now);

        assert_eq!(decoder.stats().fec_recover_attempts, 1);
        assert_eq!(decoder.stats().fec_recover_failures, 1);
        assert_eq!(decoder.stats().fec_recovered_data_shreds, 0);

        decoder.push_packet(coding_shreds[1].payload().as_ref(), now);
        assert_eq!(decoder.stats().fec_recover_attempts, 1);
        assert_eq!(decoder.stats().fec_recover_failures, 1);
    }

    #[test]
    fn mismatched_coding_root_is_rejected_before_joining_set() {
        let entries = make_entries(12);
        let (data_shreds, coding_shreds) = make_shreds(42, &entries, true, 0, 0);
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        for shred in data_shreds.iter().take(30) {
            decoder.push_packet(shred.payload().as_ref(), now);
        }
        decoder.push_packet(coding_shreds[0].payload().as_ref(), now);

        let mut corrupted = coding_shreds[1].payload().as_ref().to_vec();
        corrupted[SHRED_CODE_HEADER_LEN] ^= 1;
        let corrupted = Shred::new_from_serialized_shred(corrupted).expect("sanitized shred");
        decoder.push_packet(corrupted.payload().as_ref(), now);

        assert_eq!(decoder.stats().invalid_fec_shreds, 1);
        assert_eq!(decoder.coding[&(42, 0)].shreds.len(), 1);
        assert_eq!(decoder.stats().fec_recover_attempts, 0);
    }

    #[test]
    fn invalid_coding_cardinality_is_rejected() {
        let entries = make_entries(1);
        let (_, coding_shreds) = make_shreds(42, &entries, true, 0, 0);
        let mut payload = coding_shreds[0].payload().as_ref().to_vec();
        payload[CODING_HEADER_OFFSET..CODING_HEADER_OFFSET + 2]
            .copy_from_slice(&225u16.to_le_bytes());
        let shred = Shred::new_from_serialized_shred(payload).expect("wire-valid coding shred");
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());

        assert!(decoder
            .push_packet(shred.payload().as_ref(), Instant::now())
            .is_empty());
        assert_eq!(decoder.stats().invalid_fec_shreds, 1);
        assert!(decoder.coding.is_empty());
    }

    #[test]
    fn merkle_reed_solomon_cache_is_bounded() {
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        for data in 1..=RS_CACHE_CAPACITY + 8 {
            let coding = 64 - data;
            decoder
                .merkle_rs_cache
                .put((data, coding), ReedSolomon::new(data, coding).unwrap());
        }
        assert_eq!(decoder.merkle_rs_cache.len(), RS_CACHE_CAPACITY);
    }

    #[test]
    fn multiple_entry_batches_in_one_slot_emit_in_order() {
        let first_entries = make_entries(3);
        let second_entries = make_entries(5);
        let (first_data, first_code) = make_shreds(42, &first_entries, false, 0, 0);
        let (second_data, _) = make_shreds(
            42,
            &second_entries,
            true,
            first_data.len() as u32,
            first_code.len() as u32,
        );

        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        let mut batches = Vec::new();
        for shred in first_data.iter().chain(&second_data) {
            batches.extend(decoder.push_packet(shred.payload().as_ref(), now));
        }

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].entries.len(), first_entries.len());
        assert_eq!(batches[1].entries.len(), second_entries.len());
    }

    #[test]
    fn configured_udp_prefix_decodes_official_shreds() {
        let entries = make_entries(2);
        let (data_shreds, _) = make_shreds(42, &entries, true, 0, 0);
        let prefix = [0xaa, 0xbb, 0xcc];
        let mut decoder = RawShredDecoder::new(RawShredConfig {
            udp_payload_prefix_skip: prefix.len(),
            ..RawShredConfig::default()
        });
        let now = Instant::now();
        let mut batches = Vec::new();

        for shred in data_shreds {
            let mut packet = Vec::with_capacity(prefix.len() + shred.payload().len());
            packet.extend_from_slice(&prefix);
            packet.extend_from_slice(shred.payload().as_ref());
            batches.extend(decoder.push_packet(&packet, now));
        }

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entries.len(), entries.len());
        assert_eq!(decoder.stats().parse_errors, 0);
    }

    #[test]
    fn automatic_zero_signature_prefix_decodes_without_config() {
        let entries = make_entries(2);
        let (data_shreds, _) = make_shreds(42, &entries, true, 0, 0);
        let mut decoder = RawShredDecoder::new(RawShredConfig::default());
        let now = Instant::now();
        let mut batches = Vec::new();

        for shred in data_shreds {
            let mut packet = Vec::with_capacity(64 + shred.payload().len());
            packet.resize(64, 0);
            packet.extend_from_slice(shred.payload().as_ref());
            batches.extend(decoder.push_packet(&packet, now));
        }

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entries.len(), entries.len());
        assert_eq!(decoder.stats().parse_errors, 0);
    }

    #[test]
    fn oversized_complete_segment_is_dropped_once() {
        let entries = make_entries(2);
        let (data_shreds, _) = make_shreds(42, &entries, true, 0, 0);
        let mut decoder = RawShredDecoder::new(RawShredConfig {
            max_deshred_bytes: 1,
            ..RawShredConfig::default()
        });
        let now = Instant::now();

        for shred in &data_shreds {
            assert!(decoder
                .push_packet(shred.payload().as_ref(), now)
                .is_empty());
        }
        assert_eq!(decoder.stats().oversized_payloads, 1);

        for shred in &data_shreds {
            assert!(decoder
                .push_packet(shred.payload().as_ref(), now)
                .is_empty());
        }
        assert_eq!(decoder.stats().oversized_payloads, 1);
    }

    #[test]
    fn completed_slot_tombstones_are_capacity_evicted() {
        let entries = make_entries(1);
        let mut decoder = RawShredDecoder::new(RawShredConfig {
            max_tracked_slots: 2,
            ..RawShredConfig::default()
        });
        let now = Instant::now();

        for slot in 40..43 {
            let (data_shreds, _) = make_shreds(slot, &entries, true, 0, 0);
            for shred in data_shreds {
                decoder.push_packet(shred.payload().as_ref(), now);
            }
        }

        assert_eq!(decoder.slots.len(), 2);
        assert!(!decoder.slots.contains_key(&40));
        assert!(decoder.slots.contains_key(&41));
        assert!(decoder.slots.contains_key(&42));
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
            let (data_shreds, _coding_shreds) = shredder.entries_to_merkle_shreds_for_tests(
                &leader_keypair,
                &entries,
                true,
                Hash::default(),
                0,
                0,
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

        let (data_shreds, _coding_shreds) = shredder.entries_to_merkle_shreds_for_tests(
            &leader_keypair,
            &entries,
            true,
            Hash::default(),
            0,
            0,
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

        let (data_shreds, _coding_shreds) = shredder.entries_to_merkle_shreds_for_tests(
            &leader_keypair,
            &entries,
            true,
            Hash::default(),
            0,
            0,
            &ReedSolomonCache::default(),
            &mut ProcessShredsStats::default(),
        );
        let mut packets: Vec<Vec<u8>> = data_shreds
            .iter()
            .map(|shred| shred.payload().as_ref().to_vec())
            .collect();
        let iterations = std::env::var("RAW_SHRED_BENCH_ITERS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(10_000);

        let mut decoder = RawShredDecoder::new(RawShredConfig {
            max_tracked_slots: 1,
            ..RawShredConfig::default()
        });
        let now = Instant::now();
        let started = Instant::now();
        let mut decoded_batches = 0usize;
        let mut decoded_entries = 0usize;
        let mut decoded_transactions = 0usize;
        for iteration in 0..iterations {
            // Signature verification is deliberately outside RawShredDecoder's
            // trust boundary, so update only the wire slot to exercise distinct
            // completed slots without timing shred generation.
            let iteration_slot = slot + iteration as u64;
            for packet in &mut packets {
                packet[65..73].copy_from_slice(&iteration_slot.to_le_bytes());
            }
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

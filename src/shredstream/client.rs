use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_queue::ArrayQueue;
use futures::StreamExt;
use solana_entry::entry::Entry;
use solana_sdk::message::{v0::LoadedAddresses, VersionedMessage};
use solana_sdk::transaction::VersionedTransaction;
use tokio::task::JoinHandle;
use tonic::transport::{Channel, Endpoint};

use super::config::{JitoShredStreamConfig, ShredDecodeMode, ShredStreamConfig};
use crate::common::logs_events::PumpfunEvent;
use crate::common::AnyResult;
use crate::core::{now_micros, DexEvent};
use crate::grpc::shredstream::shredstream_proxy_client::ShredstreamProxyClient;
use crate::grpc::shredstream::SubscribeEntriesRequest;
use crate::grpc::EventTypeFilter;
use crate::parser::{PumpfunEventParser, PumpfunParserConfig, TransactionEventParser};
use crate::shred::{RawShredClient, RawShredConfig, ShredEntryBatch};

static DROPPED_EVENTS: AtomicU64 = AtomicU64::new(0);

const JITO_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const JITO_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);
const JITO_KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(5);
const JITO_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

enum EventSink<E> {
    Queue(Arc<ArrayQueue<E>>),
    Callback(Arc<dyn Fn(E) + Send + Sync>),
}

impl<E> Clone for EventSink<E> {
    fn clone(&self) -> Self {
        match self {
            Self::Queue(queue) => Self::Queue(Arc::clone(queue)),
            Self::Callback(callback) => Self::Callback(Arc::clone(callback)),
        }
    }
}

impl<E> EventSink<E> {
    #[inline]
    fn deliver(&self, event: E) {
        match self {
            EventSink::Queue(queue) => {
                if queue.push(event).is_err() {
                    record_dropped_event();
                }
            }
            EventSink::Callback(callback) => callback(event),
        }
    }
}

pub fn dropped_events() -> u64 {
    DROPPED_EVENTS.load(Ordering::Relaxed)
}

#[inline]
fn record_dropped_event() {
    let dropped = DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed) + 1;
    if dropped <= 10 || dropped.is_power_of_two() {
        log::warn!("raw shred event queue is full; dropped_event_count={dropped}");
    }
}

/// ALT resolution counters for one client's built-in DEX subscriptions.
/// Counts accumulate across reconnects and subscriptions; cloned clients share them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddressLookupStats {
    pub resolved_transactions: u64,
    /// Resolver errors and invalid resolved address counts/indices.
    pub resolution_failures: u64,
    /// ALT transactions seen without a resolver. Fully static instructions can still emit.
    pub transactions_without_resolver: u64,
}

#[derive(Default)]
struct AddressLookupCounters {
    resolved_transactions: AtomicU64,
    resolution_failures: AtomicU64,
    transactions_without_resolver: AtomicU64,
}

impl AddressLookupCounters {
    fn snapshot(&self) -> AddressLookupStats {
        AddressLookupStats {
            resolved_transactions: self.resolved_transactions.load(Ordering::Relaxed),
            resolution_failures: self.resolution_failures.load(Ordering::Relaxed),
            transactions_without_resolver: self
                .transactions_without_resolver
                .load(Ordering::Relaxed),
        }
    }
}

/// Resolve a transaction's ALT addresses in message lookup order.
///
/// Called synchronously in the receive task, only for V0 transactions with
/// lookups. Use a slot-aware, prewarmed cache; blocking RPC delays reception.
/// An error or invalid address counts/indices skips that transaction. Address
/// identity and lookup-table activation at the supplied slot are the caller's responsibility.
pub type AddressLookupResolver =
    dyn Fn(&VersionedTransaction, u64) -> AnyResult<LoadedAddresses> + Send + Sync;

/// High-level multi-source shred subscription client.
#[derive(Clone)]
pub struct ShredStreamClient {
    config: ShredStreamConfig,
    address_lookup_resolver: Option<Arc<AddressLookupResolver>>,
    address_lookup_counters: Arc<AddressLookupCounters>,
    subscription: Arc<SubscriptionState>,
}

struct SubscriptionState {
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for SubscriptionState {
    fn drop(&mut self) {
        let handle = self
            .handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }
}

impl ShredStreamClient {
    /// Create a client bound to a raw UDP address, for example `0.0.0.0:8001`.
    pub async fn new(udp_bind: impl Into<String>) -> AnyResult<Self> {
        let udp_bind: SocketAddr = udp_bind.into().parse()?;
        Self::new_with_config(ShredStreamConfig::default().with_udp_bind(udp_bind)).await
    }

    pub async fn new_with_decode_mode(decode_mode: ShredDecodeMode) -> AnyResult<Self> {
        Self::new_with_config(ShredStreamConfig::default().with_decode_mode(decode_mode)).await
    }

    pub async fn new_jito_grpc(endpoint: impl Into<String>) -> AnyResult<Self> {
        Self::new_with_config(ShredStreamConfig::jito_grpc(endpoint)).await
    }

    pub async fn new_with_config(config: ShredStreamConfig) -> AnyResult<Self> {
        Ok(Self {
            config,
            address_lookup_resolver: None,
            address_lookup_counters: Arc::new(AddressLookupCounters::default()),
            subscription: Arc::new(SubscriptionState {
                handle: Mutex::new(None),
            }),
        })
    }

    /// Use caller-resolved ALT addresses for built-in DEX subscriptions from
    /// either raw UDP or Jito entries. Configure before subscribing.
    pub fn with_address_lookup_resolver<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&VersionedTransaction, u64) -> AnyResult<LoadedAddresses> + Send + Sync + 'static,
    {
        self.address_lookup_resolver = Some(Arc::new(resolver));
        self
    }

    /// Snapshot ALT resolution for this client's built-in DEX subscriptions.
    pub fn address_lookup_stats(&self) -> AddressLookupStats {
        self.address_lookup_counters.snapshot()
    }

    /// Subscribe to all supported DEX events and return a lock-free queue.
    ///
    /// All supported decode modes feed the same parser:
    /// source -> `Entry` -> `VersionedTransaction` -> `DexEvent`.
    pub async fn subscribe(&self) -> AnyResult<Arc<ArrayQueue<DexEvent>>> {
        self.subscribe_with_filter(None).await
    }

    /// Subscribe to DEX events with parser-side event filtering.
    pub async fn subscribe_with_filter(
        &self,
        event_type_filter: Option<EventTypeFilter>,
    ) -> AnyResult<Arc<ArrayQueue<DexEvent>>> {
        self.stop().await;

        let queue = Arc::new(ArrayQueue::new(self.config.event_queue_capacity.max(1)));
        self.spawn_dex_parser(event_type_filter, EventSink::Queue(Arc::clone(&queue)))
            .await?;
        Ok(queue)
    }

    /// Lowest-latency built-in DEX path: event callback runs in the source
    /// receive task.
    pub async fn subscribe_callback<F>(&self, callback: F) -> AnyResult<()>
    where
        F: Fn(DexEvent) + Send + Sync + 'static,
    {
        self.subscribe_with_filter_callback(None, callback).await
    }

    /// Lowest-latency built-in DEX path with event filtering.
    pub async fn subscribe_with_filter_callback<F>(
        &self,
        event_type_filter: Option<EventTypeFilter>,
        callback: F,
    ) -> AnyResult<()>
    where
        F: Fn(DexEvent) + Send + Sync + 'static,
    {
        self.stop().await;
        self.spawn_dex_parser(event_type_filter, EventSink::Callback(Arc::new(callback)))
            .await
    }

    /// Compatibility convenience for the legacy PumpFun/Bonk parser.
    pub async fn subscribe_pumpfun(
        &self,
        parser_config: PumpfunParserConfig,
    ) -> AnyResult<Arc<ArrayQueue<PumpfunEvent>>> {
        self.subscribe_with_parser(PumpfunEventParser::new(parser_config))
            .await
    }

    /// Lowest-latency built-in PumpFun/Bonk path: event callback runs in the
    /// source receive task.
    pub async fn subscribe_pumpfun_callback<F>(
        &self,
        parser_config: PumpfunParserConfig,
        callback: F,
    ) -> AnyResult<()>
    where
        F: Fn(PumpfunEvent) + Send + Sync + 'static,
    {
        self.subscribe_with_parser_callback(PumpfunEventParser::new(parser_config), callback)
            .await
    }

    /// Subscribe with a custom parser and return a queue of parser events.
    pub async fn subscribe_with_parser<P>(&self, parser: P) -> AnyResult<Arc<ArrayQueue<P::Event>>>
    where
        P: TransactionEventParser + Send + 'static,
        P::Event: Send + 'static,
    {
        self.stop().await;

        let queue = Arc::new(ArrayQueue::new(self.config.event_queue_capacity.max(1)));
        self.spawn_parser(parser, EventSink::Queue(Arc::clone(&queue)))
            .await?;
        Ok(queue)
    }

    /// Subscribe with a custom parser and direct event callback.
    pub async fn subscribe_with_parser_callback<P, F>(
        &self,
        parser: P,
        callback: F,
    ) -> AnyResult<()>
    where
        P: TransactionEventParser + Send + 'static,
        P::Event: Send + 'static,
        F: Fn(P::Event) + Send + Sync + 'static,
    {
        self.stop().await;
        self.spawn_parser(parser, EventSink::Callback(Arc::new(callback)))
            .await
    }

    pub async fn stop(&self) {
        let handle = self
            .subscription
            .handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(handle) = handle {
            handle.abort();
            let _ = handle.await;
        }
    }

    async fn spawn_parser<P>(&self, parser: P, sink: EventSink<P::Event>) -> AnyResult<()>
    where
        P: TransactionEventParser + Send + 'static,
        P::Event: Send + 'static,
    {
        let config = self.config.clone();
        let previous = {
            let mut guard = self
                .subscription
                .handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let handle = tokio::spawn(async move {
                run_with_restarts(config, parser, sink).await;
            });
            guard.replace(handle)
        };
        if let Some(previous) = previous {
            previous.abort();
            let _ = previous.await;
        }
        Ok(())
    }

    async fn spawn_dex_parser(
        &self,
        event_type_filter: Option<EventTypeFilter>,
        sink: EventSink<DexEvent>,
    ) -> AnyResult<()> {
        let config = self.config.clone();
        let resolver = self.address_lookup_resolver.clone();
        let stats = Arc::clone(&self.address_lookup_counters);
        let previous = {
            let mut guard = self
                .subscription
                .handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let handle = tokio::spawn(async move {
                run_dex_with_restarts(config, event_type_filter, sink, resolver, stats).await;
            });
            guard.replace(handle)
        };
        if let Some(previous) = previous {
            previous.abort();
            let _ = previous.await;
        }
        Ok(())
    }
}

async fn run_dex_with_restarts(
    config: ShredStreamConfig,
    event_type_filter: Option<EventTypeFilter>,
    sink: EventSink<DexEvent>,
    resolver: Option<Arc<AddressLookupResolver>>,
    stats: Arc<AddressLookupCounters>,
) {
    let mut restart_attempts = 0u32;

    loop {
        match run_dex_once(
            &config,
            event_type_filter.as_ref(),
            sink.clone(),
            resolver.as_deref(),
            &stats,
        )
        .await
        {
            Ok(()) => {
                restart_attempts = 0;
            }
            Err(error) => {
                if config.max_reconnect_attempts > 0
                    && restart_attempts >= config.max_reconnect_attempts
                {
                    log::error!(
                        "{} dex client stopped after {restart_attempts} restart attempts: {error}",
                        config.decode_mode.name()
                    );
                    return;
                }
                restart_attempts = restart_attempts.saturating_add(1);
                log::error!(
                    "{} dex receive loop failed: {error}; retrying in {}ms",
                    config.decode_mode.name(),
                    config.reconnect_delay_ms
                );
                tokio::time::sleep(Duration::from_millis(config.reconnect_delay_ms)).await;
            }
        }
    }
}

async fn run_dex_once(
    config: &ShredStreamConfig,
    event_type_filter: Option<&EventTypeFilter>,
    sink: EventSink<DexEvent>,
    resolver: Option<&AddressLookupResolver>,
    stats: &AddressLookupCounters,
) -> AnyResult<()> {
    match &config.decode_mode {
        ShredDecodeMode::RawUdp(raw) => {
            run_raw_dex_once(raw.clone(), event_type_filter, sink, resolver, stats).await
        }
        ShredDecodeMode::JitoGrpc(jito) => {
            run_jito_dex_once(jito, event_type_filter, sink, resolver, stats).await
        }
    }
}

async fn run_raw_dex_once(
    raw: RawShredConfig,
    event_type_filter: Option<&EventTypeFilter>,
    sink: EventSink<DexEvent>,
    resolver: Option<&AddressLookupResolver>,
    stats: &AddressLookupCounters,
) -> AnyResult<()> {
    let mut client = RawShredClient::bind(raw)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut events = Vec::with_capacity(4);
    client
        .run_entries(|batch| {
            process_dex_entry_batch(
                batch,
                event_type_filter,
                &sink,
                &mut events,
                resolver,
                stats,
            )
        })
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}

async fn run_jito_dex_once(
    jito: &JitoShredStreamConfig,
    event_type_filter: Option<&EventTypeFilter>,
    sink: EventSink<DexEvent>,
    resolver: Option<&AddressLookupResolver>,
    stats: &AddressLookupCounters,
) -> AnyResult<()> {
    let mut client = connect_jito(jito).await?;
    let request = tonic::Request::new(SubscribeEntriesRequest {});
    let mut stream = client.subscribe_entries(request).await?.into_inner();
    let mut events = Vec::with_capacity(4);

    while let Some(message) = stream.next().await {
        let message = message?;
        let Some(entries) = decode_entries(&message.entries) else {
            log::debug!(
                "jito grpc entry decode failed slot={} bytes_len={}",
                message.slot,
                message.entries.len()
            );
            continue;
        };

        process_dex_entry_batch(
            ShredEntryBatch {
                slot: message.slot,
                entries,
            },
            event_type_filter,
            &sink,
            &mut events,
            resolver,
            stats,
        );
    }

    Err(anyhow::anyhow!("jito gRPC dex stream ended"))
}

#[inline]
fn process_dex_entry_batch(
    batch: ShredEntryBatch,
    event_type_filter: Option<&EventTypeFilter>,
    sink: &EventSink<DexEvent>,
    events: &mut Vec<DexEvent>,
    resolver: Option<&AddressLookupResolver>,
    stats: &AddressLookupCounters,
) {
    let recv_us = now_micros();
    let mut tx_index = 0u64;

    for entry in batch.entries {
        for transaction in &entry.transactions {
            events.clear();
            process_dex_transaction(
                transaction,
                batch.slot,
                tx_index,
                recv_us,
                event_type_filter,
                events,
                sink,
                resolver,
                stats,
            );
            tx_index += 1;
        }
    }
}

#[inline]
fn process_dex_transaction(
    transaction: &VersionedTransaction,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    event_type_filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
    sink: &EventSink<DexEvent>,
    resolver: Option<&AddressLookupResolver>,
    stats: &AddressLookupCounters,
) {
    parse_dex_transaction_events_with_resolver(
        transaction,
        slot,
        tx_index,
        recv_us,
        event_type_filter,
        events,
        resolver,
        stats,
    );

    for event in events.drain(..) {
        sink.deliver(event);
    }
}

#[inline]
fn parse_dex_transaction_events_with_resolver(
    transaction: &VersionedTransaction,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    event_type_filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
    resolver: Option<&AddressLookupResolver>,
    stats: &AddressLookupCounters,
) {
    if transaction.signatures.is_empty() {
        return;
    }

    let signature = transaction.signatures[0];
    if let VersionedMessage::V0(message) = &transaction.message {
        if !message.address_table_lookups.is_empty() {
            if let Some(resolver) = resolver {
                let result = resolver(transaction, slot).and_then(|addresses| {
                    super::dex::parse_transaction_dex_events_with_loaded_addresses(
                        transaction,
                        &addresses.writable,
                        &addresses.readonly,
                        signature,
                        slot,
                        tx_index,
                        recv_us,
                        event_type_filter,
                        events,
                    )
                });
                if let Err(error) = result {
                    stats.resolution_failures.fetch_add(1, Ordering::Relaxed);
                    log::debug!(target: "sol_shred_sdk::shredstream", "skipping unresolved ALT transaction signature={signature} slot={slot}: {error}");
                } else {
                    stats.resolved_transactions.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }

            stats
                .transactions_without_resolver
                .fetch_add(1, Ordering::Relaxed);
            log::trace!(
                target: "sol_shred_sdk::shredstream",
                "V0 tx has no ALT resolver; only instructions with fully resolved static keys will be parsed"
            );
        }
    }

    super::dex::parse_transaction_dex_events_with_filter(
        transaction,
        signature,
        slot,
        tx_index,
        recv_us,
        event_type_filter,
        events,
    );
}

async fn run_with_restarts<P>(config: ShredStreamConfig, mut parser: P, sink: EventSink<P::Event>)
where
    P: TransactionEventParser + Send + 'static,
    P::Event: Send + 'static,
{
    let mut restart_attempts = 0u32;

    loop {
        match run_once(&config, &mut parser, sink.clone()).await {
            Ok(()) => {
                restart_attempts = 0;
            }
            Err(error) => {
                if config.max_reconnect_attempts > 0
                    && restart_attempts >= config.max_reconnect_attempts
                {
                    log::error!(
                        "{} client stopped after {restart_attempts} restart attempts: {error}",
                        config.decode_mode.name()
                    );
                    return;
                }
                restart_attempts = restart_attempts.saturating_add(1);
                log::error!(
                    "{} receive loop failed: {error}; retrying in {}ms",
                    config.decode_mode.name(),
                    config.reconnect_delay_ms
                );
                tokio::time::sleep(Duration::from_millis(config.reconnect_delay_ms)).await;
            }
        }
    }
}

async fn run_once<P>(
    config: &ShredStreamConfig,
    parser: &mut P,
    sink: EventSink<P::Event>,
) -> AnyResult<()>
where
    P: TransactionEventParser + Send + 'static,
    P::Event: Send + 'static,
{
    match &config.decode_mode {
        ShredDecodeMode::RawUdp(raw) => run_raw_once(raw.clone(), parser, sink).await,
        ShredDecodeMode::JitoGrpc(jito) => run_jito_once(jito, parser, sink).await,
    }
}

async fn run_raw_once<P>(
    raw: RawShredConfig,
    parser: &mut P,
    sink: EventSink<P::Event>,
) -> AnyResult<()>
where
    P: TransactionEventParser + Send + 'static,
    P::Event: Send + 'static,
{
    let mut client = RawShredClient::bind(raw)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    client
        .run_entries(|batch| process_entry_batch(batch, parser, &sink))
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}

async fn run_jito_once<P>(
    jito: &JitoShredStreamConfig,
    parser: &mut P,
    sink: EventSink<P::Event>,
) -> AnyResult<()>
where
    P: TransactionEventParser + Send + 'static,
    P::Event: Send + 'static,
{
    let mut client = connect_jito(jito).await?;
    let request = tonic::Request::new(SubscribeEntriesRequest {});
    let mut stream = client.subscribe_entries(request).await?.into_inner();

    while let Some(message) = stream.next().await {
        let message = message?;
        let Some(entries) = decode_entries(&message.entries) else {
            log::debug!(
                "jito grpc entry decode failed slot={} bytes_len={}",
                message.slot,
                message.entries.len()
            );
            continue;
        };

        process_entry_batch(
            ShredEntryBatch {
                slot: message.slot,
                entries,
            },
            parser,
            &sink,
        );
    }

    Err(anyhow::anyhow!("jito gRPC stream ended"))
}

async fn connect_jito(
    config: &JitoShredStreamConfig,
) -> AnyResult<ShredstreamProxyClient<Channel>> {
    let channel = Endpoint::from_shared(config.endpoint.clone())?
        .connect_timeout(JITO_CONNECT_TIMEOUT)
        .tcp_nodelay(true)
        .http2_keep_alive_interval(JITO_KEEP_ALIVE_INTERVAL)
        .keep_alive_timeout(JITO_KEEP_ALIVE_TIMEOUT)
        .keep_alive_while_idle(true)
        .connect()
        .await?;
    Ok(ShredstreamProxyClient::new(channel).max_decoding_message_size(JITO_MAX_MESSAGE_BYTES))
}

#[inline]
fn decode_entries(bytes: &[u8]) -> Option<Vec<Entry>> {
    wincode::deserialize_exact(bytes).ok()
}

#[inline]
fn process_entry_batch<P>(batch: ShredEntryBatch, parser: &mut P, sink: &EventSink<P::Event>)
where
    P: TransactionEventParser,
{
    let recv_us = now_micros();
    let mut tx_index = 0u64;

    for entry in batch.entries {
        for transaction in &entry.transactions {
            if transaction.signatures.is_empty() {
                tx_index += 1;
                continue;
            }

            if let Err(error) = parser.parse_transaction_events(
                transaction,
                batch.slot,
                tx_index,
                recv_us,
                |event| sink.deliver(event),
            ) {
                log::debug!(
                    "raw shred transaction parse failed slot={} tx_index={tx_index}: {error}",
                    batch.slot
                );
            }
            tx_index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::program_ids::RAYDIUM_CPMM_PROGRAM_ID;
    use solana_entry::entry::Entry;
    use solana_sdk::hash::Hash;
    use solana_sdk::message::{
        compiled_instruction::CompiledInstruction, v0, MessageHeader, VersionedMessage,
    };
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::Signature;

    #[test]
    fn jito_entry_decode_rejects_trailing_bytes() {
        let entries = vec![Entry::default()];
        let encoded = wincode::serialize(&entries).expect("serialize entries");
        assert!(decode_entries(&encoded).is_some());

        let mut with_trailing_byte = encoded;
        with_trailing_byte.push(0);
        assert!(decode_entries(&with_trailing_byte).is_none());
    }

    #[test]
    fn dropping_last_client_aborts_subscription_task() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            let client = ShredStreamClient {
                config: ShredStreamConfig::default(),
                address_lookup_resolver: None,
                address_lookup_counters: Arc::new(AddressLookupCounters::default()),
                subscription: Arc::new(SubscriptionState {
                    handle: Mutex::new(None),
                }),
            };
            let task = tokio::spawn(std::future::pending::<()>());
            let abort_handle = task.abort_handle();
            *client
                .subscription
                .handle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(task);

            let clone = client.clone();
            drop(client);
            tokio::task::yield_now().await;
            assert!(!abort_handle.is_finished());

            drop(clone);
            tokio::task::yield_now().await;
            assert!(abort_handle.is_finished());
        });
    }

    fn raydium_cpmm_swap_data() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&crate::instr::raydium_cpmm::discriminators::SWAP_BASE_IN);
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(&90_u64.to_le_bytes());
        data
    }

    fn raydium_cpmm_swap_tx() -> VersionedTransaction {
        VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::V0(v0::Message {
                header: MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 0,
                },
                account_keys: std::iter::once(RAYDIUM_CPMM_PROGRAM_ID)
                    .chain((0..13).map(|_| Pubkey::new_unique()))
                    .collect(),
                recent_blockhash: Hash::default(),
                instructions: vec![CompiledInstruction::new_from_raw_parts(
                    0,
                    raydium_cpmm_swap_data(),
                    (1..14).collect(),
                )],
                address_table_lookups: Vec::new(),
            }),
        }
    }

    fn cpmm_alt_transaction() -> (VersionedTransaction, LoadedAddresses) {
        let accounts: Vec<_> = (0..13).map(|_| Pubkey::new_unique()).collect();
        let addresses = LoadedAddresses {
            writable: accounts[1..].to_vec(),
            readonly: vec![RAYDIUM_CPMM_PROGRAM_ID],
        };
        let tx = VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::V0(v0::Message {
                header: MessageHeader {
                    num_required_signatures: 1,
                    ..Default::default()
                },
                account_keys: vec![accounts[0]],
                recent_blockhash: Hash::new_from_array([9; 32]),
                instructions: vec![CompiledInstruction::new_from_raw_parts(
                    13,
                    raydium_cpmm_swap_data(),
                    (0..13).collect(),
                )],
                address_table_lookups: vec![v0::MessageAddressTableLookup {
                    account_key: Pubkey::new_unique(),
                    writable_indexes: (0..12).collect(),
                    readonly_indexes: vec![12],
                }],
            }),
        };
        (tx, addresses)
    }

    #[test]
    fn dex_subscription_resolves_loaded_program_and_accounts_and_keeps_filter() {
        let (tx, addresses) = cpmm_alt_transaction();
        let expected_pool = addresses.writable[2];
        let expected_input_mint = addresses.writable[9];
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let client = runtime
            .block_on(ShredStreamClient::new_with_config(
                ShredStreamConfig::default(),
            ))
            .unwrap()
            .with_address_lookup_resolver(move |transaction, slot| {
                assert_eq!(slot, 77);
                assert_eq!(transaction.message.static_account_keys().len(), 1);
                seen.fetch_add(1, Ordering::Relaxed);
                Ok(addresses.clone())
            });
        let queue = Arc::new(ArrayQueue::new(4));
        let sink = EventSink::Queue(Arc::clone(&queue));
        let filter = EventTypeFilter::include_only(vec![crate::grpc::EventType::RaydiumCpmmSwap]);
        let batch = ShredEntryBatch {
            slot: 77,
            entries: vec![Entry {
                num_hashes: 1,
                hash: Hash::default(),
                transactions: vec![tx.clone(), raydium_cpmm_swap_tx()],
            }],
        };
        let mut events = vec![];
        process_dex_entry_batch(
            batch,
            Some(&filter),
            &sink,
            &mut events,
            client.address_lookup_resolver.as_deref(),
            &client.address_lookup_counters,
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(client.address_lookup_stats().resolved_transactions, 1);
        let DexEvent::RaydiumCpmmSwap(event) = queue.pop().unwrap() else {
            panic!("swap")
        };
        assert_eq!(event.pool_id, expected_pool);
        assert_eq!(event.input_token_mint, expected_input_mint);
        assert_eq!(
            event.metadata.recent_blockhash.as_deref(),
            Some(tx.message.recent_blockhash().to_string().as_str())
        );
        assert!(event.metadata.grpc_recv_us > 0);
        assert_eq!(queue.pop().unwrap().metadata().tx_index, 1);
        assert!(queue.pop().is_none());
        let excluded = EventTypeFilter::include_only(vec![crate::grpc::EventType::PumpFunBuy]);
        let batch = ShredEntryBatch {
            slot: 77,
            entries: vec![Entry {
                num_hashes: 1,
                hash: Hash::default(),
                transactions: vec![tx],
            }],
        };
        process_dex_entry_batch(
            batch,
            Some(&excluded),
            &sink,
            &mut events,
            client.address_lookup_resolver.as_deref(),
            &client.address_lookup_counters,
        );
        assert!(queue.pop().is_none());
        assert_eq!(
            client.clone().address_lookup_stats(),
            AddressLookupStats {
                resolved_transactions: 2,
                ..Default::default()
            }
        );
    }

    #[test]
    fn missing_alt_resolver_skips_unresolved_instruction_and_counts_transaction() {
        let (tx, _) = cpmm_alt_transaction();
        let queue = Arc::new(ArrayQueue::new(4));
        let sink = EventSink::Queue(Arc::clone(&queue));
        let batch = ShredEntryBatch {
            slot: 77,
            entries: vec![Entry {
                num_hashes: 1,
                hash: Hash::default(),
                transactions: vec![tx, raydium_cpmm_swap_tx()],
            }],
        };
        let stats = AddressLookupCounters::default();
        process_dex_entry_batch(batch, None, &sink, &mut Vec::new(), None, &stats);
        assert_eq!(queue.pop().unwrap().metadata().tx_index, 1);
        assert!(queue.pop().is_none());
        assert_eq!(
            stats.snapshot(),
            AddressLookupStats {
                transactions_without_resolver: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn failed_alt_resolution_skips_transaction_and_continues_batch() {
        for invalid_addresses in [false, true] {
            let (tx, _) = cpmm_alt_transaction();
            let resolver = move |_: &VersionedTransaction, _: u64| -> AnyResult<LoadedAddresses> {
                if invalid_addresses {
                    Ok(LoadedAddresses::default())
                } else {
                    Err(anyhow::anyhow!("cache miss"))
                }
            };
            let queue = Arc::new(ArrayQueue::new(4));
            let sink = EventSink::Queue(Arc::clone(&queue));
            let batch = ShredEntryBatch {
                slot: 77,
                entries: vec![Entry {
                    num_hashes: 1,
                    hash: Hash::default(),
                    transactions: vec![tx, raydium_cpmm_swap_tx()],
                }],
            };
            let mut events = vec![];
            let stats = AddressLookupCounters::default();
            process_dex_entry_batch(batch, None, &sink, &mut events, Some(&resolver), &stats);
            assert_eq!(queue.pop().unwrap().metadata().tx_index, 1);
            assert!(queue.pop().is_none());
            assert!(events.is_empty());
            assert_eq!(
                stats.snapshot(),
                AddressLookupStats {
                    resolution_failures: 1,
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn default_dex_path_delivers_non_pump_events() {
        let queue = Arc::new(ArrayQueue::new(4));
        let sink = EventSink::Queue(Arc::clone(&queue));
        let mut events = Vec::with_capacity(4);
        let batch = ShredEntryBatch {
            slot: 77,
            entries: vec![Entry {
                num_hashes: 1,
                hash: Hash::default(),
                transactions: vec![raydium_cpmm_swap_tx()],
            }],
        };

        process_dex_entry_batch(
            batch,
            None,
            &sink,
            &mut events,
            None,
            &AddressLookupCounters::default(),
        );

        let event = queue.pop().expect("event");
        assert!(matches!(event, DexEvent::RaydiumCpmmSwap(_)));
        assert_eq!(event.metadata().slot, 77);
        assert!(event.metadata().grpc_recv_us > 0);
        assert!(queue.pop().is_none());
    }

    #[test]
    fn dex_filter_is_applied_before_delivery() {
        let queue = Arc::new(ArrayQueue::new(4));
        let sink = EventSink::Queue(Arc::clone(&queue));
        let mut events = Vec::with_capacity(4);
        let batch = ShredEntryBatch {
            slot: 77,
            entries: vec![Entry {
                num_hashes: 1,
                hash: Hash::default(),
                transactions: vec![raydium_cpmm_swap_tx()],
            }],
        };
        let filter = EventTypeFilter::include_only(vec![crate::grpc::EventType::PumpFunBuy]);

        process_dex_entry_batch(
            batch,
            Some(&filter),
            &sink,
            &mut events,
            None,
            &AddressLookupCounters::default(),
        );

        assert!(queue.pop().is_none());
    }
}

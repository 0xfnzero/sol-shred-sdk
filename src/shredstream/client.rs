use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_queue::ArrayQueue;
use futures::StreamExt;
use solana_entry::entry::Entry;
use solana_sdk::message::VersionedMessage;
use solana_sdk::transaction::VersionedTransaction;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

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

/// High-level multi-source shred subscription client.
#[derive(Clone)]
pub struct ShredStreamClient {
    config: ShredStreamConfig,
    subscription_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
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
            subscription_handle: Arc::new(Mutex::new(None)),
        })
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
        if let Some(handle) = self.subscription_handle.lock().await.take() {
            handle.abort();
        }
    }

    async fn spawn_parser<P>(&self, parser: P, sink: EventSink<P::Event>) -> AnyResult<()>
    where
        P: TransactionEventParser + Send + 'static,
        P::Event: Send + 'static,
    {
        let config = self.config.clone();
        let handle = tokio::spawn(async move {
            run_with_restarts(config, parser, sink).await;
        });

        *self.subscription_handle.lock().await = Some(handle);
        Ok(())
    }

    async fn spawn_dex_parser(
        &self,
        event_type_filter: Option<EventTypeFilter>,
        sink: EventSink<DexEvent>,
    ) -> AnyResult<()> {
        let config = self.config.clone();
        let handle = tokio::spawn(async move {
            run_dex_with_restarts(config, event_type_filter, sink).await;
        });

        *self.subscription_handle.lock().await = Some(handle);
        Ok(())
    }
}

async fn run_dex_with_restarts(
    config: ShredStreamConfig,
    event_type_filter: Option<EventTypeFilter>,
    sink: EventSink<DexEvent>,
) {
    let mut attempts = 0u32;

    loop {
        if config.max_reconnect_attempts > 0 && attempts >= config.max_reconnect_attempts {
            log::error!(
                "{} dex client stopped after {attempts} failed restart attempts",
                config.decode_mode.name()
            );
            return;
        }
        attempts += 1;

        match run_dex_once(&config, event_type_filter.as_ref(), sink.clone()).await {
            Ok(()) => {
                attempts = 0;
            }
            Err(error) => {
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
) -> AnyResult<()> {
    match &config.decode_mode {
        ShredDecodeMode::RawUdp(raw) => {
            run_raw_dex_once(raw.clone(), event_type_filter, sink).await
        }
        ShredDecodeMode::JitoGrpc(jito) => run_jito_dex_once(jito, event_type_filter, sink).await,
    }
}

async fn run_raw_dex_once(
    raw: RawShredConfig,
    event_type_filter: Option<&EventTypeFilter>,
    sink: EventSink<DexEvent>,
) -> AnyResult<()> {
    let mut client = RawShredClient::bind(raw)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let mut events = Vec::with_capacity(4);
    client
        .run_entries(|batch| process_dex_entry_batch(batch, event_type_filter, &sink, &mut events))
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}

async fn run_jito_dex_once(
    jito: &JitoShredStreamConfig,
    event_type_filter: Option<&EventTypeFilter>,
    sink: EventSink<DexEvent>,
) -> AnyResult<()> {
    let mut client = ShredstreamProxyClient::connect(jito.endpoint.clone()).await?;
    let request = tonic::Request::new(SubscribeEntriesRequest {});
    let mut stream = client.subscribe_entries(request).await?.into_inner();
    let mut events = Vec::with_capacity(4);

    while let Some(message) = stream.next().await {
        let message = message?;
        let Ok(entries) = bincode::deserialize::<Vec<Entry>>(&message.entries) else {
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
        );
    }

    Ok(())
}

#[inline]
fn process_dex_entry_batch(
    batch: ShredEntryBatch,
    event_type_filter: Option<&EventTypeFilter>,
    sink: &EventSink<DexEvent>,
    events: &mut Vec<DexEvent>,
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
) {
    parse_dex_transaction_events(
        transaction,
        slot,
        tx_index,
        recv_us,
        event_type_filter,
        events,
    );

    for event in events.drain(..) {
        sink.deliver(event);
    }
}

#[inline]
fn parse_dex_transaction_events(
    transaction: &VersionedTransaction,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    event_type_filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
) {
    if transaction.signatures.is_empty() {
        return;
    }

    let signature = transaction.signatures[0];
    if let VersionedMessage::V0(message) = &transaction.message {
        if !message.address_table_lookups.is_empty() {
            log::trace!(
                target: "sol_shred_sdk::shredstream",
                "V0 tx uses address lookup tables; raw shred parser uses static accounts and default placeholders for ALT-loaded accounts"
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
    crate::core::pumpfun_fee_enrich::enrich_pumpfun_same_tx_post_merge(events);

    for event in events.iter_mut() {
        if let Some(metadata) = event.metadata_mut() {
            metadata.grpc_recv_us = recv_us;
        }
    }
}

async fn run_with_restarts<P>(config: ShredStreamConfig, mut parser: P, sink: EventSink<P::Event>)
where
    P: TransactionEventParser + Send + 'static,
    P::Event: Send + 'static,
{
    let mut attempts = 0u32;

    loop {
        if config.max_reconnect_attempts > 0 && attempts >= config.max_reconnect_attempts {
            log::error!(
                "{} client stopped after {attempts} failed restart attempts",
                config.decode_mode.name()
            );
            return;
        }
        attempts += 1;

        match run_once(&config, &mut parser, sink.clone()).await {
            Ok(()) => {
                attempts = 0;
            }
            Err(error) => {
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
    let mut client = ShredstreamProxyClient::connect(jito.endpoint.clone()).await?;
    let request = tonic::Request::new(SubscribeEntriesRequest {});
    let mut stream = client.subscribe_entries(request).await?.into_inner();

    while let Some(message) = stream.next().await {
        let message = message?;
        let Ok(entries) = bincode::deserialize::<Vec<Entry>>(&message.entries) else {
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

    Ok(())
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
                account_keys: vec![RAYDIUM_CPMM_PROGRAM_ID, Pubkey::new_unique()],
                recent_blockhash: Hash::default(),
                instructions: vec![CompiledInstruction::new_from_raw_parts(
                    0,
                    raydium_cpmm_swap_data(),
                    vec![1],
                )],
                address_table_lookups: Vec::new(),
            }),
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

        process_dex_entry_batch(batch, None, &sink, &mut events);

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

        process_dex_entry_batch(batch, Some(&filter), &sink, &mut events);

        assert!(queue.pop().is_none());
    }
}

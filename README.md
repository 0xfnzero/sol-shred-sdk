# Sol Shred SDK

`sol-shred-sdk` is a multi-source Solana shred ingestion and transaction/event parsing SDK.

The default, lowest-latency path is:

```text
UDP packet -> Solana/Agave Shred -> FEC recovery -> deshred -> Entry -> VersionedTransaction -> event parser
```

Source/decode mode is selected with `ShredDecodeMode`. Native raw UDP shreds are the recommended path; Jito-style ShredStream gRPC entries are retained as a compatibility source while Jito service availability lasts.

## What This SDK Provides

| Area | Coverage |
|------|----------|
| Input | Raw UDP Solana shred payloads, or Jito-style ShredStream gRPC entries |
| Decode | `ShredDecodeMode::RawUdp` uses `solana-ledger` `Shred` parsing, Reed-Solomon recovery, `Shredder::deshred`, and bincode `Vec<Entry>` decode; `ShredDecodeMode::JitoGrpc` receives prebuilt entry batches |
| Transactions | Entry-to-transaction flattening with slot context |
| Events | `DexEvent` parser migrated from `sol-parser-sdk` ShredStream handling |
| Extensibility | `TransactionEventParser` trait for custom parser plug-ins |
| Compatibility | Preserves selected generated proto/type re-exports used by older code |

Migrated parser families match the `sol-parser-sdk` parser surface:

- PumpFun, PumpFun v2/Mayhem, Pump fees, PumpSwap
- Raydium LaunchLab, CPMM, CLMM, AMM V4
- Orca Whirlpool
- Meteora Pools, DAMM V2, DBC, DLMM
- Token accounts, nonce accounts, selected DEX account state events, and block metadata types

Raw shred subscriptions parse transaction-visible instruction data directly from
`Entry` transactions. Log-only and account-update event parsers are included for
SDK compatibility, but raw shreds do not carry execution logs or account update
payloads by themselves.

## Installation

```toml
[dependencies]
sol-shred-sdk = "3.0.1"
```

## Decode Mode

Choose the source/decoder when creating the client:

```rust
use sol_shred_sdk::{RawShredConfig, ShredDecodeMode, ShredStreamClient};

let raw = ShredStreamClient::new_with_decode_mode(
    ShredDecodeMode::raw_udp(RawShredConfig::default()),
).await?;

let jito = ShredStreamClient::new_with_decode_mode(
    ShredDecodeMode::jito_grpc("http://127.0.0.1:10000"),
).await?;
```

## Raw UDP Event Subscription

Bind a UDP socket where your Solana shred source sends raw shred datagrams:

```rust
use sol_shred_sdk::shredstream::{ShredStreamClient, ShredStreamConfig};
use sol_shred_sdk::{DexEvent, EventType, EventTypeFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ShredStreamConfig::low_latency()
        .with_udp_bind("0.0.0.0:8001".parse()?);

    let client = ShredStreamClient::new_with_config(config).await?;
    client
        .subscribe_with_filter_callback(
            Some(EventTypeFilter::include_only(vec![
                EventType::PumpFunCreate,
                EventType::RaydiumCpmmSwap,
                EventType::OrcaWhirlpoolSwap,
            ])),
            |event| {
                match event {
                    DexEvent::PumpFunCreate(create) => println!("pump create: {create:?}"),
                    DexEvent::RaydiumCpmmSwap(swap) => println!("cpmm swap: {swap:?}"),
                    DexEvent::OrcaWhirlpoolSwap(swap) => println!("orca swap: {swap:?}"),
                    other => println!("{other:?}"),
                }
            },
        )
        .await?;

    tokio::signal::ctrl_c().await?;
    client.stop().await;
    Ok(())
}
```

## Queue Subscription

For consumers that prefer polling:

```rust
use sol_shred_sdk::shredstream::ShredStreamClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = ShredStreamClient::new("0.0.0.0:8001").await?;
    let queue = client.subscribe().await?;

    loop {
        while let Some(event) = queue.pop() {
            println!("{event:?}");
        }
        tokio::task::yield_now().await;
    }
}
```

For legacy PumpFun/Bonk-only consumers, `subscribe_pumpfun`,
`subscribe_pumpfun_callback`, and the generic `subscribe_with_parser` APIs are
still available.

## Low-Level Decoder

Use the low-level API when you already own the UDP receive loop:

```rust
use std::time::Instant;
use sol_shred_sdk::{RawShredConfig, RawShredDecoder};

fn handle_packet(decoder: &mut RawShredDecoder, packet: &[u8]) {
    for batch in decoder.push_packet(packet, Instant::now()) {
        for entry in batch.entries {
            for transaction in entry.transactions {
                println!("slot={} sig={:?}", batch.slot, transaction.signatures.first());
            }
        }
    }
}

let mut decoder = RawShredDecoder::new(RawShredConfig::default());
```

## Custom Transaction Parser

`sol-shred-sdk` owns networking, shred reassembly, and transaction traversal. Higher-level crates can plug in event parsing without reimplementing shredstream handling:

```rust
use sol_shred_sdk::common::AnyResult;
use sol_shred_sdk::parser::TransactionEventParser;
use solana_sdk::transaction::VersionedTransaction;

struct MyParser;

impl TransactionEventParser for MyParser {
    type Event = String;

    fn parse_transaction_events<F>(
        &mut self,
        transaction: &VersionedTransaction,
        slot: u64,
        tx_index: u64,
        recv_us: i64,
        mut emit: F,
    ) -> AnyResult<usize>
    where
        F: FnMut(Self::Event),
    {
        if let Some(signature) = transaction.signatures.first() {
            emit(format!("slot={slot} tx_index={tx_index} recv_us={recv_us} sig={signature}"));
            return Ok(1);
        }
        Ok(0)
    }
}
```

## Configuration Notes

- `RawShredConfig::udp_payload_prefix_skip` should stay `0` for native raw shreds.
- `RawShredConfig::forward_slot_watermark` defaults to `false` so out-of-order completed slots are not dropped. Enable it only when you explicitly prefer forward-only latency over completeness.
- `ShredStreamConfig::low_latency()` favors shorter reassembly waits and faster restart.
- `ShredStreamConfig::high_throughput()` increases receive buffering, tracked slots, and queue capacity.
- `ShredDecodeMode::JitoGrpc` keeps the old entries-gRPC source but still uses the same unified `DexEvent` parser.
- The UDP receive buffer is requested with `socket2`; the OS may cap the actual value.

## Decoder Benchmark

The raw decoder has an ignored release-mode microbench that generates real Solana merkle shreds with `solana-ledger` and decodes them through `RawShredDecoder`:

```bash
RAW_SHRED_BENCH_ITERS=10000 cargo test --release --lib bench_decode_generated_shreds -- --ignored --nocapture
```

Current local reference result:

```text
packets_per_sec=1172196 slots_per_sec=36631 tx_per_sec=1172196
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

### Telegram group

https://t.me/fnzero_group

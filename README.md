<div align="center">
    <h1>⚡ Sol Shred SDK</h1>
    <h3><em>Low-latency Solana raw shred decoding and multi-DEX event parsing</em></h3>
</div>

<p align="center">
    <strong>A high-performance Rust SDK that turns Solana UDP shreds or Jito-style ShredStream entries into typed DEX events for trading bots, indexers, and real-time analytics.</strong>
</p>

<p align="center">
    <a href="https://crates.io/crates/sol-shred-sdk"><img src="https://img.shields.io/crates/v/sol-shred-sdk.svg" alt="Crates.io"></a>
    <a href="https://docs.rs/sol-shred-sdk"><img src="https://docs.rs/sol-shred-sdk/badge.svg" alt="Documentation"></a>
    <a href="https://github.com/0xfnzero/sol-shred-sdk/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
    <a href="https://github.com/0xfnzero/sol-shred-sdk"><img src="https://img.shields.io/github/stars/0xfnzero/sol-shred-sdk?style=social" alt="GitHub stars"></a>
    <a href="https://github.com/0xfnzero/sol-shred-sdk/network"><img src="https://img.shields.io/github/forks/0xfnzero/sol-shred-sdk?style=social" alt="GitHub forks"></a>
</p>

<p align="center">
    <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
    <img src="https://img.shields.io/badge/Solana-9945FF?style=for-the-badge&logo=solana&logoColor=white" alt="Solana">
    <img src="https://img.shields.io/badge/ShredStream-FF6B6B?style=for-the-badge&logo=lightning&logoColor=white" alt="ShredStream">
    <img src="https://img.shields.io/badge/DEX-4B8BBE?style=for-the-badge&logo=bitcoin&logoColor=white" alt="DEX Events">
</p>

<p align="center">
    <a href="README_CN.md">中文</a> |
    <a href="README.md">English</a> |
    <a href="https://fnzero.dev/">Website</a> |
    <a href="https://t.me/fnzero_group">Telegram</a> |
    <a href="https://discord.gg/vuazbGkqQE">Discord</a>
</p>

`sol-shred-sdk` is a low-latency Rust SDK for Solana raw shred decoding, ShredStream ingestion, and multi-DEX transaction event parsing.

The default, lowest-latency path is:

```text
UDP packet -> Solana/Agave Shred -> FEC recovery -> deshred -> Entry -> VersionedTransaction -> event parser
```

Source/decode mode is selected with `ShredDecodeMode`. Native raw UDP shreds are the recommended path; Jito-style ShredStream gRPC entries are retained as a compatibility source while Jito service availability lasts.

## What This SDK Provides

| Area | Coverage |
|------|----------|
| Input | Raw UDP Solana shred payloads, or Jito-style ShredStream gRPC entries |
| Decode | `ShredDecodeMode::RawUdp` uses `solana-ledger` `Shred` parsing, Reed-Solomon recovery, `Shredder::deshred`, and wincode `Vec<Entry>` decode; `ShredDecodeMode::JitoGrpc` receives prebuilt entry batches |
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

## Use Cases

- Solana low-latency trading bots and sniper bots
- Real-time PumpFun, PumpSwap, Raydium, Orca, and Meteora event feeds
- Raw shred UDP ingestion pipelines without relying on Jito ShredStream
- Jito ShredStream gRPC compatibility while migrating to raw shreds
- Multi-DEX indexers, analytics, and alerting systems
- Higher-level parser SDKs that want to reuse one shred/entry ingestion layer

Raw shred subscriptions parse transaction-visible instruction data directly from
`Entry` transactions. Log-only and account-update event parsers are included for
SDK compatibility, but raw shreds do not carry execution logs or account update
payloads by themselves.

## Installation

### Direct Clone

Clone this project to your project directory:

```bash
cd your_project_root_directory
git clone https://github.com/0xfnzero/sol-shred-sdk
```

Add the dependency to your `Cargo.toml`:

```toml
# Add to your Cargo.toml
sol-shred-sdk = { path = "./sol-shred-sdk", version = "4.0.0" }
```

### Use crates.io

```toml
# Add to your Cargo.toml
[dependencies]
sol-shred-sdk = "4.0.0"
```

## PumpSwap Effective Quote Reserves

PumpSwap Pool accounts and Buy/Sell events expose the appended signed
`virtual_quote_reserves` field. For quoting and indexing, use:

```text
effective_quote_reserves = pool_quote_token_account.amount + virtual_quote_reserves
```

Legacy Pool accounts remain supported and decode this field as `0`. Use checked
signed arithmetic when combining the raw token-account balance (`u64`) with the
virtual reserve (`i128`). The base reserve remains the raw base-vault balance.

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
- Merkle FEC recovery follows Agave's wire layout and Jito's threshold rule: recovery starts only after at least `num_data_shreds` distinct data/coding shards are available.
- Raw UDP decoding does not verify leader signatures because it has no leader schedule. Use a trusted local forwarder, or verify shreds before calling `RawShredDecoder` when accepting untrusted UDP sources.

## Decoder Benchmark

The raw decoder has an ignored release-mode microbench that generates real Solana merkle shreds with `solana-ledger` and decodes them through `RawShredDecoder`:

```bash
RAW_SHRED_BENCH_ITERS=10000 cargo test --release --lib bench_decode_generated_shreds -- --ignored --nocapture
```

Current local reference result:

```text
packets_per_sec=3276089 slots_per_sec=102378 tx_per_sec=3276089
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

### Telegram group

https://t.me/fnzero_group

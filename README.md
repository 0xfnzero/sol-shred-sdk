# Sol Shred SDK

`shred-parsed` is a raw Solana shred ingestion and transaction/event parsing SDK.

The primary path is:

```text
UDP packet -> Solana/Agave Shred -> FEC recovery -> deshred -> Entry -> VersionedTransaction -> event parser
```

This SDK does not implement Jito ShredStream as a supported ingestion path. New integrations should use the raw UDP `shred` and `shredstream` modules.

## What This SDK Provides

| Area | Coverage |
|------|----------|
| Input | Raw UDP Solana shred payloads |
| Decode | `solana-ledger` `Shred` parsing, Reed-Solomon recovery, `Shredder::deshred`, bincode `Vec<Entry>` decode |
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
shred-parsed = "3.0.0"
```

## Raw UDP Event Subscription

Bind a UDP socket where your Solana shred source sends raw shred datagrams:

```rust
use shred_parsed::shredstream::{ShredStreamClient, ShredStreamConfig};
use shred_parsed::{DexEvent, EventType, EventTypeFilter};

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
use shred_parsed::shredstream::ShredStreamClient;

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
use shred_parsed::{RawShredConfig, RawShredDecoder};

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
use shred_parsed::common::AnyResult;
use shred_parsed::parser::TransactionEventParser;
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
- The UDP receive buffer is requested with `socket2`; the OS may cap the actual value.
- Generated proto/type compatibility surfaces are not the recommended ingestion path.

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

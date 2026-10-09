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
- Raydium LaunchLab / StonkFun, CPMM, CLMM, AMM V4
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
sol-shred-sdk = { path = "./sol-shred-sdk", version = "4.0.2" }
```

### Use crates.io

```toml
# Add to your Cargo.toml
[dependencies]
sol-shred-sdk = "4.0.2"
```

PumpFun create/create_v2 account layout and RPC regression details: [PUMPFUN_CREATE_LAYOUT.md](docs/PUMPFUN_CREATE_LAYOUT.md).

## PumpSwap Effective Quote Reserves

Raw shreds contain outer transaction instructions, without execution logs, inner
CPI, Pool state or vault balances. Zero reserve/fee fields in outer Buy/Sell
events mean unavailable data. Quote from a coherent, premaintained account and
fee configuration cache. The instruction's `min_base_amount_out` is a lower
bound, not an executed fill; buy parsing preserves `track_volume` and `ix_name`.

PumpSwap Pool accounts and Buy/Sell events expose the appended signed
`virtual_quote_reserves` field. For quoting and indexing, use:

```text
effective_quote_reserves = pool_quote_token_account.amount + virtual_quote_reserves
```

`virtual_quote_reserves` may be negative (official rollout: September 30).
Preserve its sign in decoding, JSON consumers and indexer storage. Both buys and
sells use the effective quote reserve; the base reserve remains the raw base-vault
balance. Legacy Pool accounts decode the missing field as `0`.

```rust
use sol_shred_sdk::accounts::pumpswap::effective_quote_reserves;

// Raw quote-vault balance 1,000 plus a signed adjustment of -500.
let quote_reserve = effective_quote_reserves(1_000, -500).expect("valid pool state");
assert_eq!(quote_reserve, 500);
// Pass quote_reserve to both buy and sell quote math.
```

The helper adds in `i128` before converting to `u64`. PumpSwap guarantees the
effective reserve fits in `u64`; the helper returns `None` for a negative sum or
a sum above `u64::MAX`. A valid sum does not establish Pool/vault snapshot
consistency; maintain that consistency in the account cache. See the [official negative reserve update](https://github.com/pump-fun/pump-public-docs/blob/main/docs/NEGATIVE_VIRTUAL_QUOTE_RESERVES.md).

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

## Instruction account context

Outer swap parsing fills DLMM, Orca, CLMM, and CPMM instruction account context
and includes the transaction's recent blockhash. DLMM `min_amount_out` is the
instruction threshold, separate from execution output. CLMM quantity mode does
not establish `zero_for_one`; direction stays at its default without execution
data. CLMM `swap`/`swap_v2` expose `ix_name`, `amount`,
`other_amount_threshold`, `sqrt_price_limit_x64` and `is_base_input`.
`sqrt_price_x64` is the executed ending price and stays zero in outer-only
parsing; it no longer contains the instruction price limit. Log merging preserves
wire limits/mode and execution fields independently. The discriminator selects
the legacy or V2 account layout. Built-in parsing and gRPC account backfilling
read the full instruction tail; tick arrays are no longer capped at 16.
`fill_clmm_swap_accounts_with_count` accepts the exact instruction account count;
the getter-only compatibility helper scans up to 256 positions and stops at
the first absent/default address.
For V0 transactions, `parse_transaction_dex_events_with_loaded_addresses`
accepts caller-resolved writable and readonly ALT addresses in message lookup
order. It validates counts and instruction indices before appending events and
performs no RPC requests. The default entry point uses static accounts and skips
instructions with unresolved program or account keys; PumpFun V2 trades also require
the gRPC account layout (27 buy accounts or 26 sell accounts). Other resolved instructions
in the same transaction still parse. `parse_transaction_dex_events_best_effort`
explicitly retains legacy discriminator guesses and missing-account placeholders;
its provisional results do not guarantee gRPC parity.

Obtainable-data parity targets gRPC outer instruction parsing across PumpFun,
PumpSwap, Pump Fees, Raydium AMM V4/CLMM/CPMM/LaunchLab, Meteora
Pools/DAMM V2/DLMM and Orca Whirlpool. Creation-buy and Mayhem flags are
associated with the mint in same-transaction create instructions; PumpSwap
`is_pump_pool` comes from fee-query arguments. Logs, executed inner CPI,
balances and execution results are unavailable in raw shreds. See the local
[parser-parity tool](tools/parser-parity/README.md) for regression coverage and
its limits. CPMM initialize uses pool account 3 and creator account 0;
deposit/withdraw use pool account 2 and owner account 0.
CPMM swaps expose `ix_name`, `amount_in`/`minimum_amount_out` for exact input or
`max_amount_in`/`amount_out` for exact output, plus payer, authority and user token
accounts. These wire arguments remain separate from executed
`input_amount`/`output_amount` and survive gRPC log merging. New fields default
when deserializing older JSON; Rust struct literals need the new fields or
`..Default::default()`.

Built-in UDP/Jito DEX subscriptions can also attach an ALT cache using
`ShredStreamClient::with_address_lookup_resolver`. The callback receives the
transaction and slot and returns `message::v0::LoadedAddresses` in message
lookup order. Transactions without lookups do not invoke it. Resolution errors
or invalid counts/indices skip that transaction while reception continues. The
callback runs synchronously and should read a prewarmed cache; the caller owns
address identity and lookup-table state at the supplied slot.
`client.address_lookup_stats()` reports successful resolutions, resolution failures,
and ALT transactions received without a resolver. Counts are cumulative and shared
by client clones. Without a resolver, only instructions with fully resolved static
keys are emitted.

Orca Whirlpool liquidity instructions use pool account 0 and position account 3
(legacy) or 5 (V2). Initialization reads config/mints at 0/1/2 and pool at 4
(legacy) or 6 (V2); legacy arguments begin with a one-byte bump. Both
initialization versions and V2 liquidity instructions are recognized by their IDL
discriminators. Legacy initialization uses token program 8 for both mints; V2
uses programs 10 and 11.

Orca swap events expose `ix_name`, `amount`, `other_amount_threshold`,
`sqrt_price_limit`, and `amount_specified_is_input` as wire parameters, plus
`token_authority` and both `token_owner_account_*` addresses. Outer-only parsing
leaves executed amounts and pre/post prices at zero; thresholds and price limits
are not execution results. Both gRPC merge paths preserve these parameters while
keeping log amounts, prices, direction, and fees authoritative. Legacy/V2 account
layouts follow the instruction discriminator. Added fields default in old JSON
and do not change the Borsh event layout; struct literals can use
`..Default::default()`.

Meteora Pools swap events now separate `amount_in` and `minimum_out_amount`
from executed `in_amount`/`out_amount`. Outer-only parsing leaves execution
amounts and fees at zero and exposes all 15 IDL accounts, including `pool`,
user token accounts, vaults, LP accounts, and `protocol_token_fee`. Both gRPC
merge paths preserve instruction parameters and fill missing account fields
while keeping log execution values. Added fields default in old JSON; Rust
struct literals can use `..Default::default()`.

Meteora Pools `add_balance_liquidity`, `add_imbalance_liquidity`, and
`remove_balance_liquidity` use their current IDL discriminators. Liquidity
events expose `ix_name`, all 16 instruction accounts, and dedicated wire
parameters: LP amount and token maxima for balanced deposits, minimum LP
amount and token inputs for imbalanced deposits, and LP amount plus token
minima for balanced withdrawals. Outer-only execution quantities remain zero;
both gRPC merge paths preserve parameters and missing accounts while keeping
log execution quantities. Old JSON defaults added fields; Rust struct literals
can use `..Default::default()`. The previous unrelated liquidity discriminators
are no longer recognized.

Meteora Pools also parses `remove_liquidity_single_side` and
`bootstrap_liquidity`. Single-side removal retains `pool_token_amount`,
`minimum_out_amount`, and its 15-account layout with `user_destination_token`;
it leaves per-side execution amounts unknown. Bootstrap retains both token
inputs and all 16 accounts while leaving executed LP/token quantities at zero.
Both gRPC merge paths preserve wire parameters. Account dispatch selects these
operations by discriminator and known pool rather than account count. New fields
default in old JSON; struct literals can use `..Default::default()`.

Meteora Pools constant-product initialization with config/config2 uses current
IDL discriminators and account indices: pool/config/LP mint/token mints at
0/1/2/3/4. Events expose both token inputs, optional config2 activation point,
and all 26 accounts. `set_pool_fees` exposes the four fee ratios, partner fee
numerator, pool, and fee operator. Protocol fee fields use IDL names; historical
`owner_trade_fee_*` fields remain aliases. Logs populate both names, and gRPC
merges preserve logged fee ratios plus instruction-only partner/context fields.
Management account dispatch matches the operation and known pool. The obsolete
`initialize_pool` discriminator is no longer accepted; `CREATE_POOL` now refers
to config initialization. New fields default in old JSON.

All six initialization variants in the current Meteora Pools IDL are parsed.
Permissioned and permissionless initialization retain the full Stable curve
parameters in `stable_curve`, including token multipliers and depeg state;
fee-tier initialization retains optional `trade_fee_bps`. Customizable creation
retains `customizable_params`, including fee numerator, activation settings,
alpha-vault flag, and all 90 padding bytes. Account filling selects each
24/25/26-account layout by instruction name, preserving distinct admin/payer
and fee-owner roles. gRPC merges preserve these parameters, including zero and
absent optional values. Invalid curve/depeg enums, option tags, bool bytes, and
truncated argument data are rejected. Added fields default in old JSON.

Raydium AMM V4 legacy/V2 exact-input and exact-output swaps now expose
`ix_name`, `instruction_amount_in`, and `instruction_amount_out`. Outer-only
`amount_in`/`amount_out` remain zero until execution data is available;
`minimum_amount_out`/`max_amount_in` retain wire limits. Both gRPC merge paths
keep these parameters, including zero values, fill missing account context,
and retain logged execution amounts. New fields default in old JSON and are
skipped in Borsh, preserving the program event wire layout. Rust struct literals
can use `..Default::default()`.

The CPMM creator-fee protocol-share upgrade is supported: collection events
require the appended PDA/config accounts, AmmConfig exposes
`creator_fee_share_rate` without changing its 236-byte wire size, and the new
CreatorFeeShare account is decoded. `cpmm_creator_fee` provides canonical PDA
derivation, an idempotent upgrade helper for existing collection Instructions,
and exact integer payout splits. Swap, quote, and LP paths are unchanged.
See the [migration guide](docs/cpmm-creator-fee-upgrade.md) for account indices,
API examples, and collection-time rounding.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

### Telegram group

https://t.me/fnzero_group

### Real mainnet RPC examples

See [examples](examples/README.md) for live RPC fetching, offline replay of eleven captured transactions, and wire/token-balance validation.


## Pump upgrade (October 2026)

Compact Pump v3 and PumpSwap v2 trades use their new 17-account layouts. New typed events cover `PumpFunPostCompleteBuy`, `PumpFunComplete`, `PumpFunSweepBondingCurveFee` and `PumpSwapSweepPoolFee`, with program-scoped log and CPI parsing. Historical SOL CompleteEvent payloads remain supported. Curve/pool retained fees and synthetic counters are exposed; optional historical tails default to zero.

For a synthetic completing buy, retain TradeEvent **and** PostCompleteBuyEvent and aggregate execution amounts within the same invocation. CompleteEvent is the completion notification. For `multi_hop_swap`, retain each venue's trade events; different venues are not merged into one fill. The multi-hop intent decoder exposes the fixed user accounts, input/minimum limits and 5 roles per hop; these limits are not actual executed amounts. Streamer forwards the typed events and account fields through its parser bridge (its re-exported `parser_sdk` also provides the intent decoder).

Reference: [pump-public-docs](https://github.com/pump-fun/pump-public-docs/tree/8cda1fa30ea658b20909d8aedf002047119388d2). Validation uses offline official IDL fixtures; no live trade is sent by the tests.

# Changelog

## v6.0.0 — 2026-10-22

Synchronize SDK code, examples, version navigation and AI release rules. Read the [release and migration notes](RELEASE_NOTES.md) for callback removals, V3 defaults and the Go /v9 migration.

Release and integration notes preserved from the previous README. APIs, dependencies and validation results describe those releases, not the current contract. See the [README](README.md) and [release and migration notes](RELEASE_NOTES.md) for current usage.

## v5.0.0

See [release and migration notes](RELEASE_NOTES.md).

## v4.0.9 — Shred metadata passthrough and ordering scope

Preserve and verify transaction position and receive time through raw instruction callbacks. Entries indexes are message-local, unlike Yellowstone node indexes; use signatures for cross-message deduplication. This release clarifies API documentation and adds regression protection without changing metadata fields or numbering behavior.

## v4.0.9 — DEX-scoped RawInstruction and typed DEX identifiers

Raw instructions now respect DEX subscriptions using programs referenced by each instruction. `RawInstructionDexType` provides typed identifiers with unchanged JSON strings. Unknown router layouts do not establish pool addresses, trade direction, or method versions.

## v4.0.9 — raw Shred instructions

The compatibility `PumpfunEventParser` and `ShredStreamGrpc` callbacks now forward `PumpfunEvent::RawInstruction`. Standalone trades are no longer dropped or merged into unrelated creates. The log-only compatibility parser preserves nested Pump legs and discards events from failed parent invocations. ShredStream gRPC transports pre-execution transactions, not Yellowstone execution metadata.

Run `cargo run --example pump_raw_replay` for a freshly captured Delta transaction (2026-10-19, slot 458542546). It emits the visible raw instruction accounts/data without inferring an internal buy/sell. The shared decoder now also accepts current Anchor event CPI prefixes; Shred itself does not contain execution metadata.

Default Shred parsing emits `RawInstruction` for unsupported third-party programs, including Rapid/Delta, without inferring direction, amounts, or internal buy methods. Infrastructure programs (System, Token, ATA, ComputeBudget, etc.) are omitted by default. Supported official DEX parsers are unchanged; official Pump/PumpSwap multi-hop instructions retain `PumpRouteIntent`.

The payload contains `program_id`, `instruction_index`, `accounts`, `account_indices`, `instruction_data`, and transaction metadata. Account order and duplicates are preserved. Unresolved ALT addresses are null/None, with original indices retained; supply cached loaded addresses or a resolver. No hot-path RPC is performed. Data is copied verbatim without requiring a known layout.

Run `cargo run --example raw_instruction_filter` to compare PumpFun-only, Raydium CPMM-only, and all-raw subscriptions against signed offline transactions; the output includes `dexes`.

The raw event retains `metadata.tx_index` and `metadata.grpc_recv_us` from the parser arguments (streamer names the receive field `metadata.recv_us`). The default Entries client index is local to one received Entries message and restarts at zero for the next message; it is not a confirmed slot-wide transaction index. `instruction_index` orders outer instructions within a transaction. Receive time is a local microsecond timestamp, not an execution-order guarantee. Standalone replay APIs may default these values to zero; pass the upstream values when available.

Without a filter, unknown third-party instructions are returned. Broad trade/swap filters (for example `PumpFunTrade` or `RaydiumCpmmSwap`) also return raw instructions associated with that DEX; direction-specific filters do not infer a buy or sell. Explicit `RawInstruction` selects all outer instructions only when no protocol restriction is set; streamer protocol restrictions always apply. Excluding `RawInstruction` wins. `dex_type` uses the public `RawInstructionDexType` enum (typed string constants in Go); JSON values remain unchanged, such as `"pumpfun"` and `"raydium_cpmm"`. `dexes` contains `{ dex_type, program_id, pool }` for each recognized DEX program targeted or referenced by this instruction, not other instructions in the transaction. Multi-DEX wrappers retain every association. Unknown or unresolved ALT references cannot satisfy a DEX subscription. `pool` is currently null: an unknown wrapper layout cannot establish the pool account. A program reference is not proof of an executed CPI. Raw callbacks are not fills.

Wrapper adapter functions remain explicit compatibility helpers only; default Shred parsing does not call them. The released-version history below describes previous behavior.

## v4.0.6 — Pump route event support

Route intents expose `is_buy` and `ix_name`, using existing trade field names and official IDL names. RapidBuy reports a buy intent and `buy_exact_quote_in_v2`; BuyPumpDelta reports only a buy intent. Multi-hop direction and unknown methods remain null. Currencies remain in `hops[].base_mint` / `quote_mint`. These hints do not confirm execution; filtering does not convert intents into trades.

Streamer forwards complete route payloads as `DexEvent::PumpRouteIntentEvent` with protocol/event filtering. Old JSON defaults missing hints to unknown. Rust struct literals need the new fields; exhaustive streamer event matches need the new variant.

## v4.0.5 — Pump routes and offline replay

Adds native Pump route-intent parsing for official multi-hop instructions and the exact RapidBuy/BuyPumpDelta wrapper layouts captured in reported transactions. Adds locally cached ALT resolution and rejects unresolved account identities by default, with explicit best-effort compatibility APIs where supported. Route intents expose instruction constraints and hop identities; confirmed execution amounts require execution metadata.

Includes the latest complete implementation, signed transaction fixtures, native offline-bank regression examples and reproducible local hot-path benchmarks. Unknown wrapper layouts are not inferred from embedded bytes. Local tests do not measure production network latency.

## v4.0.4 — Pump depth-2 compatibility

Adds signed Pump depth-2 create-event replay coverage, including ALT-loaded accounts and executed logs. Existing event layouts and public parsing interfaces remain unchanged.

Validation includes official Pump SDK 4.1.0 instruction comparisons, signed local LiteSVM simulations and SDK regressions. Local simulation state/config is synthetic; no funded transactions were broadcast and production latency was not measured.

## v4.0.3 — Signed transaction and hot-path hardening

Hardens signed shred/FEC replay, rejection recovery, loaded-address and wire boundaries. Aligns DEX instruction/account layouts, compact Pump/PumpSwap events, CPMM creator fees, route attribution and liquidity/reward parsing. Malformed protobuf program keys remain unknown rather than being classified as SystemProgram. Adds signed transaction and offline bank regression fixtures.

Validation includes local CPU benchmarks and offline signed-bank scenarios. Measured hot paths use cached inputs without RPC. Benchmarks do not establish production network or transaction-landing latency. No funded mainnet transactions were broadcast.

## Pump routes in ShredStream (offline, before execution)

The DEX shred parser recognizes official `multi_hop_swap` and
`multi_hop_curve_swap`, plus the captured RapidBuy (`rapidV5DA…`, 41-byte layout)
and BuyPumpDelta (`J7pourVw…`, 28-byte layout) wrapper versions. It emits
`DexEvent::PumpRouteIntent`, with the original instruction index, user, input and
output accounts, route hops and original instruction data. A two-hop route is
**one route event with two hops**; three separate wrapper calls produce three
route events. Unknown programs or changed layouts are not guessed by scanning
for embedded Pump bytes.

**These events are intents, not confirmed fills.** They contain no execution
status, actual fill amounts, reserves or post balances. `amount_in` and
`min_amount_out` are explicit instruction constraints where decoded; BuyPumpDelta
chooses its input at runtime, so these fields are `None`. Failed or conditional
execution cannot be determined from raw shreds. Use execution metadata for fills.

Subscribe with `EventType::PumpRouteIntent`. Broad `PumpFunTrade` subscriptions
also receive route intents; official `MultiHopSwap` routes can contain both Pump
families and also match `PumpSwapTrade`. Direction-specific buy/sell and create
filters do not match route intents. `exclude_types([PumpRouteIntent])` suppresses
all intents. Consumers must handle the new `DexEvent::PumpRouteIntent` variant
rather than assuming every callback is a trade variant.

`filter: None` already includes route intents; changing the filter does not
convert them into `PumpFunTrade`. For RapidBuy and BuyPumpDelta, `is_buy` is
`Some(true)`, relative to `hops[0].base_mint`; the payment mint is
`hops[0].quote_mint`. RapidBuy also exposes
`ix_name = Some("buy_exact_quote_in_v2")` from its validated embedded
instruction. BuyPumpDelta's expected method is unknown (`None`). Both hints
are `None` for multi-hop routes. These are pre-execution hints, not proof of
a successful CPI. Older serialized events deserialize with unknown hints.

```rust,ignore
if let DexEvent::PumpRouteIntent(intent) = event {
    println!("buy intent: {:?}, expected method: {:?}",
        intent.is_buy, intent.ix_name);
    for hop in &intent.hops {
        println!("base: {}, quote: {}", hop.base_mint, hop.quote_mint);
    }
}
```

The new optional fields preserve JSON read compatibility, but downstream Rust
code constructing `PumpRouteIntentEvent` literals must supply the two fields.

ALT addresses must come from a prewarmed, slot-aware local cache. Both SDKs expose
`shredstream::parse_transaction_dex_events_with_loaded_addresses`: pass all loaded
writable addresses, then all loaded readonly addresses, in the message lookup
order. Count/index errors return before appending events. The caller must ensure
address identity and lookup-table activation at the transaction slot. The default
parser skips unresolved instruction accounts instead of emitting placeholder
pubkeys. `parse_transaction_dex_events_best_effort` retains the older provisional
behavior for direct instructions; route adapters always require resolved keys.

High-level clients accept `with_address_lookup_resolver(|transaction, slot| ...)`,
returning `LoadedAddresses` from your cache. Cache errors skip the transaction;
without a resolver, fully static instructions can still parse. Do not perform RPC
or blocking I/O in this synchronous callback. The SDK parsing path performs no
RPC, simulation, or transaction submission.

Regression coverage includes the six original signed mainnet transaction wires,
ALT/filter/error cases, signed static official routes, and malformed/truncated
instructions. Run `cargo test --test pump_shred_routes`. The tests are offline;
synthetic signing validates extraction and does not simulate program execution.

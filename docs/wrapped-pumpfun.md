# Wrapped Pump.fun transactions

## Callback contract

Third-party input is opaque. The SDK does not infer wrapper ABIs, reorder wrapper
accounts or reconstruct a Pump buy/sell instruction. Consumers interpret the
original input in the trade event's `third_program`.

On Shred, an unknown outer program referencing Pump.fun in **that instruction's**
accounts produces a `PumpFunTrade` candidate with `parse_status=Unresolved`.
This does not establish a trade, direction, mint, pool or amount: the program may
only prepare accounts. Missing trade fields retain defaults, not proven zero fills.
Broad PumpFunTrade subscriptions receive candidates; buy/sell-only subscriptions
do not. Other unknown instructions are skipped, without scanning all DEX families.

On gRPC/RPC, supplied execution metadata and actual Pump CPI instructions determine
the trade. Its exact originating outer instruction fills `third_program`.
Direct calls have no third-party payload. A successful invocation can be
`ExecutionConfirmed` even when some execution amounts are unavailable.

One outer instruction can invoke zero, one or several trades. Compare signature
and outer instruction index rather than expecting equal Shred/gRPC event counts.

## Original input and metadata

`third_program: Option<RawInstructionPayload>` contains:

- `program_id`: original outer program.
- `instruction_index`: outer instruction position.
- `accounts`: original ordered addresses, including duplicates and unresolved `None` entries.
- `account_indices`: original indices into the transaction account table.
- `instruction_data`: unchanged instruction bytes.

Transaction metadata stays on the enclosing event, including `tx_index`,
`grpc_recv_us` and `recent_blockhash`. A local Shred index is not confirmed
slot-wide execution order. Cached loaded addresses may be supplied for ALT
resolution; the parsing hot path never calls RPC.

## Removed interfaces

`DexEvent::PumpRouteIntent`, `DexEvent::RawInstruction`, their subscription
variants and their route/raw parsers have been removed. Remove those subscriptions
and match arms. Subscribe to PumpFunTrade and inspect `third_program` and
`parse_status` instead. There is no separate raw callback or built-in wrapper-rule
engine. Official multi-hop instructions no longer emit route-intent callbacks;
their executed trade legs remain available through gRPC/RPC.

Other existing native DEX parsers and gRPC trade provenance remain unchanged.
The stock-price configuration's `RawInstructionDexType` identifiers remain for
compatibility; they do not cause raw-instruction parsing or hot-path DEX scans.

## Verification

Run `cargo run --example pump_raw_replay` from the repository. Saved captures are offline evidence, not current mainnet simulation. This SDK decodes outer instructions only. Use sol-parser-sdk for execution-aware gRPC replay.

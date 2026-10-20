# Wrapped Pump.fun instructions

The Shred parsing hot path never calls RPC. It sees signed outer instructions and locally resolved ALT addresses, not executed CPI instructions. Missing ALT addresses remain null/None; account order and duplicates are retained.

Unknown third-party instructions referencing Pump.fun in their own accounts produce a PumpFunTrade candidate with parse_status=Unresolved and third_program containing program_id, instruction_index, accounts, account_indices and instruction_data. This is not proof of a trade, direction or amount. Broad PumpFunTrade subscriptions receive candidates; buy/sell-only filters do not. Metadata retains upstream tx_index and receive time; a locally assigned Shred index is not confirmed slot order.

Explicit RawInstruction subscriptions remain available for analysis. Non-Pump raw instructions retain DEX-scoped filtering. Native instruction events use InstructionDecoded. This SDK's Shred path cannot establish ExecutionConfirmed; use sol-parser-sdk's gRPC execution parser for that evidence.

Reconstruction is opt-in: load reviewed WrappedPumpfunRule values once and apply parse_wrapped_pumpfun_instruction to raw instructions. A candidate payload can be adapted using with_metadata. Exact program, data length, guarded bytes, account count and canonical accounts must match. Restored events retain the original payload, use official IDL instruction names, and leave executed amounts unknown. Program upgrades can invalidate rules without changing their byte shape.

Run `cargo run --example wrapped_pumpfun_replay` for offline signed-wire replay and exact CPI comparison. This example explicitly requests raw instructions; saved execution metadata is used only as a test oracle. Run `cargo test --test raw_instructions` for candidate defaults, filters, account order, missing ALT keys and metadata.

The analyzer in sol-parser-sdk (`cargo run --example wrapped_pumpfun_rpc_analyzer`) fetches signatures only when explicitly requested. It emits candidate rules, never installs them. Runtime-derived or ambiguous inputs are rejected. The reported J7 transaction cannot reconstruct its quote input; Rapid buys in the saved corpus have guarded opt-in rules, while unmatched layouts remain unresolved. See the fixtures for exact signatures and captured slots; these are historical regression captures.

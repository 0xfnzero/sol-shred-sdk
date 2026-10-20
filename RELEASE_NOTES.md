# sol-shred-sdk v5.0.0

Shred wrapper candidates, lossless original instructions, and explicit parsing evidence.

## Changes

- Pump-referencing unknown outer instructions produce PumpFunTrade candidates with parse_status=Unresolved and third_program containing original program_id, instruction_index, ordered accounts, account_indices and instruction_data.
- Direction-specific subscriptions exclude unresolved candidates. Explicit RawInstruction subscriptions remain available; other DEX raw filtering remains instruction-local.
- Preserve transaction index and receive time. No RPC is added to the parsing hot path.
- Organize examples and documentation; tests use the SDK's own language. Offline captured transactions do not represent fresh mainnet execution or production latency measurements.

## Migration

third_program now contains RawInstructionPayload rather than a rule identifier. It does not contain duplicated metadata/dexes; metadata remains on the event. Check parse_status: Unresolved is only a possible DEX interaction, InstructionDecoded establishes input decoding, and ExecutionConfirmed reflects supplied execution evidence, not necessarily known fill amounts. Do not treat zero defaults as proven zero fills. Rust callers constructing trade structs must initialize the new fields or use ..Default::default().

No RPC is performed by the parsing hot path. Explicit analysis/discovery tools can fetch RPC data outside that path. Wrapper reconstruction remains opt-in and is not proof of future third-party program behavior.

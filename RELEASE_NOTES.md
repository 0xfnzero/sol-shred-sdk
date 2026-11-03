# sol-shred-sdk v7.0.0

Released November 3, 2026.

## Changes

Remove parse-status types and fields; preserve wrapper provenance. Add ShredStream x-token authentication and expanded ALT resolver context. Preserve Pump.fun creation transaction versions and ALT account keys. Synchronize native replay examples and downstream API semantics. No RPC is added to parsing.

## Migration

Remove TradeParseStatus / PumpFunParseStatus / parse_status usage. Update custom resolver callbacks and initialize new event/config fields; use constructors or Default where available. This release changes public Rust APIs and therefore increments the major version.

## Validation

Library regressions, captured creation transactions and multi-hop parsing tests passed. Live provider subscriptions were not exercised.

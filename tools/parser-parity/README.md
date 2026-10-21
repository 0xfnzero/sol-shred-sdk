# Parser comparison

This developer diagnostic compares sol-parser-sdk's outer-instruction path with
sol-shred-sdk. It must not use execution-aware gRPC as the oracle: execution
confirmation, actual fills and unresolved wrapper candidates have different
contracts. The adapter supplies the same resolved account table without RPC.

Run from the sol-shred-sdk repository:

```sh
CARGO_TARGET_DIR=target cargo run --manifest-path tools/parser-parity/Cargo.toml
```

On macOS, building RocksDB may require
`DYLD_LIBRARY_PATH=/Library/Developer/CommandLineTools/usr/lib`.

This is currently a diagnostic, not a passing all-protocol equivalence gate.
The 2026-10-22 run compared 48,325 generated transaction/filter scenarios and
14 saved transactions. It reported 4,181 differing cases, including repeated
filter permutations. Existing differences include Pump creation account fields,
legacy track_volume, CLMM personal_position fields, and strict rejection of
short/malformed instruction bodies. These are not 4,181 independent bugs.
Do not mask fields or relax strict input validation merely to obtain equality.

Focused V3/third_program regressions and native SDK tests are separate checks;
their passing results do not establish whole-parser equivalence or live network
performance.

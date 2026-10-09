# Shred / gRPC instruction parity

Run from the `sol-shred-sdk` repository root with sibling checkouts of
`sol-parser-sdk` and `solana-program-idls`:

```sh
CARGO_TARGET_DIR=target cargo run --manifest-path tools/parser-parity/Cargo.toml --quiet
```

On macOS, if bindgen cannot locate the Command Line Tools libclang:

```sh
DYLD_LIBRARY_PATH=/Library/Developer/CommandLineTools/usr/lib \
LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib \
CARGO_TARGET_DIR=target \
cargo run --manifest-path tools/parser-parity/Cargo.toml --quiet
```

An optional first argument changes the IDL directory. The tool uses the peer
SDK's local `solana-keypair` patch and requires no RPC or credentials.

The comparator generates IDL argument encodings with independent argument values, zero/nonzero values and
`u64::MAX` / `u128::MAX`
for PumpFun, PumpSwap, Pump Fees, Raydium AMM V4/CLMM/CPMM/LaunchLab,
Meteora Pools/DAMM V2/DLMM and Orca Whirlpool. It compares the public shred
transaction parser with the peer's gRPC instruction parser using execution
metadata stripped away, and exits unsuccessfully for any mismatch.

Checks include static accounts, caller-resolved ALT accounts (including loaded
program IDs), unresolved program/account indices, incomplete create context,
event filters and trade normalization, independent invocations in
one transaction, same-pool calls by different users, mixed instruction layouts,
all 104 shared event-filter variants in include/exclude modes, optional accounts
and remaining-account extensions, every data/account prefix of supported inputs, and PumpFun creation plus buys for two distinct mints. Creation
context must survive filters and must not leak to unrelated buys. PumpSwap fee
queries deliberately vary the bool independently of the market-cap byte.
CPMM swap arguments and payer/authority/user token accounts additionally have
independent assertions against wire bytes and IDL positions; execution amounts
must remain unset in outer-only parsing. CLMM has independent assertions for
wire amount, threshold, 128-bit price limit and quantity mode; executed amounts,
ending price and direction remain unset. Remaining-account cases extend to
24 and 64 appended keys, with independent assertions for complete CLMM tick
array ordering and resolved-ALT equivalence for long tails. The tool uses arbitrary-precision JSON
numbers to compare full-width 128-bit values without rounding.
Unsupported IDL instructions are also checked for unexpected shred-only events;
IDL coverage does not imply that every IDL instruction emits an SDK event.
Orca's legacy/V2 initialization and liquidity instructions must emit their SDK
events; dedicated assertions reject shared missing discriminator support and
verify pool/position/config/mint/program indices and the legacy bump offset.
Orca swap assertions independently check wire amounts, thresholds, price limits,
direction, user account indices, and zero execution fields in outer-only events.
Meteora Pools swap assertions check both wire parameters, all 15 IDL accounts,
and zero execution amounts/fees without relying on the peer parser.
Balanced/imbalanced deposit, balanced/single-side withdrawal, and bootstrap
instructions must emit
events; independent checks validate wire limits, all 15/16 accounts, and zero
outer-only execution quantities against the IDL.
Config/config2 constant-product initialization and `set_pool_fees` must emit
events; independent assertions check all 26 creation accounts, token inputs,
optional activation point, fee ratios, partner numerator, and fee operator.
All other initialization variants must emit events, with independent checks
for their 24/25/26-account layouts, Stable curve parameters, optional fee tier,
and customizable settings including all 90 padding bytes.
AMM V4 legacy/V2 swap assertions independently check the specified instruction
amounts and limits while requiring zero outer-only execution amounts.
CPMM creator collection instructions must emit events with the upgraded 15/16
account lists; independent checks verify the appended PDA/config indices and
all original account positions.

Only execution-derived `sol_balance` / `token_balance` fields are excluded from
JSON comparison. Receive timestamps are compared exactly, including a nonzero
caller-supplied timestamp. No instruction, account or metadata fields are excluded.

The tool also replays ten saved mainnet RPC/Yellowstone fixtures with resolved
ALTs. These fixtures currently produce zero supported outer DEX events after
execution metadata is removed, so they validate the empty/router path only.
Positive outer DEX coverage comes from the generated transactions. Executed
inner CPI instructions, logs, execution status, fees charged and post-execution
balances cannot be recovered from raw shreds and are outside this comparison.

As of this revision: 392 supported instruction inputs, 46,464 generated
transaction/filter scenarios, and 1,208 unsupported IDL inputs, with no
mismatches. This finite suite checks parser behavior, not whether constructed
transactions would successfully execute on-chain.

Run the same comparison with both SDKs' zero-copy parsers enabled by adding
`--features parse-zero-copy` to the run command.

The peer SDK and IDLs are local dependencies; changing either can legitimately
change the result. This is a local regression tool and adds no CI workflow.

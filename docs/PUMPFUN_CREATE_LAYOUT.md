# Pump create / create_v2 account decoding

Checked against the official Pump IDL and `@pump-fun/pump-sdk@2.0.0` on 2026-10-07.

`create` and `create_v2` have different fixed account lists. In particular,
`user` / `token_program` are at 7 / 9 for `create`, and 5 / 7 for `create_v2`.
The public IDL defines 16 fixed `create_v2` accounts and does **not** define its
quote remaining accounts. The official SDK's `createV2QuoteRemainingAccounts`
appends quote mint, bonding-curve quote ATA, quote token program and quote control
after that list. Historical mainnet transactions also use the first three quote
accounts without quote control. Further remaining accounts do not shift them.

The SDK now generates fixed indices and discriminators by account name from
[the pinned layout snapshot](../tests/fixtures/pump_create_layout.json).
RPC instruction parsing, shred parsing and account filling share the quote-tail
decoder. Parsing uses compiled constants and three account reads: no runtime
IDL download, JSON parsing, RPC lookup or PDA derivation is added.

No quote tail means native SOL. A partial, unresolved or invalid quote tail means
unknown (`Pubkey::default()`), never an assumed SOL pair. A complete tail requires
a nonzero mint/vault and a supported SPL Token or Token-2022 program. Authoritative
event fields are preserved by account filling; merging unknown fields does not
convert them into SOL. Raw shreds do not contain execution logs or loaded ALT
addresses. Provide resolved ALT keys for reliable quote classification. The RPC replay
tests expand the effective key list in a test-only adapter before invoking the
released shred transaction parser; this does not validate raw shred/FEC decoding.

Refresh the fixed layout when the official IDL changes:

```sh
python3 tools/sync_pump_create_layout.py --idl /path/to/pump.json --source https://github.com/pump-fun/pump-public-docs/blob/COMMIT/idl/pump.json
python3 tools/sync_pump_create_layout.py --check
```

Review SDK remaining-account changes separately: they cannot be inferred from
the IDL fixed list alone. Review and run the regression tests before deployment.
The layout is versioned with the parser, rather than changing silently at runtime.

`tests/pumpfun_create_regression.rs` checks fixed fields by IDL name, both create
parser paths, malformed tails, account fillers and merge behavior. New RPC
fixtures include a successful USDC create at slot 426008840 and a successful WSOL
create at slot 425973593. Their provenance and independently decoded CreateEvent
fields are in `tests/fixtures/pump_create_rpc_manifest.json`. The 20-account WSOL
transaction at slot 426008789 failed on chain; it is retained as evidence and
excluded from successful execution assertions. Synthetic tests cover longer
quote tails and unresolved ALTs.

Version 4.0.1 already used the correct fixed user/token-program indices and
quote-tail positions. Version 4.0.2 shares the IDL-derived mappings across the
create parser paths and tightens partial-tail, missing-key and token-program
validation to avoid false SOL classification.

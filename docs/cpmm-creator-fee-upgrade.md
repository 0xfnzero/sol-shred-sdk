# CPMM creator-fee protocol share

The collection upgrade appends accounts without reordering the original 14:

| Instruction | Appended accounts (one-based) |
| --- | --- |
| `CollectCreatorFee` | #15 `creator_fee_share` |
| `CollectCreatorFeePermissionless` | #15 `amm_config`, #16 `creator_fee_share` |

Both collection paths emit `DexEvent::RaydiumCpmmCollectCreatorFee`. The parser
requires all 15/16 accounts, retains their addresses, and does not invent
collection amounts or execution success from raw shreds. Event filters include
`EventType::RaydiumCpmmCollectCreatorFee`.

## Upgrade an existing call

```rust,ignore
use sol_shred_sdk::cpmm_creator_fee::upgrade_creator_fee_collection_instruction;

// Existing CPMM Instruction with its original 14 AccountMeta entries.
// amm_config must be the config identified by the pool's PoolState.
let share_pda = upgrade_creator_fee_collection_instruction(&mut ix, &amm_config)?;
```

The helper preserves the original accounts and privileges, appends readonly
accounts, and accepts an already-upgraded call after checking its trailing
addresses. An error leaves the call unchanged. It rejects another program,
another instruction, an unexpected account count, or mismatched config/PDA.
The helper does not fetch or submit a transaction.

`derive_creator_fee_share(&creator, &amm_config)` returns the address and bump
for `[b"creator_fee_share", creator, amm_config]` under the CPMM program. Always
pass this address, even if the PDA has not been created. Existence is not needed
to construct the instruction.

## Decode accounts and estimate payouts

`RaydiumCpmmAmmConfig.creator_fee_share_rate` occupies the first former padding
u64. Total size remains **236 bytes** including the discriminator; the remaining
padding is `[u64; 14]`. Code or serialized models treating the tail as 15 padding
values must migrate. `PoolState` remains **637 bytes**, with unchanged offsets.

The **145-byte** CreatorFeeShare account exposes `bump`, `creator`, `amm_config`,
`share_rate`, and eight padding u64 values through
`DexEvent::RaydiumCpmmCreatorFeeShareAccount`. Account subscriptions can request
`EventType::AccountRaydiumCpmmCreatorFeeShare`.

```rust,ignore
use sol_shred_sdk::cpmm_creator_fee::{effective_creator_fee_share_rate, split_creator_fee};

// Use the rate from the existing, validated PDA for this creator/config pair;
// use None when it does not exist. Some(0) explicitly overrides a nonzero default.
let rate = effective_creator_fee_share_rate(config.creator_fee_share_rate, override_rate)
    .ok_or("invalid creator fee share rate")?;
let token_0 = split_creator_fee(pool.creator_fees_token_0, rate).unwrap();
let token_1 = split_creator_fee(pool.creator_fees_token_1, rate).unwrap();
// Each split has creator_amount and protocol_amount.
```

Rates use a denominator of **1,000,000**. The exact split is
`protocol_amount = floor(accrued_fee * rate / 1_000_000)` and
`creator_amount = accrued_fee - protocol_amount`. Multiplication uses u128;
rounding dust stays with the creator. Rates above 1,000,000 are rejected.

The rate is resolved at collection time, so a snapshot estimate can change
before collection executes. `protocol_fees_token_0/1` also increase at creator-fee
collection; monitors must not attribute every increase to swaps. Raw shreds
cannot supply account snapshots or confirm the resulting payout. Swap, quote,
and LP calculations are unchanged.

Sources: [upgrade changelog](https://docs.raydium.io/reference/changelog/2026-09-19-cpmm-creator-fee-protocol-share),
[instruction account tables](https://docs.raydium.io/products/cpmm/instructions),
[collection arithmetic](https://docs.raydium.io/products/cpmm/fees).

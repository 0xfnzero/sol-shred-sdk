use base64::{engine::general_purpose::STANDARD, Engine as _};
use sol_shred_sdk::{
    accounts::{pumpswap, token::AccountData},
    core::{
        events::{PumpSwapBuyEvent, PumpSwapSellEvent},
        merger::{merge_events, merge_grpc_instruction_into_log},
    },
    instr::pump_amm_inner,
    logs::pump_amm,
    DexEvent, EventMetadata, Pubkey,
};

#[test]
fn pool_signed_adjustment_and_legacy_defaults_produce_effective_reserves() {
    for adjustment in [-1_000i128, -500, 0, 500] {
        // Pool discriminator + boost-era layout: the i128 starts at byte 245.
        let mut data = vec![0; 261];
        data[..8].copy_from_slice(pumpswap::discriminators::POOL_ACCOUNT);
        data[245..261].copy_from_slice(&adjustment.to_le_bytes());
        for legacy in [false, true] {
            let mut bytes = data.clone();
            if legacy {
                bytes.truncate(245);
                bytes.resize(252, 0);
            }
            let account = AccountData {
                pubkey: Pubkey::new_unique(),
                owner: Pubkey::new_unique(),
                data: bytes,
                executable: false,
                lamports: 1,
                rent_epoch: 0,
            };
            let DexEvent::PumpSwapPoolAccount(event) =
                pumpswap::parse_pool(&account, EventMetadata::default()).unwrap()
            else {
                panic!("expected Pool account")
            };
            let expected_adjustment = if legacy { 0 } else { adjustment };
            assert_eq!(event.pool.virtual_quote_reserves, expected_adjustment);
            assert_eq!(
                pumpswap::effective_quote_reserves(1_000, event.pool.virtual_quote_reserves),
                Some((1_000 + expected_adjustment) as u64)
            );
        }
    }
}

#[test]
fn buy_and_sell_log_cpi_and_json_preserve_signed_reserves() {
    for adjustment in [i128::MIN, -1_000, -500, 0, 500, i128::MAX] {
        for buy in [true, false] {
            // Buy includes its empty Borsh ix_name; Sell has a fixed prefix.
            // The boost-era tail is cashback(16), buyback(16), i128(16),
            // can_boost(1), base_supply(8).
            let prefix_len = if buy { 397 } else { 352 };
            let mut data = vec![0; prefix_len + 57];
            data[48..56].copy_from_slice(&1_000u64.to_le_bytes());
            data[prefix_len + 32..prefix_len + 48].copy_from_slice(&adjustment.to_le_bytes());
            let (decoded, discriminator) = if buy {
                (
                    pump_amm::parse_buy_from_data(&data, EventMetadata::default()).unwrap(),
                    pump_amm_inner::discriminators::BUY,
                )
            } else {
                (
                    pump_amm::parse_sell_from_data(&data, EventMetadata::default()).unwrap(),
                    pump_amm_inner::discriminators::SELL,
                )
            };
            let cpi = pump_amm_inner::parse_pumpswap_inner_instruction(
                &discriminator,
                &data,
                EventMetadata::default(),
            )
            .unwrap();
            let mut log_bytes = discriminator[8..].to_vec();
            log_bytes.extend_from_slice(&data);
            let log = format!("Program data: {}", STANDARD.encode(log_bytes));
            let mut log_event =
                pump_amm::parse_log(&log, Default::default(), 123, 0, None, 0).unwrap();
            let prefixed_log = format!("诊断前缀: {log}");
            let prefixed_event =
                pump_amm::parse_log(&prefixed_log, Default::default(), 123, 0, None, 0).unwrap();
            let event_discriminator = u64::from_le_bytes(discriminator[8..].try_into().unwrap());
            assert!(pump_amm::is_event_type(&log, event_discriminator));
            assert!(pump_amm::is_event_type(&prefixed_log, event_discriminator));
            let outer = if buy {
                DexEvent::PumpSwapBuy(PumpSwapBuyEvent::default())
            } else {
                DexEvent::PumpSwapSell(PumpSwapSellEvent::default())
            };
            // Outer instruction reserves are unavailable and default to zero.
            // They must not replace the authoritative signed value from a log.
            merge_grpc_instruction_into_log(&mut log_event, outer.clone());
            let mut merged = outer;
            merge_events(&mut merged, cpi.clone());
            for event in [decoded, cpi, log_event, prefixed_event, merged] {
                let (raw, signed) = match event {
                    DexEvent::PumpSwapBuy(e) => {
                        let json = serde_json::to_string(&e).unwrap();
                        let roundtrip: PumpSwapBuyEvent = serde_json::from_str(&json).unwrap();
                        (
                            roundtrip.pool_quote_token_reserves,
                            roundtrip.virtual_quote_reserves,
                        )
                    }
                    DexEvent::PumpSwapSell(e) => {
                        let json = serde_json::to_string(&e).unwrap();
                        let roundtrip: PumpSwapSellEvent = serde_json::from_str(&json).unwrap();
                        (
                            roundtrip.pool_quote_token_reserves,
                            roundtrip.virtual_quote_reserves,
                        )
                    }
                    _ => panic!("expected trade"),
                };
                assert_eq!(signed, adjustment);
                assert_eq!(raw, 1_000);
                let expected = match adjustment {
                    -1_000 => Some(0),
                    -500 => Some(500),
                    0 => Some(1_000),
                    500 => Some(1_500),
                    _ => None,
                };
                assert_eq!(pumpswap::effective_quote_reserves(raw, signed), expected);
            }
        }
    }
}

#[test]
fn effective_quote_reserves_match_checked_reference_across_signed_input_range() {
    fn reference(raw: u64, adjustment: i128) -> Option<u64> {
        i128::from(raw)
            .checked_add(adjustment)
            .and_then(|sum| u64::try_from(sum).ok())
    }
    let max = i128::from(u64::MAX);
    for raw in [0, 1, 1_000, u64::MAX - 1, u64::MAX] {
        for adjustment in [
            i128::MIN,
            i128::MIN + max,
            -max - 1,
            -max,
            -i128::from(raw) - 1,
            -i128::from(raw),
            -1,
            0,
            1,
            max - i128::from(raw),
            max - i128::from(raw) + 1,
            max,
            i128::MAX - max,
            i128::MAX - 1,
            i128::MAX,
        ] {
            assert_eq!(
                pumpswap::effective_quote_reserves(raw, adjustment),
                reference(raw, adjustment),
                "raw={raw}, adjustment={adjustment}"
            );
        }
    }

    // Deterministic full-width inputs; no RNG dependency or chain-state guarantee.
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..50_000 {
        let raw = next();
        let adjustment = ((u128::from(next()) << 64) | u128::from(next())) as i128;
        for adjustment in [
            adjustment,
            i128::from(next()) - i128::from(raw),
            i128::from(next()),
            -i128::from(next()),
        ] {
            assert_eq!(
                pumpswap::effective_quote_reserves(raw, adjustment),
                reference(raw, adjustment)
            );
        }
    }
}

#[test]
fn effective_quote_reserves_handle_u64_boundaries_without_wrapping() {
    let max = i128::from(u64::MAX);
    for (raw, adjustment, expected) in [
        (0, 0, Some(0)),
        (u64::MAX, 0, Some(u64::MAX)),
        (u64::MAX, -max, Some(0)),
        (0, max, Some(u64::MAX)),
        (1_000, max - 1_000, Some(u64::MAX)),
        (1_000, -1_001, None),
        (u64::MAX, 1, None),
        (0, i128::MIN, None),
        (0, i128::MAX, None),
        (1, i128::MAX, None),
    ] {
        assert_eq!(
            pumpswap::effective_quote_reserves(raw, adjustment),
            expected
        );
    }
}

#[test]
fn pool_and_global_flags_reject_non_borsh_bools() {
    // Offsets include the discriminator and follow the official account IDL.
    for (discriminator, size, flag_offsets, pool) in [
        (
            pumpswap::discriminators::POOL_ACCOUNT,
            271,
            [243, 244, 269, 270],
            true,
        ),
        (
            pumpswap::discriminators::GLOBAL_CONFIG_ACCOUNT,
            949,
            [417, 642, 939, 940],
            false,
        ),
    ] {
        let mut account = AccountData {
            pubkey: Pubkey::new_unique(),
            owner: Pubkey::new_unique(),
            data: vec![0; size],
            executable: false,
            lamports: 1,
            rent_epoch: 0,
        };
        account.data[..8].copy_from_slice(discriminator);
        if pool {
            account.data[245..261].copy_from_slice(&(-500i128).to_le_bytes());
        }
        let parse = |account: &AccountData| {
            if pool {
                pumpswap::parse_pool(account, EventMetadata::default())
            } else {
                pumpswap::parse_global_config(account, EventMetadata::default())
            }
        };
        for offset in flag_offsets {
            for byte in 0..=u8::MAX {
                account.data[offset] = byte;
                assert_eq!(
                    parse(&account).is_some(),
                    byte <= 1,
                    "pool={pool}, offset={offset}, byte={byte}"
                );
            }
            account.data[offset] = 0;
        }
        // Missing appended fields still default to zero/false on legacy accounts.
        account.data.truncate(if pool { 252 } else { 642 });
        assert!(parse(&account).is_some());
    }
}

use sol_shred_sdk::{
    accounts::{pumpswap, token::AccountData},
    core::events::{PumpSwapBuyEvent, PumpSwapSellEvent},
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
            let (log, discriminator) = if buy {
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
            for event in [log, cpi] {
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

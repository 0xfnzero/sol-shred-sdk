//! Official MeteoraAg/damm-v2 event.rs at a85c926607433f23f0ea60f4ca7b1ae92f4156cb.
//! Bodies are serialized independently of SDK decoders, using exact field order.
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{
    core::events::{DexEvent, EventMetadata},
    grpc::types::{event_type_from_dex_event, EventType, EventTypeFilter},
    instr::{
        all_inner::meteora_damm, parse_instruction_unified, program_ids::METEORA_DAMM_V2_PROGRAM_ID,
    },
    logs::{discriminator_lut, optimized_matcher},
    pubkey::Pubkey,
    signature::Signature,
};
const DISCS: [[u8; 8]; 4] = [
    [198, 182, 183, 52, 97, 12, 49, 56],
    [129, 91, 188, 3, 246, 52, 185, 249],
    [104, 233, 237, 122, 199, 191, 121, 85],
    [218, 86, 147, 200, 235, 188, 215, 231],
];
const TYPES: [EventType; 4] = [
    EventType::MeteoraDammV2ClaimPositionFee,
    EventType::MeteoraDammV2InitializeReward,
    EventType::MeteoraDammV2FundReward,
    EventType::MeteoraDammV2ClaimReward,
];
const N: u64 = u64::MAX - 7;
const RATE: u128 = u128::MAX - 19;
fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn body(kind: usize) -> Vec<u8> {
    let mut b = Vec::new();
    for n in 1..=if kind == 0 || kind == 2 { 3 } else { 4 } {
        b.extend_from_slice(&[n; 32]);
    }
    match kind {
        0 => {
            b.extend_from_slice(&N.to_le_bytes());
            b.extend_from_slice(&(N - 1).to_le_bytes());
        }
        1 => {
            b.push(255);
            b.extend_from_slice(&N.to_le_bytes());
        }
        2 => {
            b.push(255);
            for n in [N, N - 1, N - 2] {
                b.extend_from_slice(&n.to_le_bytes());
            }
            for n in [RATE, RATE - 1] {
                b.extend_from_slice(&n.to_le_bytes());
            }
        }
        3 => {
            b.push(255);
            b.extend_from_slice(&N.to_le_bytes());
        }
        _ => unreachable!(),
    }
    assert_eq!(b.len(), [112, 137, 153, 137][kind]);
    b
}
fn cpi(kind: usize, b: &[u8]) -> Vec<u8> {
    [
        vec![228, 69, 165, 46, 81, 203, 154, 29],
        DISCS[kind].to_vec(),
        b.to_vec(),
    ]
    .concat()
}
fn log(kind: usize, b: &[u8]) -> String {
    format!(
        "Program data: {}",
        STANDARD.encode([DISCS[kind].as_slice(), b].concat())
    )
}
fn metadata() -> EventMetadata {
    EventMetadata {
        signature: Signature::from([9; 64]),
        slot: 42,
        tx_index: 6,
        block_time_us: 73,
        grpc_recv_us: 89,
        recent_blockhash: Some("retained".into()),
    }
}
fn assert_event(kind: usize, event: &DexEvent) {
    assert_eq!(event_type_from_dex_event(event), Some(TYPES[kind]));
    match event {
        DexEvent::MeteoraDammV2ClaimPositionFee(e) => {
            assert_eq!((e.pool, e.position, e.owner), (key(1), key(2), key(3)));
            assert_eq!((e.fee_a_claimed, e.fee_b_claimed), (N, N - 1));
        }
        DexEvent::MeteoraDammV2InitializeReward(e) => {
            assert_eq!(
                (e.pool, e.reward_mint, e.funder, e.creator),
                (key(1), key(2), key(3), key(4))
            );
            assert_eq!((e.reward_index, e.reward_duration), (255, N));
        }
        DexEvent::MeteoraDammV2FundReward(e) => {
            assert_eq!((e.pool, e.funder, e.mint_reward), (key(1), key(2), key(3)));
            assert_eq!(e.reward_index, 255);
            assert_eq!(
                (
                    e.amount,
                    e.transfer_fee_excluded_amount_in,
                    e.reward_duration_end
                ),
                (N, N - 1, N - 2)
            );
            assert_eq!((e.pre_reward_rate, e.post_reward_rate), (RATE, RATE - 1));
        }
        DexEvent::MeteoraDammV2ClaimReward(e) => {
            assert_eq!(
                (e.pool, e.position, e.owner, e.mint_reward),
                (key(1), key(2), key(3), key(4))
            );
            assert_eq!((e.reward_index, e.total_reward), (255, N));
        }
        _ => panic!("wrong DAMM fee/reward event"),
    }
}
#[test]
fn all_four_log_cpi_and_public_unified_paths_preserve_exact_fields_and_metadata() {
    for kind in 0..4 {
        let b = body(kind);
        let wire = cpi(kind, &b);
        let text = log(kind, &b);
        let m = metadata();
        let event = meteora_damm::parse(wire[..16].try_into().unwrap(), &b, m.clone()).unwrap();
        assert_event(kind, &event);
        assert_eq!(event.metadata().recent_blockhash, m.recent_blockhash);
        let lut = discriminator_lut::parse_with_discriminator(
            u64::from_le_bytes(DISCS[kind]),
            &b,
            m.clone(),
        )
        .unwrap();
        assert_event(kind, &lut);
        let direct = sol_shred_sdk::logs::meteora_damm::parse_log(
            &text,
            m.signature,
            m.slot,
            m.tx_index,
            Some(m.block_time_us),
            m.grpc_recv_us,
        )
        .unwrap();
        assert_event(kind, &direct);
        let unified = parse_instruction_unified(
            &wire,
            &[],
            m.signature,
            m.slot,
            m.tx_index,
            Some(m.block_time_us),
            m.grpc_recv_us,
            None,
            &METEORA_DAMM_V2_PROGRAM_ID,
        )
        .unwrap();
        assert_event(kind, &unified);
        for e in [&direct, &unified] {
            assert_eq!(
                (
                    e.metadata().signature,
                    e.metadata().slot,
                    e.metadata().tx_index,
                    e.metadata().block_time_us,
                    e.metadata().grpc_recv_us
                ),
                (
                    m.signature,
                    m.slot,
                    m.tx_index,
                    m.block_time_us,
                    m.grpc_recv_us
                )
            );
        }
        let mut mutable = event;
        mutable.metadata_mut().unwrap().slot = 99;
        assert_eq!(mutable.metadata().slot, 99);
    }
}
#[test]
fn optimized_scoped_and_unscoped_filters_recognize_each_reward_and_fee_type() {
    for kind in 0..4 {
        let text = log(kind, &body(kind));
        let filter = EventTypeFilter::include_only(vec![TYPES[kind]]);
        assert!(filter.includes_meteora_damm_v2());
        for program in [None, Some(&METEORA_DAMM_V2_PROGRAM_ID)] {
            let e = optimized_matcher::parse_log_optimized_with_program_id(
                &text,
                Default::default(),
                1,
                2,
                Some(3),
                4,
                Some(&filter),
                false,
                Some(&[5; 32]),
                program,
            )
            .unwrap();
            assert_event(kind, &e);
            assert!(e.metadata().recent_blockhash.is_some());
            let unrelated = EventTypeFilter::include_only(vec![TYPES[(kind + 1) % 4]]);
            assert!(optimized_matcher::parse_log_optimized_with_program_id(
                &text,
                Default::default(),
                1,
                2,
                None,
                4,
                Some(&unrelated),
                false,
                None,
                program
            )
            .is_none());
        }
    }
}
#[test]
fn every_body_truncation_is_rejected_by_log_cpi_and_unified_entry_points() {
    for kind in 0..4 {
        let b = body(kind);
        for len in 0..b.len() {
            let wire = cpi(kind, &b[..len]);
            assert!(
                meteora_damm::parse(wire[..16].try_into().unwrap(), &b[..len], metadata())
                    .is_none(),
                "kind={kind}, len={len}"
            );
            assert!(sol_shred_sdk::logs::meteora_damm::parse_log(
                &log(kind, &b[..len]),
                Default::default(),
                1,
                0,
                None,
                0
            )
            .is_none());
            assert!(parse_instruction_unified(
                &wire,
                &[],
                Default::default(),
                1,
                0,
                None,
                0,
                None,
                &METEORA_DAMM_V2_PROGRAM_ID
            )
            .is_none());
        }
        let mut invalid = cpi(kind, &b);
        invalid[0] ^= 1;
        assert!(parse_instruction_unified(
            &invalid,
            &[],
            Default::default(),
            1,
            0,
            None,
            0,
            None,
            &METEORA_DAMM_V2_PROGRAM_ID
        )
        .is_none());
        assert!(sol_shred_sdk::logs::meteora_damm::parse_log(
            "Program data: !!!",
            Default::default(),
            1,
            0,
            None,
            0
        )
        .is_none());
    }
}

#[test]
fn full_width_reward_rates_roundtrip_through_json_values_without_precision_loss() {
    let wire = cpi(2, &body(2));
    let event =
        meteora_damm::parse(wire[..16].try_into().unwrap(), &wire[16..], metadata()).unwrap();
    let DexEvent::MeteoraDammV2FundReward(reward) = event else {
        panic!()
    };
    let json = serde_json::to_value(&reward).unwrap();
    assert_eq!(
        json["pre_reward_rate"].as_str(),
        Some(RATE.to_string().as_str())
    );
    assert_eq!(
        json["post_reward_rate"].as_str(),
        Some((RATE - 1).to_string().as_str())
    );
    let parsed: sol_shred_sdk::core::events::MeteoraDammV2FundRewardEvent =
        serde_json::from_value(json.clone()).unwrap();
    assert_eq!(
        (parsed.pre_reward_rate, parsed.post_reward_rate),
        (RATE, RATE - 1)
    );
    assert_eq!(parsed.amount, N);
    for bad in [
        serde_json::json!("340282366920938463463374607431768211456"),
        serde_json::json!(-1),
        serde_json::json!(1.5),
    ] {
        let mut changed = json.clone();
        changed["pre_reward_rate"] = bad;
        assert!(
            serde_json::from_value::<sol_shred_sdk::core::events::MeteoraDammV2FundRewardEvent>(
                changed
            )
            .is_err()
        );
    }
    let mut numeric = json;
    numeric["pre_reward_rate"] = serde_json::json!(7);
    assert_eq!(
        serde_json::from_value::<sol_shred_sdk::core::events::MeteoraDammV2FundRewardEvent>(
            numeric
        )
        .unwrap()
        .pre_reward_rate,
        7
    );
}

#[test]
fn exact_event_bodies_reject_trailing_bytes_and_lut_preserves_prior_pump_entries() {
    for kind in 0..4 {
        let mut b = body(kind);
        b.push(1);
        let wire = cpi(kind, &b);
        assert!(meteora_damm::parse(wire[..16].try_into().unwrap(), &b, metadata()).is_none());
        assert!(sol_shred_sdk::logs::meteora_damm::parse_log(
            &log(kind, &b),
            Default::default(),
            1,
            0,
            None,
            0
        )
        .is_none());
    }
    for (disc, name) in [
        (18146529233607700591, "PUMPFUN_POST_COMPLETE_BUY"),
        (3118876958563052404, "PUMPFUN_SWEEP_BONDING_CURVE_FEE"),
        (619296439455019615, "PUMPFUN_COMPLETE"),
        (11927646055507993730, "PUMPSWAP_SWEEP_POOL_FEE"),
    ] {
        assert_eq!(discriminator_lut::discriminator_to_name(disc), Some(name));
    }
}

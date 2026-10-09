use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{
    core::events::*,
    grpc::types::{event_type_from_dex_event, EventType, EventTypeFilter},
    instr::{
        all_inner::raydium_cpmm, parse_instruction_unified, program_ids::RAYDIUM_CPMM_PROGRAM_ID,
    },
    logs::{discriminator_lut, optimized_matcher},
    pubkey::Pubkey,
};
const DISC: [u8; 8] = [121, 163, 205, 201, 57, 218, 117, 60];
const HIGH: u64 = u64::MAX - 7;
fn body(kind: u8) -> Vec<u8> {
    let mut b = vec![3; 32];
    for n in 0..7 {
        b.extend_from_slice(&(HIGH - n).to_le_bytes());
    }
    b.push(kind);
    b
}
fn log(b: &[u8]) -> String {
    format!(
        "Program data: {}",
        STANDARD.encode([DISC.as_slice(), b].concat())
    )
}
fn expected(kind: u8) -> EventType {
    if kind == 0 {
        EventType::RaydiumCpmmDeposit
    } else {
        EventType::RaydiumCpmmWithdraw
    }
}
fn verify(event: &DexEvent, kind: u8) {
    assert_eq!(event_type_from_dex_event(event), Some(expected(kind)));
    let DexEvent::RaydiumCpmmLpChange(e) = event else {
        panic!("not execution event")
    };
    assert_eq!(e.pool_id, Pubkey::new_from_array([3; 32]));
    assert_eq!(
        [
            e.lp_amount_before,
            e.token_0_vault_before,
            e.token_1_vault_before,
            e.token_0_amount,
            e.token_1_amount,
            e.token_0_transfer_fee,
            e.token_1_transfer_fee
        ],
        [
            HIGH,
            HIGH - 1,
            HIGH - 2,
            HIGH - 3,
            HIGH - 4,
            HIGH - 5,
            HIGH - 6
        ]
    );
    assert_eq!(e.change_type, kind);
}
#[test]
fn official_deposit_withdraw_lp_event_preserves_execution_data_and_metadata_on_all_public_paths() {
    for kind in [0, 1] {
        let b = body(kind);
        let m = EventMetadata {
            slot: 42,
            tx_index: 7,
            block_time_us: 9,
            grpc_recv_us: 11,
            recent_blockhash: Some("retained".into()),
            ..Default::default()
        };
        let e = sol_shred_sdk::logs::raydium_cpmm::parse_log(
            &log(&b),
            m.signature,
            m.slot,
            m.tx_index,
            Some(m.block_time_us),
            m.grpc_recv_us,
        )
        .unwrap();
        verify(&e, kind);
        assert_eq!(e.metadata().slot, 42);
        let e = raydium_cpmm::parse(
            &raydium_cpmm::discriminators::LP_CHANGE_EVENT,
            &b,
            m.clone(),
        )
        .unwrap();
        verify(&e, kind);
        assert_eq!(e.metadata().recent_blockhash, m.recent_blockhash);
        verify(
            &discriminator_lut::parse_with_discriminator(u64::from_le_bytes(DISC), &b, m.clone())
                .unwrap(),
            kind,
        );
        let wire = [raydium_cpmm::discriminators::LP_CHANGE_EVENT.as_slice(), &b].concat();
        verify(
            &parse_instruction_unified(
                &wire,
                &[],
                m.signature,
                m.slot,
                m.tx_index,
                Some(m.block_time_us),
                m.grpc_recv_us,
                None,
                &RAYDIUM_CPMM_PROGRAM_ID,
            )
            .unwrap(),
            kind,
        );
    }
}
#[test]
fn existing_deposit_withdraw_filters_are_selected_from_official_change_type() {
    for kind in [0, 1] {
        let text = log(&body(kind));
        for program in [None, Some(&RAYDIUM_CPMM_PROGRAM_ID)] {
            for include in [0, 1] {
                let filter = EventTypeFilter::include_only(vec![expected(include)]);
                let e = optimized_matcher::parse_log_optimized_with_program_id(
                    &text,
                    Default::default(),
                    1,
                    0,
                    None,
                    0,
                    Some(&filter),
                    false,
                    None,
                    program,
                );
                if kind == include {
                    verify(&e.unwrap(), kind)
                } else {
                    assert!(e.is_none())
                }
            }
        }
    }
}
#[test]
fn lp_event_rejects_every_truncation_trailing_data_invalid_kind_and_envelope() {
    for kind in [0, 1] {
        let b = body(kind);
        for len in 0..b.len() {
            assert!(sol_shred_sdk::logs::raydium_cpmm::parse_log(
                &log(&b[..len]),
                Default::default(),
                1,
                0,
                None,
                0
            )
            .is_none());
            assert!(raydium_cpmm::parse(
                &raydium_cpmm::discriminators::LP_CHANGE_EVENT,
                &b[..len],
                Default::default()
            )
            .is_none());
        }
        let mut b = b;
        b.push(0);
        assert!(raydium_cpmm::parse(
            &raydium_cpmm::discriminators::LP_CHANGE_EVENT,
            &b,
            Default::default()
        )
        .is_none());
    }
    assert!(sol_shred_sdk::logs::raydium_cpmm::parse_log(
        &log(&body(2)),
        Default::default(),
        1,
        0,
        None,
        0
    )
    .is_none());
    let mut d = raydium_cpmm::discriminators::LP_CHANGE_EVENT;
    d[0] ^= 1;
    assert!(raydium_cpmm::parse(&d, &body(0), Default::default()).is_none());
}

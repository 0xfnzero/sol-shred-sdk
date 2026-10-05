use base64::{engine::general_purpose::STANDARD, Engine as _};
use sol_shred_sdk::logs::pump_amm;

#[test]
fn fast_discriminator_requires_eight_decoded_bytes() {
    for len in 0..=12 {
        let bytes = vec![0x5a; len];
        let log = format!("Program data: {}", STANDARD.encode(&bytes));
        let expected = (len >= 8).then_some(u64::from_le_bytes([0x5a; 8]));
        assert_eq!(pump_amm::get_event_type_fast(&log), expected, "len={len}");
    }
}

#[test]
fn fast_discriminator_preserves_little_endian_and_whitespace_compatibility() {
    for discriminator in [
        pump_amm::discriminators::BUY,
        pump_amm::discriminators::SELL,
        pump_amm::discriminators::CREATE_POOL,
        pump_amm::discriminators::ADD_LIQUIDITY,
        pump_amm::discriminators::REMOVE_LIQUIDITY,
    ] {
        let mut bytes = discriminator.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0; 16]);
        let encoded = STANDARD.encode(bytes);
        let spaced = encoded
            .as_bytes()
            .chunks(4)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join(" \t\r\n");
        for body in [
            encoded.clone(),
            spaced,
            format!("{}{}{}", &encoded[..16], " ".repeat(32_768), &encoded[16..]),
        ] {
            let log = format!("诊断: Program data: {body}");
            assert_eq!(pump_amm::get_event_type_fast(&log), Some(discriminator));
            assert!(pump_amm::is_event_type(&log, discriminator));
        }
    }
}

#[test]
fn complete_log_decoder_handles_buffer_and_base64_boundaries() {
    // A valid boost-era Buy with an empty name and a signed reserve adjustment.
    let mut bytes = pump_amm::discriminators::BUY.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0; 397 + 57]);
    bytes[8 + 397 + 32..8 + 397 + 48].copy_from_slice(&(-500i128).to_le_bytes());
    let parse = |body: &str| {
        pump_amm::parse_log(
            &format!("Program data: {body}"),
            Default::default(),
            1,
            0,
            None,
            0,
        )
    };
    assert!(parse(&STANDARD.encode(&bytes)).is_some());
    for encoded_len in [2_696, 2_700, 2_701, 2_732, 2_736] {
        let body = "A".repeat(encoded_len);
        assert!(parse(&body).is_none(), "encoded_len={encoded_len}");
    }
    for invalid in ["!", "é", "=", "\0"] {
        let mut body = STANDARD.encode(&bytes);
        body.replace_range(16..17, invalid);
        assert!(parse(&body).is_none());
    }
}

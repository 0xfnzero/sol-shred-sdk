use sol_shred_sdk::accounts::utils::{read_pubkey, read_u16_le, read_u64_le, read_u8};

#[test]
fn account_readers_reject_out_of_bounds_and_overflowing_offsets() {
    let data = [0x5a; 40];
    for len in 0..=data.len() {
        let bytes = &data[..len];
        for offset in (0..=41).chain(usize::MAX - 32..=usize::MAX) {
            let fits = |width| offset.checked_add(width).is_some_and(|end| end <= len);
            assert_eq!(read_pubkey(bytes, offset).is_some(), fits(32));
            assert_eq!(read_u64_le(bytes, offset).is_some(), fits(8));
            assert_eq!(read_u16_le(bytes, offset).is_some(), fits(2));
            assert_eq!(read_u8(bytes, offset).is_some(), fits(1));
        }
    }
}

#[test]
fn account_readers_preserve_unaligned_little_endian_values() {
    let data: Vec<u8> = (0..40).collect();
    assert_eq!(read_u16_le(&data, 1), Some(0x0201));
    assert_eq!(read_u64_le(&data, 1), Some(0x0807_0605_0403_0201));
    assert_eq!(read_pubkey(&data, 1).unwrap().to_bytes(), data[1..33]);
    assert_eq!(read_pubkey(&data, 8).unwrap().to_bytes(), data[8..40]);
    assert_eq!(read_u64_le(&data, 32), Some(0x2726_2524_2322_2120));
    assert_eq!(read_u16_le(&data, 38), Some(0x2726));
    assert_eq!(read_u8(&data, 39), Some(39));
}

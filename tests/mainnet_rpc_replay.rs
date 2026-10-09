#[path = "../examples/common/mod.rs"]
mod common;

#[test]
fn captured_mainnet_swaps_match_wire_and_token_balances() {
    common::replay_corpus().unwrap();
}

#[test]
fn corrupt_balance_is_detected() {
    let mut j: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mainnet/orca-0.json")).unwrap();
    for b in j["meta"]["postTokenBalances"].as_array_mut().unwrap() {
        let amount: u64 = b["uiTokenAmount"]["amount"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        b["uiTokenAmount"]["amount"] = (amount + 1).to_string().into();
    }
    assert!(common::validate(&j)
        .unwrap_err()
        .to_string()
        .contains("token balance mismatch"));
}

#[test]
fn real_v1_reference_does_not_fabricate_a_dlmm_swap() {
    let j: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/mainnet_negative/referenced_dlmm_v1.json"
    ))
    .unwrap();
    assert_eq!(j["version"], 1);
    let (outer, route, checked) = common::validate(&j).unwrap();
    assert_eq!(outer, 0);
    assert!(route.succeeded);
    assert!(route.legs.is_empty());
    assert_eq!(checked, 0);
    assert_eq!(
        route.signature.to_string(),
        "2nYbrhhkGx8YiN4upcAAS8SaLTFRrrJAY98m2nqdyEnZ1nEH7BmGcyjYq1BBLVRvqoQYmw2Ta1ryfvSGVgBiENeM"
    );
}

// Modified mainnet messages become explicitly synthetic signed projections.
// This keeps ABI/PDA rejection tests beyond the cold-path signature gate.
fn resign_projection(tx: &mut sol_shred_sdk::transaction::VersionedTransaction) {
    use solana_keypair::{Keypair, Signer};
    use sol_shred_sdk::message::VersionedMessage;
    let count = tx.message.header().num_required_signatures as usize;
    let signers: Vec<_> = (0..count).map(|_| Keypair::new()).collect();
    let keys = match &mut tx.message {
        VersionedMessage::Legacy(m) => &mut m.account_keys,
        VersionedMessage::V0(m) => &mut m.account_keys,
        VersionedMessage::V1(m) => &mut m.account_keys,
    };
    for (key, signer) in keys.iter_mut().zip(&signers) { *key = signer.pubkey(); }
    let message = tx.message.serialize();
    tx.signatures = signers.iter().map(|s| s.sign_message(&message)).collect();
}

#[test]
fn truncated_real_lp_instruction_is_rejected() {
    use base64::Engine;
    use sol_shred_sdk::{message::VersionedMessage, transaction::VersionedTransaction};
    let mut j: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mainnet/cpmm-deposit.json")).unwrap();
    let raw = base64::engine::general_purpose::STANDARD
        .decode(j["transaction"][0].as_str().unwrap())
        .unwrap();
    let mut tx: VersionedTransaction = wincode::deserialize(&raw).unwrap();
    let instructions = match &mut tx.message {
        VersionedMessage::Legacy(m) => &mut m.instructions,
        VersionedMessage::V0(m) => &mut m.instructions,
        VersionedMessage::V1(m) => &mut m.instructions,
    };
    let ix = instructions
        .iter_mut()
        .find(|i| i.data.get(..8) == Some(&[242, 35, 198, 137, 82, 225, 242, 182]))
        .unwrap();
    ix.data.truncate(16);
    resign_projection(&mut tx);
    j["transaction"][0] = base64::engine::general_purpose::STANDARD
        .encode(wincode::serialize(&tx).unwrap())
        .into();
    assert!(common::instructions::validate(&j)
        .unwrap_err()
        .to_string()
        .contains("supported instruction did not parse"));
}

#[test]
fn real_collection_requires_the_canonical_share_pda() {
    use base64::Engine;
    use sol_shred_sdk::{message::VersionedMessage, transaction::VersionedTransaction};
    let mut j: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/mainnet/cpmm-collect-creator-fee.json"
    ))
    .unwrap();
    let raw = base64::engine::general_purpose::STANDARD
        .decode(j["transaction"][0].as_str().unwrap())
        .unwrap();
    let mut tx: VersionedTransaction = wincode::deserialize(&raw).unwrap();
    let instructions = match &mut tx.message {
        VersionedMessage::Legacy(m) => &mut m.instructions,
        VersionedMessage::V0(m) => &mut m.instructions,
        VersionedMessage::V1(m) => &mut m.instructions,
    };
    let ix = instructions
        .iter_mut()
        .find(|i| i.data.get(..8) == Some(&[20, 22, 86, 123, 198, 28, 219, 132]))
        .unwrap();
    assert_eq!(ix.accounts.len(), 15);
    // Point the appended share account at the creator instead of its canonical PDA.
    ix.accounts[14] = ix.accounts[0];
    resign_projection(&mut tx);
    j["transaction"][0] = base64::engine::general_purpose::STANDARD
        .encode(wincode::serialize(&tx).unwrap())
        .into();
    assert!(common::instructions::validate(&j)
        .unwrap_err()
        .to_string()
        .contains("creator_fee_share PDA mismatch"));
}

#[path = "../examples/common/mod.rs"]
mod common;

#[test]
fn captured_mainnet_compact_and_legacy_swaps_match_wire_and_balances() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/pump_upgrade/mainnet.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        common::validate(&case["encoded"]).unwrap_or_else(|e| panic!("{}: {e}", case["signature"]));
    }
}

mod filters;

use base64::Engine;
use prost::Message as ProstMessage;
use serde_json::Value;
use sol_shred_sdk::{
    hash::Hash,
    instruction::CompiledInstruction,
    message::{v0, MessageHeader, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};
use std::{fs, path::Path};
use yellowstone_grpc_proto::prelude::{Message, Transaction, TransactionStatusMeta};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let idls = Path::new(
        args.get(1)
            .map(String::as_str)
            .unwrap_or("../solana-program-idls"),
    );
    let mut differences = Vec::new();
    let mut compared = 0;
    let mut compared_transactions = 0;
    let mut unsupported = 0;
    let mut templates = Vec::new();
    let programs = [
        (
            "pump.json",
            sol_shred_sdk::instr::program_ids::PUMPFUN_PROGRAM_ID,
        ),
        (
            "pump_amm.json",
            sol_shred_sdk::instr::program_ids::PUMPSWAP_PROGRAM_ID,
        ),
        (
            "pump_fees.json",
            sol_shred_sdk::instr::program_ids::PUMP_FEES_PROGRAM_ID,
        ),
        (
            "raydium_launchpad.json",
            sol_shred_sdk::instr::program_ids::RAYDIUM_LAUNCHLAB_PROGRAM_ID,
        ),
        (
            "raydium_cpmm.json",
            sol_shred_sdk::instr::program_ids::RAYDIUM_CPMM_PROGRAM_ID,
        ),
        (
            "raydium_clmm.json",
            sol_shred_sdk::instr::program_ids::RAYDIUM_CLMM_PROGRAM_ID,
        ),
        (
            "orca_whirlpool.json",
            sol_shred_sdk::instr::program_ids::ORCA_WHIRLPOOL_PROGRAM_ID,
        ),
        (
            "meteora_amm.json",
            sol_shred_sdk::instr::program_ids::METEORA_POOLS_PROGRAM_ID,
        ),
        (
            "meteora_damm_v2.json",
            sol_shred_sdk::instr::program_ids::METEORA_DAMM_V2_PROGRAM_ID,
        ),
        (
            "meteora_dlmm.json",
            sol_shred_sdk::instr::program_ids::METEORA_DLMM_PROGRAM_ID,
        ),
        (
            "raydium_amm.json",
            sol_shred_sdk::instr::program_ids::RAYDIUM_AMM_V4_PROGRAM_ID,
        ),
    ];
    for (file, program) in programs {
        let path = idls.join(file);
        assert!(path.exists(), "missing IDL: {}", path.display());
        let idl: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        for ix in idl["instructions"].as_array().unwrap() {
            let name = ix["name"].as_str().unwrap();
            let discriminator: Vec<u8> = if let Some(disc) = ix["discriminator"].as_array() {
                disc.iter().map(|v| v.as_u64().unwrap() as u8).collect()
            } else if file == "raydium_amm.json" {
                let disc = match name {
                    "initialize2" => 1,
                    "deposit" => 3,
                    "withdraw" => 4,
                    "withdrawPnl" => 7,
                    "swapBaseIn" => 9,
                    "swapBaseOut" => 11,
                    "swapBaseInV2" => 16,
                    "swapBaseOutV2" => 17,
                    _ => continue,
                };
                vec![disc]
            } else {
                panic!("missing discriminator in {file}/{name}");
            };
            for seed in [0u8, 1, 2, 255] {
                let mut data = discriminator.clone();
                for (arg_index, arg) in ix["args"].as_array().unwrap().iter().enumerate() {
                    // This fixture supplies only fixed accounts. A DLMM hook slice
                    // would require corresponding appended accounts; use an empty
                    // Borsh vector to keep these positive swap cases valid.
                    if file == "meteora_dlmm.json"
                        && matches!(
                            name,
                            "swap2" | "swap_exact_out2" | "swap_with_price_impact2"
                        )
                        && arg["name"] == "remaining_accounts_info"
                    {
                        data.extend_from_slice(&0u32.to_le_bytes());
                        continue;
                    }
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        encode_type(
                            &arg["type"],
                            &idl,
                            if seed == 0 {
                                0
                            } else {
                                seed.wrapping_add(arg_index as u8)
                            },
                            &mut data,
                        )
                    }))
                    .unwrap_or_else(|_| panic!("failed to generate {file}/{name}: {arg}"));
                }
                let Some(layout) = ix["accounts"].as_array() else {
                    continue;
                };
                let accounts: Vec<Pubkey> = layout
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        if a["optional"] == true {
                            program
                        } else if let Some(address) = a["address"].as_str() {
                            address.parse().unwrap()
                        } else {
                            let mut key = [0; 32];
                            key[0] = (i + 1) as u8;
                            key[31] = 1;
                            Pubkey::from(key)
                        }
                    })
                    .collect();
                let n = accounts.len();
                let mut keys = accounts;
                keys.push(program);
                let mut transaction = VersionedTransaction {
                    signatures: vec![Signature::default()],
                    message: VersionedMessage::V0(v0::Message {
                        header: MessageHeader {
                            num_required_signatures: 1,
                            num_readonly_signed_accounts: 0,
                            num_readonly_unsigned_accounts: 0,
                        },
                        account_keys: keys.clone(),
                        recent_blockhash: Hash::default(),
                        instructions: vec![CompiledInstruction::new_from_raw_parts(
                            n as u8,
                            data.clone(),
                            (0..n as u8).collect(),
                        )],
                        address_table_lookups: vec![],
                    }),
                };
                let mut grpc_tx = Some(Transaction {
                    message: Some(Message {
                        account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                        recent_blockhash: vec![0; 32],
                        instructions: vec![yellowstone_grpc_proto::prelude::CompiledInstruction {
                            program_id_index: n as u32,
                            data,
                            accounts: (0..n as u8).collect(),
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                });
                if file == "pump_amm.json" {
                    // Standalone fee queries are observable, but carry no pool identity.
                    // They must not override the swap's pool flag.
                    let flag = seed % 2;
                    let mut fees_data = vec![231, 37, 126, 85, 207, 91, 63, 52, flag];
                    fees_data.extend_from_slice(&((1 - flag) as u128).to_le_bytes());
                    fees_data.extend_from_slice(&123u64.to_le_bytes());
                    fees_data.push(0);
                    let VersionedMessage::V0(message) = &mut transaction.message else {
                        unreachable!()
                    };
                    let fee_idx = message.account_keys.len();
                    let fees = sol_shred_sdk::instr::program_ids::PUMP_FEES_PROGRAM_ID;
                    message.account_keys.push(fees);
                    message
                        .instructions
                        .push(CompiledInstruction::new_from_raw_parts(
                            fee_idx as u8,
                            fees_data.clone(),
                            vec![],
                        ));
                    let message = grpc_tx.as_mut().unwrap().message.as_mut().unwrap();
                    message.account_keys.push(fees.to_bytes().to_vec());
                    message.instructions.push(
                        yellowstone_grpc_proto::prelude::CompiledInstruction {
                            program_id_index: fee_idx as u32,
                            data: fees_data,
                            accounts: vec![],
                        },
                    );
                }
                let expected =
                    sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
                        &TransactionStatusMeta::default(),
                        &grpc_tx,
                        Default::default(),
                        42,
                        3,
                        None,
                        0,
                        None,
                    );
                if expected.is_empty()
                    && file == "orca_whirlpool.json"
                    && matches!(
                        name,
                        "initialize_pool"
                            | "initialize_pool_v2"
                            | "increase_liquidity"
                            | "increase_liquidity_v2"
                            | "decrease_liquidity"
                            | "decrease_liquidity_v2"
                    )
                {
                    differences.push(format!(
                        "{file}/{name}/case{seed}: required Orca IDL instruction is unsupported"
                    ));
                }
                if expected.is_empty()
                    && file == "meteora_amm.json"
                    && matches!(
                        name,
                        "add_balance_liquidity"
                            | "add_imbalance_liquidity"
                            | "remove_balance_liquidity"
                            | "remove_liquidity_single_side"
                            | "bootstrap_liquidity"
                    )
                {
                    differences.push(format!("{file}/{name}/case{seed}: required Pools liquidity instruction is unsupported"));
                }
                if expected.is_empty()
                    && file == "meteora_amm.json"
                    && matches!(
                        name,
                        "set_pool_fees"
                            | "initialize_permissionless_constant_product_pool_with_config"
                            | "initialize_permissionless_constant_product_pool_with_config2"
                    )
                {
                    differences.push(format!("{file}/{name}/case{seed}: required Pools management instruction is unsupported"));
                }
                if expected.is_empty()
                    && file == "meteora_amm.json"
                    && matches!(
                        name,
                        "initialize_permissioned_pool"
                            | "initialize_permissionless_pool"
                            | "initialize_permissionless_pool_with_fee_tier"
                            | "initialize_customizable_permissionless_constant_product_pool"
                    )
                {
                    differences.push(format!(
                        "{file}/{name}/case{seed}: required Pools creation is unsupported"
                    ));
                }
                if expected.is_empty()
                    && file == "raydium_cpmm.json"
                    && matches!(
                        name,
                        "collect_creator_fee" | "collect_creator_fee_permissionless"
                    )
                {
                    differences.push(format!(
                        "{file}/{name}/case{seed}: upgraded creator collection is unsupported"
                    ));
                }
                if expected.is_empty() {
                    let mut actual = vec![];
                    sol_shred_sdk::parse_transaction_dex_events(
                        &transaction,
                        Signature::default(),
                        42,
                        3,
                        0,
                        &mut actual,
                    );
                    unsupported += 1;
                    if !actual.is_empty() {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: shred emits events absent from gRPC"
                        ));
                    }
                    continue;
                }
                if seed == 1 {
                    templates.push((format!("{file}/{name}"), transaction.clone()));
                }
                compared += 1;
                compared_transactions += 6; // static, ALT, three filters, independent invocations
                let mut actual = vec![];
                sol_shred_sdk::parse_transaction_dex_events(
                    &transaction,
                    Signature::default(),
                    42,
                    3,
                    0,
                    &mut actual,
                );
                // Independently verify CPMM wire fields so shared SDK mistakes
                // cannot pass the differential comparison.
                if let Some(sol_shred_sdk::DexEvent::RaydiumCpmmSwap(event)) = actual.first() {
                    let ix = &transaction.message.instructions()[0];
                    let first = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
                    let second = u64::from_le_bytes(ix.data[16..24].try_into().unwrap());
                    let keys = transaction.message.static_account_keys();
                    let args = if event.base_input {
                        (event.amount_in, event.minimum_amount_out)
                    } else {
                        (event.max_amount_in, event.amount_out)
                    };
                    if args != (first, second)
                        || event.payer != keys[ix.accounts[0] as usize]
                        || event.authority != keys[ix.accounts[1] as usize]
                        || event.input_token_account != keys[ix.accounts[4] as usize]
                        || event.output_token_account != keys[ix.accounts[5] as usize]
                        || (event.input_amount, event.output_amount) != (0, 0)
                    {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: CPMM wire semantics differ from IDL"
                        ));
                    }
                }
                if let Some(sol_shred_sdk::DexEvent::RaydiumClmmSwap(event)) = actual.first() {
                    let ix = &transaction.message.instructions()[0];
                    let amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
                    let threshold = u64::from_le_bytes(ix.data[16..24].try_into().unwrap());
                    let price_limit = u128::from_le_bytes(ix.data[24..40].try_into().unwrap());
                    if (
                        event.amount,
                        event.other_amount_threshold,
                        event.sqrt_price_limit_x64,
                    ) != (amount, threshold, price_limit)
                        || event.is_base_input != (ix.data[40] == 1)
                        || event.ix_name != name
                        || (event.amount_0, event.amount_1, event.sqrt_price_x64) != (0, 0, 0)
                        || event.zero_for_one
                    {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: CLMM wire semantics differ from IDL"
                        ));
                    }
                }
                if file == "meteora_amm.json" && name == "swap" {
                    let ix = &transaction.message.instructions()[0];
                    let keys = transaction.message.static_account_keys();
                    let account = |index: usize| keys[ix.accounts[index] as usize];
                    let valid = match actual.first() {
                        Some(sol_shred_sdk::DexEvent::MeteoraPoolsSwap(e)) => {
                            e.ix_name == "swap"
                                && e.amount_in
                                    == u64::from_le_bytes(ix.data[8..16].try_into().unwrap())
                                && e.minimum_out_amount
                                    == u64::from_le_bytes(ix.data[16..24].try_into().unwrap())
                                && (
                                    e.in_amount,
                                    e.out_amount,
                                    e.trade_fee,
                                    e.admin_fee,
                                    e.host_fee,
                                ) == (0, 0, 0, 0, 0)
                                && e.pool == account(0)
                                && e.user_source_token == account(1)
                                && e.user_destination_token == account(2)
                                && e.a_vault == account(3)
                                && e.b_vault == account(4)
                                && e.a_token_vault == account(5)
                                && e.b_token_vault == account(6)
                                && e.a_vault_lp_mint == account(7)
                                && e.b_vault_lp_mint == account(8)
                                && e.a_vault_lp == account(9)
                                && e.b_vault_lp == account(10)
                                && e.protocol_token_fee == account(11)
                                && e.user == account(12)
                                && e.vault_program == account(13)
                                && e.token_program == account(14)
                        }
                        _ => false,
                    };
                    if !valid {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: Pools swap differs from IDL"
                        ));
                    }
                }
                if file == "meteora_amm.json"
                    && matches!(
                        name,
                        "add_balance_liquidity"
                            | "add_imbalance_liquidity"
                            | "remove_balance_liquidity"
                            | "remove_liquidity_single_side"
                            | "bootstrap_liquidity"
                    )
                {
                    let ix = &transaction.message.instructions()[0];
                    let keys = transaction.message.static_account_keys();
                    let account = |index: usize| keys[ix.accounts[index] as usize];
                    let value = |index: usize| {
                        u64::from_le_bytes(
                            ix.data[8 + index * 8..16 + index * 8].try_into().unwrap(),
                        )
                    };
                    let valid = match (name, actual.first()) {
                        (
                            "add_balance_liquidity",
                            Some(sol_shred_sdk::DexEvent::MeteoraPoolsAddLiquidity(e)),
                        ) => {
                            e.ix_name == name
                                && e.pool_token_amount == value(0)
                                && e.maximum_token_a_amount == value(1)
                                && e.maximum_token_b_amount == value(2)
                                && e.pool == account(0)
                                && e.lp_mint == account(1)
                                && e.user_pool_lp == account(2)
                                && e.a_vault_lp == account(3)
                                && e.b_vault_lp == account(4)
                                && e.a_vault == account(5)
                                && e.b_vault == account(6)
                                && e.a_vault_lp_mint == account(7)
                                && e.b_vault_lp_mint == account(8)
                                && e.a_token_vault == account(9)
                                && e.b_token_vault == account(10)
                                && e.user_a_token == account(11)
                                && e.user_b_token == account(12)
                                && e.user == account(13)
                                && e.vault_program == account(14)
                                && e.token_program == account(15)
                                && e.lp_mint_amount == 0
                                && e.token_a_amount == 0
                                && e.token_b_amount == 0
                        }
                        (
                            "add_imbalance_liquidity",
                            Some(sol_shred_sdk::DexEvent::MeteoraPoolsAddLiquidity(e)),
                        ) => {
                            e.ix_name == name
                                && e.minimum_pool_token_amount == value(0)
                                && e.token_a_in_amount == value(1)
                                && e.token_b_in_amount == value(2)
                                && e.pool == account(0)
                                && e.lp_mint == account(1)
                                && e.user_pool_lp == account(2)
                                && e.a_vault_lp == account(3)
                                && e.b_vault_lp == account(4)
                                && e.a_vault == account(5)
                                && e.b_vault == account(6)
                                && e.a_vault_lp_mint == account(7)
                                && e.b_vault_lp_mint == account(8)
                                && e.a_token_vault == account(9)
                                && e.b_token_vault == account(10)
                                && e.user_a_token == account(11)
                                && e.user_b_token == account(12)
                                && e.user == account(13)
                                && e.vault_program == account(14)
                                && e.token_program == account(15)
                                && e.lp_mint_amount == 0
                                && e.token_a_amount == 0
                                && e.token_b_amount == 0
                        }
                        (
                            "remove_balance_liquidity",
                            Some(sol_shred_sdk::DexEvent::MeteoraPoolsRemoveLiquidity(e)),
                        ) => {
                            e.ix_name == name
                                && e.pool_token_amount == value(0)
                                && e.minimum_a_token_out == value(1)
                                && e.minimum_b_token_out == value(2)
                                && e.pool == account(0)
                                && e.lp_mint == account(1)
                                && e.user_pool_lp == account(2)
                                && e.a_vault_lp == account(3)
                                && e.b_vault_lp == account(4)
                                && e.a_vault == account(5)
                                && e.b_vault == account(6)
                                && e.a_vault_lp_mint == account(7)
                                && e.b_vault_lp_mint == account(8)
                                && e.a_token_vault == account(9)
                                && e.b_token_vault == account(10)
                                && e.user_a_token == account(11)
                                && e.user_b_token == account(12)
                                && e.user == account(13)
                                && e.vault_program == account(14)
                                && e.token_program == account(15)
                                && e.lp_unmint_amount == 0
                                && e.token_a_out_amount == 0
                                && e.token_b_out_amount == 0
                        }
                        (
                            "remove_liquidity_single_side",
                            Some(sol_shred_sdk::DexEvent::MeteoraPoolsRemoveLiquidity(e)),
                        ) => {
                            e.ix_name == name
                                && e.pool_token_amount == value(0)
                                && e.minimum_out_amount == value(1)
                                && e.pool == account(0)
                                && e.lp_mint == account(1)
                                && e.user_pool_lp == account(2)
                                && e.a_vault_lp == account(3)
                                && e.b_vault_lp == account(4)
                                && e.a_vault == account(5)
                                && e.b_vault == account(6)
                                && e.a_vault_lp_mint == account(7)
                                && e.b_vault_lp_mint == account(8)
                                && e.a_token_vault == account(9)
                                && e.b_token_vault == account(10)
                                && e.user_destination_token == account(11)
                                && e.user == account(12)
                                && e.vault_program == account(13)
                                && e.token_program == account(14)
                                && e.lp_unmint_amount == 0
                                && e.token_a_out_amount == 0
                                && e.token_b_out_amount == 0
                                && e.user_a_token == Pubkey::default()
                                && e.user_b_token == Pubkey::default()
                        }
                        (
                            "bootstrap_liquidity",
                            Some(sol_shred_sdk::DexEvent::MeteoraPoolsBootstrapLiquidity(e)),
                        ) => {
                            e.ix_name == name
                                && e.token_a_in_amount == value(0)
                                && e.token_b_in_amount == value(1)
                                && e.pool == account(0)
                                && e.lp_mint == account(1)
                                && e.user_pool_lp == account(2)
                                && e.a_vault_lp == account(3)
                                && e.b_vault_lp == account(4)
                                && e.a_vault == account(5)
                                && e.b_vault == account(6)
                                && e.a_vault_lp_mint == account(7)
                                && e.b_vault_lp_mint == account(8)
                                && e.a_token_vault == account(9)
                                && e.b_token_vault == account(10)
                                && e.user_a_token == account(11)
                                && e.user_b_token == account(12)
                                && e.user == account(13)
                                && e.vault_program == account(14)
                                && e.token_program == account(15)
                                && e.lp_mint_amount == 0
                                && e.token_a_amount == 0
                                && e.token_b_amount == 0
                        }
                        _ => false,
                    };
                    if !valid {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: Pools liquidity differs from IDL"
                        ));
                    }
                }
                if file == "meteora_amm.json"
                    && matches!(
                        name,
                        "set_pool_fees"
                            | "initialize_permissionless_constant_product_pool_with_config"
                            | "initialize_permissionless_constant_product_pool_with_config2"
                    )
                {
                    let ix = &transaction.message.instructions()[0];
                    let keys = transaction.message.static_account_keys();
                    let account = |index: usize| keys[ix.accounts[index] as usize];
                    let value = |index: usize| {
                        u64::from_le_bytes(
                            ix.data[8 + index * 8..16 + index * 8].try_into().unwrap(),
                        )
                    };
                    let valid = match actual.first() {
                        Some(sol_shred_sdk::DexEvent::MeteoraPoolsPoolCreated(e)) => {
                            let activation = if name.ends_with("config2") && ix.data[24] == 1 {
                                Some(u64::from_le_bytes(ix.data[25..33].try_into().unwrap()))
                            } else {
                                None
                            };
                            e.ix_name == name
                                && e.pool_type == 0
                                && e.token_a_in_amount == value(0)
                                && e.token_b_in_amount == value(1)
                                && e.activation_point == activation
                                && e.pool == account(0)
                                && e.config == account(1)
                                && e.lp_mint == account(2)
                                && e.token_a_mint == account(3)
                                && e.token_b_mint == account(4)
                                && e.a_vault == account(5)
                                && e.b_vault == account(6)
                                && e.a_token_vault == account(7)
                                && e.b_token_vault == account(8)
                                && e.a_vault_lp_mint == account(9)
                                && e.b_vault_lp_mint == account(10)
                                && e.a_vault_lp == account(11)
                                && e.b_vault_lp == account(12)
                                && e.payer_token_a == account(13)
                                && e.payer_token_b == account(14)
                                && e.payer_pool_lp == account(15)
                                && e.protocol_token_a_fee == account(16)
                                && e.protocol_token_b_fee == account(17)
                                && e.payer == account(18)
                                && e.rent == account(19)
                                && e.mint_metadata == account(20)
                                && e.metadata_program == account(21)
                                && e.vault_program == account(22)
                                && e.token_program == account(23)
                                && e.associated_token_program == account(24)
                                && e.system_program == account(25)
                        }
                        Some(sol_shred_sdk::DexEvent::MeteoraPoolsSetPoolFees(e)) => {
                            e.ix_name == name
                                && e.pool == account(0)
                                && e.fee_operator == account(1)
                                && e.trade_fee_numerator == value(0)
                                && e.trade_fee_denominator == value(1)
                                && e.protocol_trade_fee_numerator == value(2)
                                && e.protocol_trade_fee_denominator == value(3)
                                && e.new_partner_fee_numerator == value(4)
                        }
                        _ => false,
                    };
                    if !valid {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: Pools management differs from IDL"
                        ));
                    }
                }
                if file == "meteora_amm.json"
                    && matches!(
                        name,
                        "initialize_permissioned_pool"
                            | "initialize_permissionless_pool"
                            | "initialize_permissionless_pool_with_fee_tier"
                            | "initialize_customizable_permissionless_constant_product_pool"
                    )
                {
                    let ix = &transaction.message.instructions()[0];
                    let keys = transaction.message.static_account_keys();
                    let account = |index: usize| keys[ix.accounts[index] as usize];
                    let wire = &ix.data[8..];
                    let value = |index: usize| {
                        u64::from_le_bytes(wire[index..index + 8].try_into().unwrap())
                    };
                    let valid = match actual.first() {
                        Some(sol_shred_sdk::DexEvent::MeteoraPoolsPoolCreated(e)) => {
                            let layout_ok = match name {
                                "initialize_permissioned_pool" => {
                                    e.pool == account(0)
                                        && e.lp_mint == account(1)
                                        && e.token_a_mint == account(2)
                                        && e.token_b_mint == account(3)
                                        && e.a_vault == account(4)
                                        && e.b_vault == account(5)
                                        && e.a_vault_lp_mint == account(6)
                                        && e.b_vault_lp_mint == account(7)
                                        && e.a_vault_lp == account(8)
                                        && e.b_vault_lp == account(9)
                                        && e.admin_token_a == account(10)
                                        && e.admin_token_b == account(11)
                                        && e.admin_pool_lp == account(12)
                                        && e.protocol_token_a_fee == account(13)
                                        && e.protocol_token_b_fee == account(14)
                                        && e.admin == account(15)
                                        && e.fee_owner == account(16)
                                        && e.rent == account(17)
                                        && e.mint_metadata == account(18)
                                        && e.metadata_program == account(19)
                                        && e.vault_program == account(20)
                                        && e.token_program == account(21)
                                        && e.associated_token_program == account(22)
                                        && e.system_program == account(23)
                                }
                                "initialize_permissionless_pool" => {
                                    e.pool == account(0)
                                        && e.lp_mint == account(1)
                                        && e.token_a_mint == account(2)
                                        && e.token_b_mint == account(3)
                                        && e.a_vault == account(4)
                                        && e.b_vault == account(5)
                                        && e.a_token_vault == account(6)
                                        && e.b_token_vault == account(7)
                                        && e.a_vault_lp_mint == account(8)
                                        && e.b_vault_lp_mint == account(9)
                                        && e.a_vault_lp == account(10)
                                        && e.b_vault_lp == account(11)
                                        && e.payer_token_a == account(12)
                                        && e.payer_token_b == account(13)
                                        && e.payer_pool_lp == account(14)
                                        && e.protocol_token_a_fee == account(15)
                                        && e.protocol_token_b_fee == account(16)
                                        && e.payer == account(17)
                                        && e.fee_owner == account(18)
                                        && e.rent == account(19)
                                        && e.mint_metadata == account(20)
                                        && e.metadata_program == account(21)
                                        && e.vault_program == account(22)
                                        && e.token_program == account(23)
                                        && e.associated_token_program == account(24)
                                        && e.system_program == account(25)
                                }
                                "initialize_permissionless_pool_with_fee_tier" => {
                                    e.pool == account(0)
                                        && e.lp_mint == account(1)
                                        && e.token_a_mint == account(2)
                                        && e.token_b_mint == account(3)
                                        && e.a_vault == account(4)
                                        && e.b_vault == account(5)
                                        && e.a_token_vault == account(6)
                                        && e.b_token_vault == account(7)
                                        && e.a_vault_lp_mint == account(8)
                                        && e.b_vault_lp_mint == account(9)
                                        && e.a_vault_lp == account(10)
                                        && e.b_vault_lp == account(11)
                                        && e.payer_token_a == account(12)
                                        && e.payer_token_b == account(13)
                                        && e.payer_pool_lp == account(14)
                                        && e.protocol_token_a_fee == account(15)
                                        && e.protocol_token_b_fee == account(16)
                                        && e.payer == account(17)
                                        && e.fee_owner == account(18)
                                        && e.rent == account(19)
                                        && e.mint_metadata == account(20)
                                        && e.metadata_program == account(21)
                                        && e.vault_program == account(22)
                                        && e.token_program == account(23)
                                        && e.associated_token_program == account(24)
                                        && e.system_program == account(25)
                                }
                                "initialize_customizable_permissionless_constant_product_pool" => {
                                    e.pool == account(0)
                                        && e.lp_mint == account(1)
                                        && e.token_a_mint == account(2)
                                        && e.token_b_mint == account(3)
                                        && e.a_vault == account(4)
                                        && e.b_vault == account(5)
                                        && e.a_token_vault == account(6)
                                        && e.b_token_vault == account(7)
                                        && e.a_vault_lp_mint == account(8)
                                        && e.b_vault_lp_mint == account(9)
                                        && e.a_vault_lp == account(10)
                                        && e.b_vault_lp == account(11)
                                        && e.payer_token_a == account(12)
                                        && e.payer_token_b == account(13)
                                        && e.payer_pool_lp == account(14)
                                        && e.protocol_token_a_fee == account(15)
                                        && e.protocol_token_b_fee == account(16)
                                        && e.payer == account(17)
                                        && e.rent == account(18)
                                        && e.mint_metadata == account(19)
                                        && e.metadata_program == account(20)
                                        && e.vault_program == account(21)
                                        && e.token_program == account(22)
                                        && e.associated_token_program == account(23)
                                        && e.system_program == account(24)
                                }
                                _ => false,
                            };
                            let params_ok = if name
                                == "initialize_customizable_permissionless_constant_product_pool"
                            {
                                let offset = if wire[20] == 1 { 29 } else { 21 };
                                let activation = if wire[20] == 1 { Some(value(21)) } else { None };
                                e.pool_type == 0
                                    && e.stable_curve.is_none()
                                    && e.trade_fee_bps.is_none()
                                    && (e.token_a_in_amount, e.token_b_in_amount)
                                        == (value(0), value(8))
                                    && e.activation_point == activation
                                    && e.customizable_params.as_ref().is_some_and(|p| {
                                        p.trade_fee_numerator
                                            == u32::from_le_bytes(wire[16..20].try_into().unwrap())
                                            && p.activation_point == activation
                                            && p.has_alpha_vault == (wire[offset] == 1)
                                            && p.activation_type == wire[offset + 1]
                                            && p.padding == wire[offset + 2..offset + 92]
                                    })
                            } else {
                                let offset = if wire[0] == 0 { 1 } else { 51 };
                                let tier = name.ends_with("fee_tier");
                                let curve_ok = if wire[0] == 0 {
                                    e.stable_curve.is_none()
                                } else {
                                    e.stable_curve.as_ref().is_some_and(|p| {
                                        p.amp == value(1)
                                            && p.token_a_multiplier == value(9)
                                            && p.token_b_multiplier == value(17)
                                            && p.precision_factor == wire[25]
                                            && p.base_virtual_price == value(26)
                                            && p.base_cache_updated == value(34)
                                            && p.depeg_type == wire[42]
                                            && p.last_amp_updated_timestamp == value(43)
                                    })
                                };
                                let inputs = if name == "initialize_permissioned_pool" {
                                    (0, 0)
                                } else {
                                    let start = offset + if tier { 8 } else { 0 };
                                    (value(start), value(start + 8))
                                };
                                e.pool_type == wire[0]
                                    && curve_ok
                                    && e.trade_fee_bps
                                        == if tier { Some(value(offset)) } else { None }
                                    && (e.token_a_in_amount, e.token_b_in_amount) == inputs
                                    && e.customizable_params.is_none()
                            };
                            e.ix_name == name && layout_ok && params_ok
                        }
                        _ => false,
                    };
                    if !valid {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: Pools creation differs from IDL"
                        ));
                    }
                }
                if file == "raydium_amm.json"
                    && matches!(
                        name,
                        "swapBaseIn" | "swapBaseOut" | "swapBaseInV2" | "swapBaseOutV2"
                    )
                {
                    let ix = &transaction.message.instructions()[0];
                    let first = u64::from_le_bytes(ix.data[1..9].try_into().unwrap());
                    let second = u64::from_le_bytes(ix.data[9..17].try_into().unwrap());
                    let input = matches!(ix.data[0], 9 | 16);
                    let expected_name = match ix.data[0] {
                        9 => "swap_base_in",
                        11 => "swap_base_out",
                        16 => "swap_base_in_v2",
                        17 => "swap_base_out_v2",
                        _ => unreachable!(),
                    };
                    let valid = match actual.first() {
                        Some(sol_shred_sdk::DexEvent::RaydiumAmmV4Swap(e)) => {
                            e.ix_name == expected_name
                                && (e.amount_in, e.amount_out) == (0, 0)
                                && (e.instruction_amount_in, e.instruction_amount_out)
                                    == if input { (first, 0) } else { (0, second) }
                                && (e.minimum_amount_out, e.max_amount_in)
                                    == if input { (second, 0) } else { (0, first) }
                        }
                        _ => false,
                    };
                    if !valid {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: AMM swap quantities differ from wire"
                        ));
                    }
                }
                if file == "raydium_cpmm.json"
                    && matches!(
                        name,
                        "collect_creator_fee" | "collect_creator_fee_permissionless"
                    )
                {
                    let ix = &transaction.message.instructions()[0];
                    let keys = transaction.message.static_account_keys();
                    let account = |i: usize| keys[ix.accounts[i] as usize];
                    let permissionless = name.ends_with("permissionless");
                    let valid = match actual.first() {
                        Some(sol_shred_sdk::DexEvent::RaydiumCpmmCollectCreatorFee(e)) => {
                            e.permissionless == permissionless
                                && e.creator == account(usize::from(permissionless))
                                && e.payer == account(0)
                                && e.authority == account(if permissionless { 2 } else { 1 })
                                && e.pool_state == account(if permissionless { 3 } else { 2 })
                                && e.amm_config == account(if permissionless { 14 } else { 3 })
                                && e.creator_fee_share
                                    == account(if permissionless { 15 } else { 14 })
                                && (
                                    e.token_0_vault,
                                    e.token_1_vault,
                                    e.vault_0_mint,
                                    e.vault_1_mint,
                                ) == (account(4), account(5), account(6), account(7))
                                && (
                                    e.creator_token_0,
                                    e.creator_token_1,
                                    e.token_0_program,
                                    e.token_1_program,
                                ) == (account(8), account(9), account(10), account(11))
                                && (e.associated_token_program, e.system_program)
                                    == (account(12), account(13))
                        }
                        _ => false,
                    };
                    if !valid {
                        differences.push(format!("{file}/{name}/case{seed}: creator collection accounts differ from upgrade"));
                    }
                }
                if file == "orca_whirlpool.json" {
                    let ix = &transaction.message.instructions()[0];
                    let keys = transaction.message.static_account_keys();
                    let account = |index: usize| keys[ix.accounts[index] as usize];
                    let valid = match actual.first() {
                        Some(sol_shred_sdk::DexEvent::OrcaWhirlpoolSwap(e)) => {
                            let v2 = name == "swap_v2";
                            e.ix_name == name
                                && e.amount
                                    == u64::from_le_bytes(ix.data[8..16].try_into().unwrap())
                                && e.other_amount_threshold
                                    == u64::from_le_bytes(ix.data[16..24].try_into().unwrap())
                                && e.sqrt_price_limit
                                    == u128::from_le_bytes(ix.data[24..40].try_into().unwrap())
                                && e.amount_specified_is_input == (ix.data[40] == 1)
                                && e.a_to_b == (ix.data[41] == 1)
                                && (
                                    e.input_amount,
                                    e.output_amount,
                                    e.pre_sqrt_price,
                                    e.post_sqrt_price,
                                ) == (0, 0, 0, 0)
                                && e.whirlpool == account(if v2 { 4 } else { 2 })
                                && e.token_authority == account(if v2 { 3 } else { 1 })
                                && e.token_owner_account_a == account(if v2 { 7 } else { 3 })
                                && e.token_owner_account_b == account(if v2 { 9 } else { 5 })
                        }
                        Some(sol_shred_sdk::DexEvent::OrcaWhirlpoolPoolInitialized(e)) => {
                            let v2 = name == "initialize_pool_v2";
                            let offset = if v2 { 8 } else { 9 };
                            let spacing =
                                u16::from_le_bytes(ix.data[offset..offset + 2].try_into().unwrap());
                            let price = u128::from_le_bytes(
                                ix.data[offset + 2..offset + 18].try_into().unwrap(),
                            );
                            e.whirlpool == account(if v2 { 6 } else { 4 })
                                && e.whirlpools_config == account(0)
                                && (e.token_mint_a, e.token_mint_b) == (account(1), account(2))
                                && (e.tick_spacing, e.initial_sqrt_price) == (spacing, price)
                                && (e.token_program_a, e.token_program_b)
                                    == (
                                        account(if v2 { 10 } else { 8 }),
                                        account(if v2 { 11 } else { 8 }),
                                    )
                        }
                        Some(sol_shred_sdk::DexEvent::OrcaWhirlpoolLiquidityIncreased(e)) => {
                            e.whirlpool == account(0)
                                && e.position == account(if name.ends_with("_v2") { 5 } else { 3 })
                        }
                        Some(sol_shred_sdk::DexEvent::OrcaWhirlpoolLiquidityDecreased(e)) => {
                            e.whirlpool == account(0)
                                && e.position == account(if name.ends_with("_v2") { 5 } else { 3 })
                        }
                        _ => true,
                    };
                    if !valid {
                        differences.push(format!(
                            "{file}/{name}/case{seed}: Orca layout differs from IDL"
                        ));
                    }
                }
                let mut alt_tx = transaction.clone();
                let VersionedMessage::V0(message) = &mut alt_tx.message else {
                    unreachable!()
                };
                let readonly = message.account_keys[n..].to_vec();
                let writable = message.account_keys[1..n].to_vec();
                message.account_keys.truncate(1);
                message
                    .address_table_lookups
                    .push(v0::MessageAddressTableLookup {
                        account_key: Pubkey::from([99; 32]),
                        writable_indexes: (0..writable.len() as u8).collect(),
                        readonly_indexes: (0..readonly.len() as u8).collect(),
                    });
                let mut alt_events = vec![];
                sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
                    &alt_tx,
                    &writable,
                    &readonly,
                    Signature::default(),
                    42,
                    3,
                    0,
                    None,
                    &mut alt_events,
                )
                .unwrap();
                if normalize(serde_json::to_value(&actual).unwrap())
                    != normalize(serde_json::to_value(&alt_events).unwrap())
                {
                    differences.push(format!(
                        "{file}/{name}/case{seed}: resolved ALT differs from static parsing"
                    ));
                }
                check_generated(
                    &alt_tx,
                    &format!("{file}/{name}/case{seed}/unresolved-ALT"),
                    &mut differences,
                );
                let mut missing_account = transaction.clone();
                let VersionedMessage::V0(message) = &mut missing_account.message else {
                    unreachable!()
                };
                if let Some(account) = message.instructions[0].accounts.first_mut() {
                    *account = 255;
                }
                check_generated(
                    &missing_account,
                    &format!("{file}/{name}/case{seed}/unresolved-account"),
                    &mut differences,
                );
                compared_transactions += 2;
                for (label, reference_filter, shred_filter) in [
                    (
                        "trade",
                        sol_parser_sdk::grpc::types::EventTypeFilter::include_only(vec![
                            sol_parser_sdk::grpc::types::EventType::PumpFunTrade,
                        ]),
                        sol_shred_sdk::EventTypeFilter::include_only(vec![
                            sol_shred_sdk::EventType::PumpFunTrade,
                        ]),
                    ),
                    (
                        "buy",
                        sol_parser_sdk::grpc::types::EventTypeFilter::include_only(vec![
                            sol_parser_sdk::grpc::types::EventType::PumpFunBuy,
                        ]),
                        sol_shred_sdk::EventTypeFilter::include_only(vec![
                            sol_shred_sdk::EventType::PumpFunBuy,
                        ]),
                    ),
                    (
                        "swap",
                        sol_parser_sdk::grpc::types::EventTypeFilter::include_only(vec![
                            sol_parser_sdk::grpc::types::EventType::MeteoraDlmmSwap,
                            sol_parser_sdk::grpc::types::EventType::RaydiumCpmmSwap,
                        ]),
                        sol_shred_sdk::EventTypeFilter::include_only(vec![
                            sol_shred_sdk::EventType::MeteoraDlmmSwap,
                            sol_shred_sdk::EventType::RaydiumCpmmSwap,
                        ]),
                    ),
                ] {
                    let filtered_expected =
                        sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
                            &TransactionStatusMeta::default(),
                            &grpc_tx,
                            Default::default(),
                            42,
                            3,
                            None,
                            0,
                            Some(&reference_filter),
                        )
                        .into_iter()
                        .map(|e| reference_filter.normalize_dex_event(e))
                        .collect::<Vec<_>>();
                    let mut filtered_actual = vec![];
                    sol_shred_sdk::parse_transaction_dex_events_with_filter(
                        &transaction,
                        Signature::default(),
                        42,
                        3,
                        0,
                        Some(&shred_filter),
                        &mut filtered_actual,
                    );
                    if normalize(serde_json::to_value(filtered_expected).unwrap())
                        != normalize(serde_json::to_value(filtered_actual).unwrap())
                    {
                        differences
                            .push(format!("{file}/{name}/case{seed}: {label} filter differs"));
                    }
                }
                // Two independent invocations must keep their own account context.
                let mut multi_tx = transaction.clone();
                let VersionedMessage::V0(message) = &mut multi_tx.message else {
                    unreachable!()
                };
                let base = message.account_keys.len();
                let shifted: Vec<_> = keys[..n]
                    .iter()
                    .enumerate()
                    .map(|(i, key)| {
                        if layout[i]["address"].is_string() || layout[i]["optional"] == true {
                            return *key;
                        }
                        let mut bytes = key.to_bytes();
                        bytes[31] = bytes[31].wrapping_add(17);
                        Pubkey::from(bytes)
                    })
                    .collect();
                message.account_keys.extend(shifted.iter().copied());
                let mut extra = message.instructions[0].clone();
                extra.accounts = (base..base + n).map(|i| i as u8).collect();
                message.instructions.push(extra.clone());
                let mut multi_grpc = grpc_tx.clone();
                let message = multi_grpc.as_mut().unwrap().message.as_mut().unwrap();
                message
                    .account_keys
                    .extend(shifted.iter().map(|k| k.to_bytes().to_vec()));
                message
                    .instructions
                    .push(yellowstone_grpc_proto::prelude::CompiledInstruction {
                        program_id_index: extra.program_id_index as u32,
                        accounts: extra.accounts,
                        data: extra.data,
                    });
                let multi_expected =
                    sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
                        &TransactionStatusMeta::default(),
                        &multi_grpc,
                        Default::default(),
                        42,
                        3,
                        None,
                        0,
                        None,
                    );
                let mut multi_actual = vec![];
                sol_shred_sdk::parse_transaction_dex_events(
                    &multi_tx,
                    Signature::default(),
                    42,
                    3,
                    0,
                    &mut multi_actual,
                );
                let mut fields = vec![];
                diff(
                    &normalize(serde_json::to_value(multi_expected).unwrap()),
                    &normalize(serde_json::to_value(multi_actual).unwrap()),
                    String::new(),
                    &mut fields,
                );
                if !fields.is_empty() {
                    differences.push(format!(
                        "{file}/{name}/case{seed}: multiple invocations: {}",
                        fields.join("; ")
                    ));
                }
                if file == "pump.json" && name.starts_with("buy") {
                    for create_name in ["create", "create_v2"] {
                        let create_ix = idl["instructions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|ix| ix["name"] == create_name)
                            .unwrap();
                        let mut data: Vec<_> = create_ix["discriminator"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_u64().unwrap() as u8)
                            .collect();
                        for arg in create_ix["args"].as_array().unwrap() {
                            encode_type(&arg["type"], &idl, seed, &mut data);
                        }
                        let mut create_tx = multi_tx.clone();
                        let VersionedMessage::V0(message) = &mut create_tx.message else {
                            unreachable!()
                        };
                        let base = message.account_keys.len();
                        let mint_index = if name.ends_with("v2") { 1 } else { 2 };
                        let create_keys: Vec<Pubkey> = create_ix["accounts"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .enumerate()
                            .map(|(i, a)| {
                                if i == 0 {
                                    keys[mint_index]
                                } else if let Some(address) = a["address"].as_str() {
                                    address.parse().unwrap()
                                } else {
                                    Pubkey::from([(i + 50) as u8; 32])
                                }
                            })
                            .collect();
                        message.account_keys.extend(create_keys.iter().copied());
                        message
                            .instructions
                            .push(CompiledInstruction::new_from_raw_parts(
                                n as u8,
                                data.clone(),
                                (base..base + create_keys.len()).map(|i| i as u8).collect(),
                            ));
                        let mut create_grpc = multi_grpc.clone();
                        let message = create_grpc.as_mut().unwrap().message.as_mut().unwrap();
                        message
                            .account_keys
                            .extend(create_keys.iter().map(|k| k.to_bytes().to_vec()));
                        message.instructions.push(
                            yellowstone_grpc_proto::prelude::CompiledInstruction {
                                program_id_index: n as u32,
                                data,
                                accounts: (base..base + create_keys.len())
                                    .map(|i| i as u8)
                                    .collect(),
                            },
                        );
                        // A create discriminator from an unresolved program must not
                        // mark buys, while a known create's visible mint remains usable
                        // even if its other accounts are not yet resolved.
                        for unknown_program in [false, true] {
                            let mut unresolved_create = create_tx.clone();
                            let VersionedMessage::V0(message) = &mut unresolved_create.message
                            else {
                                unreachable!()
                            };
                            let lookup_index = message.account_keys.len() as u8;
                            let create = message.instructions.last_mut().unwrap();
                            if unknown_program {
                                create.program_id_index = lookup_index;
                            } else {
                                create.accounts[1] = lookup_index;
                            }
                            message
                                .address_table_lookups
                                .push(v0::MessageAddressTableLookup {
                                    account_key: Pubkey::from([88; 32]),
                                    writable_indexes: vec![],
                                    readonly_indexes: vec![0],
                                });
                            check_generated(&unresolved_create, &format!("{name}+{create_name}/case{seed}/unresolved-create/unknown-program{unknown_program}"), &mut differences);
                            compared_transactions += 1;
                        }
                        for only_buy in [false, true] {
                            compared_transactions += 1;
                            let reference_filter =
                                sol_parser_sdk::grpc::types::EventTypeFilter::include_only(vec![
                                    sol_parser_sdk::grpc::types::EventType::PumpFunTrade,
                                ]);
                            let shred_filter = sol_shred_sdk::EventTypeFilter::include_only(vec![
                                sol_shred_sdk::EventType::PumpFunTrade,
                            ]);
                            let expected = sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
                                &TransactionStatusMeta::default(), &create_grpc, Default::default(), 42, 3, None, 0,
                                only_buy.then_some(&reference_filter),
                            ).into_iter().map(|e| if only_buy { reference_filter.normalize_dex_event(e) } else { e }).collect::<Vec<_>>();
                            let mut actual = vec![];
                            sol_shred_sdk::parse_transaction_dex_events_with_filter(
                                &create_tx,
                                Signature::default(),
                                42,
                                3,
                                0,
                                only_buy.then_some(&shred_filter),
                                &mut actual,
                            );
                            let mut fields = vec![];
                            diff(
                                &normalize(serde_json::to_value(expected).unwrap()),
                                &normalize(serde_json::to_value(actual).unwrap()),
                                String::new(),
                                &mut fields,
                            );
                            if !fields.is_empty() {
                                differences.push(format!(
                                    "{name}+{create_name}/case{seed}/filtered{only_buy}: {}",
                                    fields.join("; ")
                                ));
                            }
                        }
                    }
                }
                // Incomplete wire payloads and older/shorter account layouts must
                // agree with the reference without panicking or inventing fields.
                let first = &transaction.message.instructions()[0];
                for length in 0..first.data.len() {
                    let mut variant = transaction.clone();
                    let VersionedMessage::V0(message) = &mut variant.message else {
                        unreachable!()
                    };
                    message.instructions[0].data.truncate(length);
                    check_generated(
                        &variant,
                        &format!("{file}/{name}/case{seed}/data-prefix{length}"),
                        &mut differences,
                    );
                    compared_transactions += 1;
                }
                for length in 0..first.accounts.len() {
                    let mut variant = transaction.clone();
                    let VersionedMessage::V0(message) = &mut variant.message else {
                        unreachable!()
                    };
                    message.instructions[0].accounts.truncate(length);
                    check_generated(
                        &variant,
                        &format!("{file}/{name}/case{seed}/account-prefix{length}"),
                        &mut differences,
                    );
                    compared_transactions += 1;
                }
                // Exercise present optional accounts and remaining-account extensions.
                for tail_count in [0usize, 1, 2, 3, 4, 8, 24, 64] {
                    let mut variant = transaction.clone();
                    let VersionedMessage::V0(message) = &mut variant.message else {
                        unreachable!()
                    };
                    for (i, account) in layout.iter().enumerate() {
                        if account["optional"] == true {
                            message.account_keys[i] = Pubkey::from([(i + 70) as u8; 32]);
                        }
                    }
                    for i in 0..tail_count {
                        let index = message.account_keys.len();
                        message
                            .account_keys
                            .push(Pubkey::from([(i + 130) as u8; 32]));
                        message.instructions[0].accounts.push(index as u8);
                    }
                    check_generated(
                        &variant,
                        &format!("{file}/{name}/case{seed}/optional-present+tail{tail_count}"),
                        &mut differences,
                    );
                    if file == "raydium_clmm.json" && matches!(name, "swap" | "swap_v2") {
                        let message = match &variant.message {
                            VersionedMessage::V0(m) => m,
                            _ => unreachable!(),
                        };
                        let ix = &message.instructions[0];
                        let start = if name == "swap_v2" { 13 } else { 9 };
                        let expected_ticks = ix
                            .accounts
                            .iter()
                            .skip(start)
                            .map(|index| message.account_keys[*index as usize])
                            .collect::<Vec<_>>();
                        let mut actual = Vec::new();
                        sol_shred_sdk::parse_transaction_dex_events(
                            &variant,
                            Signature::default(),
                            42,
                            3,
                            777,
                            &mut actual,
                        );
                        if !matches!(actual.first(), Some(sol_shred_sdk::DexEvent::RaydiumClmmSwap(e)) if e.tick_arrays == expected_ticks)
                        {
                            differences.push(format!("{file}/{name}/case{seed}/tail{tail_count}: CLMM tail truncated or reordered"));
                        }
                    }
                    if file == "raydium_clmm.json"
                        && matches!(name, "swap" | "swap_v2")
                        && tail_count >= 24
                    {
                        let mut alt = variant.clone();
                        let VersionedMessage::V0(message) = &mut alt.message else {
                            unreachable!()
                        };
                        let loaded = message.account_keys.split_off(1);
                        message.address_table_lookups = vec![v0::MessageAddressTableLookup {
                            account_key: Pubkey::from([99; 32]),
                            writable_indexes: vec![],
                            readonly_indexes: (0..loaded.len() as u8).collect(),
                        }];
                        let mut resolved = Vec::new();
                        sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
                            &alt,
                            &[],
                            &loaded,
                            Signature::default(),
                            42,
                            3,
                            777,
                            None,
                            &mut resolved,
                        )
                        .unwrap();
                        let mut static_events = Vec::new();
                        sol_shred_sdk::parse_transaction_dex_events(
                            &variant,
                            Signature::default(),
                            42,
                            3,
                            777,
                            &mut static_events,
                        );
                        if serde_json::to_value(resolved).unwrap()
                            != serde_json::to_value(static_events).unwrap()
                        {
                            differences.push(format!("{file}/{name}/case{seed}/tail{tail_count}: resolved ALT loses long tail"));
                        }
                        compared_transactions += 1;
                    }
                    compared_transactions += 1;
                }
                // Same pool/mint, different users: account context must stay per invocation.
                let mut same_pool = multi_tx.clone();
                let VersionedMessage::V0(message) = &mut same_pool.message else {
                    unreachable!()
                };
                let second = message.instructions.last().unwrap().accounts.clone();
                for (i, account) in layout.iter().enumerate() {
                    let name = account["name"].as_str().unwrap_or("");
                    if matches!(
                        name,
                        "pool"
                            | "pool_state"
                            | "poolState"
                            | "lb_pair"
                            | "lbPair"
                            | "mint"
                            | "base_mint"
                            | "baseMint"
                    ) {
                        message.account_keys[second[i] as usize] = keys[i];
                    }
                }
                check_generated(
                    &same_pool,
                    &format!("{file}/{name}/case{seed}/same-pool-different-user"),
                    &mut differences,
                );
                compared_transactions += 1;
                let expected = normalize(serde_json::to_value(expected).unwrap());
                let actual = normalize(serde_json::to_value(actual).unwrap());
                if expected != actual {
                    let mut fields = vec![];
                    diff(&expected, &actual, String::new(), &mut fields);
                    differences.push(format!("{file}/{name}/case{seed}: {}", fields.join("; ")));
                }
            }
        }
    }
    for (i, (label, first)) in templates.iter().enumerate() {
        let (other, second) = &templates[(i + 1) % templates.len()];
        let mut mixed = first.clone();
        let VersionedMessage::V0(message) = &mut mixed.message else {
            unreachable!()
        };
        let remap: Vec<u8> = second
            .message
            .static_account_keys()
            .iter()
            .map(|key| {
                if let Some(index) = message
                    .account_keys
                    .iter()
                    .position(|candidate| candidate == key)
                {
                    index as u8
                } else {
                    let index = message.account_keys.len();
                    assert!(index < 256);
                    message.account_keys.push(*key);
                    index as u8
                }
            })
            .collect();
        for ix in second.message.instructions() {
            let mut ix = ix.clone();
            ix.program_id_index = remap[ix.program_id_index as usize];
            ix.accounts = ix
                .accounts
                .iter()
                .map(|index| remap[*index as usize])
                .collect();
            message.instructions.push(ix);
        }
        check_generated(&mixed, &format!("mixed {label}+{other}"), &mut differences);
        compared_transactions += 1;
        compared_transactions +=
            check_all_filters(&mixed, &format!("mixed {label}+{other}"), &mut differences);
    }
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sol-parser-sdk/tests/fixtures");
    let mut fixture_count = 0;
    let mut fixture_events = 0;
    let update = yellowstone_grpc_proto::prelude::SubscribeUpdateTransaction::decode(
        fs::read(fixtures.join("pumpfun_yellowstone_transaction.bin"))
            .unwrap()
            .as_slice(),
    )
    .unwrap();
    let info = update.transaction.unwrap();
    let grpc_tx = info.transaction.unwrap();
    let source_meta = info.meta.unwrap();
    let msg = grpc_tx.message.as_ref().unwrap();
    let meta = TransactionStatusMeta {
        loaded_writable_addresses: source_meta.loaded_writable_addresses,
        loaded_readonly_addresses: source_meta.loaded_readonly_addresses,
        ..Default::default()
    };
    let pubkey = |bytes: &[u8]| Pubkey::from(<[u8; 32]>::try_from(bytes).unwrap());
    let signature = Signature::try_from(info.signature.as_slice()).unwrap();
    let header = msg.header.as_ref().unwrap();
    let raw = VersionedTransaction {
        signatures: vec![signature],
        message: VersionedMessage::V0(v0::Message {
            header: MessageHeader {
                num_required_signatures: header.num_required_signatures as u8,
                num_readonly_signed_accounts: header.num_readonly_signed_accounts as u8,
                num_readonly_unsigned_accounts: header.num_readonly_unsigned_accounts as u8,
            },
            account_keys: msg.account_keys.iter().map(|k| pubkey(k)).collect(),
            recent_blockhash: Hash::new_from_array(
                msg.recent_blockhash.as_slice().try_into().unwrap(),
            ),
            instructions: msg
                .instructions
                .iter()
                .map(|ix| {
                    CompiledInstruction::new_from_raw_parts(
                        ix.program_id_index as u8,
                        ix.data.clone(),
                        ix.accounts.clone(),
                    )
                })
                .collect(),
            address_table_lookups: msg
                .address_table_lookups
                .iter()
                .map(|l| v0::MessageAddressTableLookup {
                    account_key: pubkey(&l.account_key),
                    writable_indexes: l.writable_indexes.clone(),
                    readonly_indexes: l.readonly_indexes.clone(),
                })
                .collect(),
        }),
    };
    let expected = sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
        &meta,
        &Some(grpc_tx),
        signature.as_ref().try_into().expect("signature bytes"),
        update.slot,
        info.index,
        None,
        0,
        None,
    );
    let mut actual = vec![];
    let writable: Vec<_> = meta
        .loaded_writable_addresses
        .iter()
        .map(|k| pubkey(k))
        .collect();
    let readonly: Vec<_> = meta
        .loaded_readonly_addresses
        .iter()
        .map(|k| pubkey(k))
        .collect();
    sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
        &raw,
        &writable,
        &readonly,
        signature,
        update.slot,
        info.index,
        0,
        None,
        &mut actual,
    )
    .unwrap();
    fixture_count += 1;
    fixture_events += expected.len();
    let mut fields = vec![];
    diff(
        &normalize(serde_json::to_value(expected).unwrap()),
        &normalize(serde_json::to_value(actual).unwrap()),
        String::new(),
        &mut fields,
    );
    if !fields.is_empty() {
        differences.push(format!(
            "pumpfun_yellowstone_transaction.bin: {}",
            fields.join("; ")
        ));
    }
    for entry in fs::read_dir(&fixtures).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let json: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let rpc: solana_transaction_status::EncodedConfirmedTransactionWithStatusMeta =
            serde_json::from_value(json.clone()).unwrap();
        let (source_meta, grpc_tx) = sol_parser_sdk::convert_rpc_to_grpc(&rpc).unwrap();
        let meta = TransactionStatusMeta {
            loaded_writable_addresses: source_meta.loaded_writable_addresses,
            loaded_readonly_addresses: source_meta.loaded_readonly_addresses,
            ..Default::default()
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(json["transaction"][0].as_str().unwrap())
            .unwrap();
        let tx: VersionedTransaction = wincode::deserialize(&bytes).unwrap();
        let writable: Vec<Pubkey> = meta
            .loaded_writable_addresses
            .iter()
            .map(|b| Pubkey::from(<[u8; 32]>::try_from(b.as_slice()).unwrap()))
            .collect();
        let readonly: Vec<Pubkey> = meta
            .loaded_readonly_addresses
            .iter()
            .map(|b| Pubkey::from(<[u8; 32]>::try_from(b.as_slice()).unwrap()))
            .collect();
        let signature = tx.signatures[0];
        let expected = sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
            &meta,
            &Some(grpc_tx),
            signature.as_ref().try_into().expect("signature bytes"),
            rpc.slot,
            0,
            None,
            0,
            None,
        );
        let mut actual = vec![];
        sol_shred_sdk::parse_transaction_dex_events_with_loaded_addresses(
            &tx,
            &writable,
            &readonly,
            signature,
            rpc.slot,
            0,
            0,
            None,
            &mut actual,
        )
        .unwrap();
        fixture_count += 1;
        fixture_events += expected.len();
        let mut fields = vec![];
        diff(
            &normalize(serde_json::to_value(expected).unwrap()),
            &normalize(serde_json::to_value(actual).unwrap()),
            String::new(),
            &mut fields,
        );
        if !fields.is_empty() {
            differences.push(format!(
                "{}: {}",
                path.file_name().unwrap().to_string_lossy(),
                fields.join("; ")
            ));
        }
    }
    println!("Compared {fixture_count} saved mainnet transactions, {fixture_events} outer events (with resolved ALTs)");
    println!("Compared {compared_transactions} generated transaction/filter scenarios; {unsupported} unsupported IDL cases checked for unexpected shred events");
    println!(
        "Compared {compared} supported outer instructions; {} mismatches",
        differences.len()
    );
    for d in &differences {
        println!("{d}");
    }
    if !differences.is_empty() {
        std::process::exit(1);
    }
}

fn generated_grpc(tx: &VersionedTransaction) -> Option<Transaction> {
    Some(Transaction {
        message: Some(Message {
            account_keys: tx
                .message
                .static_account_keys()
                .iter()
                .map(|k| k.to_bytes().to_vec())
                .collect(),
            recent_blockhash: tx.message.recent_blockhash().to_bytes().to_vec(),
            instructions: tx
                .message
                .instructions()
                .iter()
                .map(|ix| yellowstone_grpc_proto::prelude::CompiledInstruction {
                    program_id_index: ix.program_id_index as u32,
                    accounts: ix.accounts.clone(),
                    data: ix.data.clone(),
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    })
}

fn check_all_filters(
    tx: &VersionedTransaction,
    label: &str,
    differences: &mut Vec<String>,
) -> usize {
    let grpc = generated_grpc(tx);
    let pairs = filters::pairs();
    for (reference_type, shred_type) in &pairs {
        for exclude in [false, true] {
            let reference = if exclude {
                sol_parser_sdk::grpc::types::EventTypeFilter::exclude_types(vec![*reference_type])
            } else {
                sol_parser_sdk::grpc::types::EventTypeFilter::include_only(vec![*reference_type])
            };
            let shred = if exclude {
                sol_shred_sdk::EventTypeFilter::exclude_types(vec![*shred_type])
            } else {
                sol_shred_sdk::EventTypeFilter::include_only(vec![*shred_type])
            };
            let expected = sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
                &TransactionStatusMeta::default(),
                &grpc,
                Default::default(),
                42,
                3,
                None,
                777,
                Some(&reference),
            )
            .into_iter()
            .map(|event| reference.normalize_dex_event(event))
            .collect::<Vec<_>>();
            let mut actual = vec![];
            sol_shred_sdk::parse_transaction_dex_events_with_filter(
                tx,
                Signature::default(),
                42,
                3,
                777,
                Some(&shred),
                &mut actual,
            );
            let mut fields = vec![];
            diff(
                &normalize(serde_json::to_value(expected).unwrap()),
                &normalize(serde_json::to_value(actual).unwrap()),
                String::new(),
                &mut fields,
            );
            if !fields.is_empty() {
                differences.push(format!(
                    "{label}/filter{shred_type:?}/exclude{exclude}: {}",
                    fields.join("; ")
                ));
            }
        }
    }
    pairs.len() * 2
}

fn check_generated(tx: &VersionedTransaction, label: &str, differences: &mut Vec<String>) {
    let grpc = generated_grpc(tx);
    let expected = sol_parser_sdk::grpc::instruction_parser::parse_instructions_enhanced(
        &TransactionStatusMeta::default(),
        &grpc,
        Default::default(),
        42,
        3,
        None,
        777,
        None,
    );
    let mut actual = vec![];
    sol_shred_sdk::parse_transaction_dex_events(tx, Signature::default(), 42, 3, 777, &mut actual);
    for event in &actual {
        let meta = event.metadata();
        if meta.grpc_recv_us != 777 {
            differences.push(format!("{label}: supplied receive timestamp was lost"));
        }
    }
    let mut fields = vec![];
    diff(
        &normalize(serde_json::to_value(expected).unwrap()),
        &normalize(serde_json::to_value(actual).unwrap()),
        String::new(),
        &mut fields,
    );
    if !fields.is_empty() {
        differences.push(format!("{label}: {}", fields.join("; ")));
    }
}

fn normalize(mut v: Value) -> Value {
    if let Value::Array(events) = &mut v {
        for e in events {
            if let Some(obj) = e.as_object_mut() {
                for body in obj.values_mut() {
                    if let Some(fields) = body.as_object_mut() {
                        // Execution balance snapshots are unavailable in raw shreds.
                        fields.remove("sol_balance");
                        fields.remove("token_balance");
                    }
                }
            }
        }
    }
    v
}
fn diff(a: &Value, b: &Value, path: String, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                match b.get(k) {
                    Some(other) => diff(v, other, format!("{path}.{k}"), out),
                    None => out.push(format!("{path}.{k}: missing from shred")),
                }
            }
            for k in b.keys().filter(|k| !a.contains_key(*k)) {
                out.push(format!("{path}.{k}: shred-only"));
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                diff(a, b, format!("{path}[{i}]"), out);
            }
        }
        _ if a != b => out.push(format!("{path}: grpc={a} shred={b}")),
        _ => {}
    }
}

#[cfg(test)]
mod comparator_tests {
    use super::diff;
    use serde_json::{json, Value};

    fn differences(expected: Value, actual: Value) -> Vec<String> {
        let mut out = vec![];
        diff(&expected, &actual, String::new(), &mut out);
        out
    }

    #[test]
    fn nullable_fields_require_matching_presence_in_both_directions() {
        assert_eq!(
            differences(json!({"recent_blockhash": null}), json!({})),
            vec![".recent_blockhash: missing from shred"]
        );
        assert_eq!(
            differences(json!({}), json!({"recent_blockhash": null})),
            vec![".recent_blockhash: shred-only"]
        );
        assert!(differences(
            json!({"recent_blockhash": null}),
            json!({"recent_blockhash": null})
        )
        .is_empty());
    }

    #[test]
    fn nested_fields_extra_keys_and_arrays_remain_observable() {
        assert_eq!(
            differences(
                json!([{ "PumpFunBuy": { "metadata": { "recent_blockhash": null } } }]),
                json!([{ "PumpFunBuy": { "metadata": {} } }])
            ),
            vec!["[0].PumpFunBuy.metadata.recent_blockhash: missing from shred"]
        );
        assert_eq!(
            differences(json!({"amount": 1}), json!({"amount": 1, "extra": 2})),
            vec![".extra: shred-only"]
        );
        assert!(!differences(json!([1]), json!([1, 2])).is_empty());
        assert!(!differences(json!([1, 2]), json!([2, 1])).is_empty());
    }
}

// Generate valid Borsh argument layouts directly from the reference IDLs, using
// both zero and nonzero values so equality checks exercise parameter mapping.
fn encode_type(ty: &Value, idl: &Value, seed: u8, out: &mut Vec<u8>) {
    if let Some(name) = ty.as_str() {
        match name {
            "bool" => out.push(seed % 2),
            "u8" | "i8" => out.push(seed),
            "u16" | "i16" => out.extend_from_slice(&(123u16 * seed as u16).to_le_bytes()),
            "u32" | "i32" => out.extend_from_slice(&(123u32 * seed as u32).to_le_bytes()),
            "u64" | "i64" => out.extend_from_slice(
                &(if seed == 255 {
                    u64::MAX
                } else {
                    123u64 * seed as u64
                })
                .to_le_bytes(),
            ),
            "u128" => out.extend_from_slice(
                &(if seed == 255 {
                    u128::MAX
                } else {
                    123u128 * seed as u128
                })
                .to_le_bytes(),
            ),
            "i128" => out.extend_from_slice(&(123i128 * seed as i128).to_le_bytes()),
            "u256" | "i256" => {
                out.extend_from_slice(&(123u128 * seed as u128).to_le_bytes());
                out.extend_from_slice(&[0; 16]);
            }
            "f32" => out.extend_from_slice(&(seed as f32).to_le_bytes()),
            "f64" => out.extend_from_slice(&(seed as f64).to_le_bytes()),
            "pubkey" | "publicKey" => out.extend_from_slice(&[seed.wrapping_mul(42); 32]),
            "string" | "bytes" => {
                let data = if seed == 0 { "" } else { "parity-test" };
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(data.as_bytes());
            }
            _ => panic!("unsupported primitive {name}"),
        }
    } else if let Some(inner) = ty.get("option") {
        out.push(u8::from(seed != 0));
        if seed != 0 {
            encode_type(inner, idl, seed, out);
        }
    } else if let Some(inner) = ty.get("vec") {
        let count = seed % 3;
        out.extend_from_slice(&(count as u32).to_le_bytes());
        for _ in 0..count {
            encode_type(inner, idl, seed, out);
        }
    } else if let Some(array) = ty.get("array").and_then(Value::as_array) {
        for _ in 0..array[1].as_u64().unwrap() {
            encode_type(&array[0], idl, seed, out);
        }
    } else if let Some(defined) = ty.get("defined") {
        let name = defined
            .as_str()
            .or_else(|| defined["name"].as_str())
            .unwrap();
        let definition = idl["types"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("missing type {name}"));
        match definition["type"]["kind"].as_str().unwrap() {
            "struct" => {
                for field in definition["type"]["fields"].as_array().unwrap() {
                    encode_type(field.get("type").unwrap_or(field), idl, seed, out);
                }
            }
            "enum" => {
                let variants = definition["type"]["variants"].as_array().unwrap();
                let index = seed as usize % variants.len();
                out.push(index as u8);
                if let Some(fields) = variants[index]["fields"].as_array() {
                    for field in fields {
                        encode_type(field.get("type").unwrap_or(field), idl, seed, out);
                    }
                }
            }
            "alias" => encode_type(&definition["type"]["value"], idl, seed, out),
            kind => panic!("unsupported type {kind}"),
        }
    } else {
        panic!("unsupported IDL type {ty}");
    }
}

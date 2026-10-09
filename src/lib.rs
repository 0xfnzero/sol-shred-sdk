// Parser entry points intentionally keep explicit context parameters for API compatibility.
#![allow(clippy::too_many_arguments)]

extern crate self as solana_sdk;

// Compatibility facade for the pre-4.0 monolithic SDK paths used internally.
// Every item is backed by the aligned modular Solana crates above.
pub use bs58;
pub use solana_pubkey::pubkey;
pub use solana_pubkey::Pubkey;

pub mod commitment_config {
    pub use solana_commitment_config::CommitmentConfig;
}

pub mod hash {
    pub use solana_hash::Hash;
    pub use solana_sha256_hasher::hashv;
}

pub mod instruction {
    pub use solana_message::compiled_instruction::CompiledInstruction;
}

pub mod message {
    pub use solana_message::{compiled_instruction, v0, MessageHeader, VersionedMessage};
}

pub mod pubkey {
    pub use solana_pubkey::{pubkey, Pubkey};
}

pub mod signature {
    pub use solana_signature::Signature;
}

pub mod transaction {
    pub use solana_transaction::versioned::VersionedTransaction;
}

pub mod accounts;
pub mod common;
pub mod constants;
pub mod core;
pub mod cpmm_creator_fee;
pub mod grpc;
pub mod instr;
pub mod logs;
pub mod parser;
pub mod shred;
pub mod shredstream;
pub mod stonkfun_registry;
pub mod transaction_route;

pub use common::logs_events::PumpfunEvent;
pub use common::AnyResult;
pub use core::{DexEvent, EventMetadata};
pub use grpc::types::{EventType, EventTypeFilter, Protocol};
#[allow(deprecated)]
pub use grpc::ShredStreamGrpc;
pub use parser::{PumpfunEventParser, PumpfunParserConfig};
pub use shred::{RawShredClient, RawShredConfig, RawShredDecoder, ShredEntryBatch, ShredTxBatch};
pub use shredstream::{
    parse_transaction_dex_events, parse_transaction_dex_events_best_effort,
    parse_transaction_dex_events_with_filter, parse_transaction_dex_events_with_loaded_addresses,
    AddressLookupResolver, AddressLookupStats, JitoShredStreamConfig, ShredDecodeMode,
    ShredStreamClient, ShredStreamConfig,
};
pub use stonkfun_registry::{StonkFunGraduatedPool, StonkFunPoolRegistry};
pub use transaction_route::{
    analyze_yellowstone_transaction_routes, InstructionPosition, NativeTokenAction,
    RouteNativeTokenAction, RouteSwapLeg, RouteTokenTransfer, RouteUnknownInvocation, SwapProtocol,
    TransactionRoute,
};

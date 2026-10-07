<div align="center">
    <h1>⚡ Sol Shred SDK</h1>
    <h3><em>低延迟 Solana 原始 Shred 解码与多 DEX 事件解析</em></h3>
</div>

<p align="center">
    <strong>高性能 Rust SDK，将 Solana UDP Shred 或 Jito 风格 ShredStream Entry 转换为强类型 DEX 事件，适用于交易机器人、索引服务和实时分析。</strong>
</p>

<p align="center">
    <a href="https://crates.io/crates/sol-shred-sdk"><img src="https://img.shields.io/crates/v/sol-shred-sdk.svg" alt="Crates.io"></a>
    <a href="https://docs.rs/sol-shred-sdk"><img src="https://docs.rs/sol-shred-sdk/badge.svg" alt="Documentation"></a>
    <a href="https://github.com/0xfnzero/sol-shred-sdk/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
    <a href="https://github.com/0xfnzero/sol-shred-sdk"><img src="https://img.shields.io/github/stars/0xfnzero/sol-shred-sdk?style=social" alt="GitHub stars"></a>
    <a href="https://github.com/0xfnzero/sol-shred-sdk/network"><img src="https://img.shields.io/github/forks/0xfnzero/sol-shred-sdk?style=social" alt="GitHub forks"></a>
</p>

<p align="center">
    <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
    <img src="https://img.shields.io/badge/Solana-9945FF?style=for-the-badge&logo=solana&logoColor=white" alt="Solana">
    <img src="https://img.shields.io/badge/ShredStream-FF6B6B?style=for-the-badge&logo=lightning&logoColor=white" alt="ShredStream">
    <img src="https://img.shields.io/badge/DEX-4B8BBE?style=for-the-badge&logo=bitcoin&logoColor=white" alt="DEX Events">
</p>

<p align="center">
    <a href="README_CN.md">中文</a> |
    <a href="README.md">English</a> |
    <a href="https://fnzero.dev/">官网</a> |
    <a href="https://t.me/fnzero_group">Telegram</a> |
    <a href="https://discord.gg/vuazbGkqQE">Discord</a>
</p>

`sol-shred-sdk` 是用于 Solana 原始 Shred 解码、ShredStream 接入和多 DEX 交易事件解析的低延迟 Rust SDK。

默认的最低延迟路径：

```text
UDP packet -> Solana/Agave Shred -> FEC recovery -> deshred -> Entry -> VersionedTransaction -> event parser
```

通过 `ShredDecodeMode` 选择数据源和解码模式。推荐使用原生 raw UDP shred 路径；在 Jito 服务可用期间，仍保留 Jito 风格 ShredStream gRPC Entry 作为兼容数据源。

## SDK 能力

| 方向 | 覆盖范围 |
|------|----------|
| 输入 | 原始 Solana UDP Shred payload，或 Jito 风格 ShredStream gRPC Entry |
| 解码 | `ShredDecodeMode::RawUdp` 使用 `solana-ledger` 解析 Shred、Reed-Solomon 恢复、`Shredder::deshred` 和 wincode `Vec<Entry>` 解码；`ShredDecodeMode::JitoGrpc` 接收预构建 Entry batch |
| 交易 | 带 slot 上下文的 Entry 到交易展开 |
| 事件 | 从 `sol-parser-sdk` ShredStream 路径迁移的 `DexEvent` 解析器 |
| 扩展 | 通过 `TransactionEventParser` trait 接入自定义解析器 |
| 兼容 | 保留旧版代码使用的部分生成 proto/type re-export |

支持的解析器系列与 `sol-parser-sdk` 保持一致：

- PumpFun、PumpFun v2/Mayhem、Pump Fees、PumpSwap
- Raydium LaunchLab、CPMM、CLMM、AMM V4
- Orca Whirlpool
- Meteora Pools、DAMM V2、DBC、DLMM
- Token account、Nonce account、部分 DEX account state 事件和 block metadata 类型

## 使用场景

- Solana 低延迟交易机器人和狙击机器人
- PumpFun、PumpSwap、Raydium、Orca、Meteora 实时事件流
- 不依赖 Jito ShredStream 的原始 UDP Shred 接入管道
- 迁移到 raw shred 期间的 Jito ShredStream gRPC 兼容
- 多 DEX 索引、分析和告警系统
- 复用统一 Shred/Entry 接入层的上层解析 SDK

Raw shred 订阅直接从 `Entry` 交易解析交易中可见的指令数据。SDK 为兼容性保留了仅日志和账户更新事件解析器，但 raw shred 本身不携带执行日志或账户更新 payload。

## 安装

### 直接克隆

将此项目克隆到您的项目目录：

```bash
cd your_project_root_directory
git clone https://github.com/0xfnzero/sol-shred-sdk
```

在您的 `Cargo.toml` 中添加依赖：

```toml
# 添加到您的 Cargo.toml
sol-shred-sdk = { path = "./sol-shred-sdk", version = "4.0.2" }
```

### 使用 crates.io

```toml
# 添加到您的 Cargo.toml
[dependencies]
sol-shred-sdk = "4.0.2"
```

PumpFun create/create_v2 账户布局与 RPC 回归验证：[PUMPFUN_CREATE_LAYOUT.md](docs/PUMPFUN_CREATE_LAYOUT.md)。

## PumpSwap 有效 Quote Reserves

PumpSwap Pool account 和 Buy/Sell 事件会暴露追加的带符号 `virtual_quote_reserves` 字段。报价和索引时使用：

```text
effective_quote_reserves = pool_quote_token_account.amount + virtual_quote_reserves
```

`virtual_quote_reserves` 可以为负值（官方文档标注 9 月 30 日启用）。解析、JSON 消费和索引存储均须保留符号，不能转为无符号整数或将负值截为零。买入和卖出都使用有效 quote 储备，Base reserve 仍使用原始 base vault 余额。旧版 Pool account 缺失该字段时解析为 `0`。

```rust
use sol_shred_sdk::accounts::pumpswap::effective_quote_reserves;

// 原始 quote vault 余额 1,000，加上带符号的虚拟储备 -500。
let quote_reserve = effective_quote_reserves(1_000, -500).expect("valid pool state");
assert_eq!(quote_reserve, 500);
// 买入和卖出报价均使用 quote_reserve。
```

该工具先在 `i128` 中相加，再转换为 `u64`。链上程序保证有效储备非负且不超过 `u64`；工具在相加结果为负或超过 `u64::MAX` 时返回 `None`。有效数值不能证明 Pool/vault 快照一致，快照一致性需由账户缓存维护。详见[官方负虚拟储备更新说明](https://github.com/pump-fun/pump-public-docs/blob/main/docs/NEGATIVE_VIRTUAL_QUOTE_RESERVES.md)。

原始 shreds 只包含外层交易指令，不包含执行日志、内部 CPI、Pool 状态或 vault 余额。因此外层 Buy/Sell 解析事件的储备及费率默认 `0` 表示未知，不能视为池的真实值。报价需要预先维护一致的 Pool/vault/手续费配置缓存；不要在交易热路径临时查询 RPC。`buy_exact_quote_in` 的 `min_base_amount_out` 是指令参数下限，不是实际成交量。外层买入解析已保留 `track_volume` 和 `ix_name`，并同步当前追加账户位置。

真实 RPC 示例及验证边界见 [PumpSwap RPC 文档](examples/PUMPSWAP_RPC.md)。

## 解码模式

创建客户端时选择数据源和解码器：

```rust
use sol_shred_sdk::{RawShredConfig, ShredDecodeMode, ShredStreamClient};

let raw = ShredStreamClient::new_with_decode_mode(
    ShredDecodeMode::raw_udp(RawShredConfig::default()),
).await?;

let jito = ShredStreamClient::new_with_decode_mode(
    ShredDecodeMode::jito_grpc("http://127.0.0.1:10000"),
).await?;
```

## Raw UDP 事件订阅

绑定接收 Solana raw shred datagram 的 UDP socket：

```rust
use sol_shred_sdk::shredstream::{ShredStreamClient, ShredStreamConfig};
use sol_shred_sdk::{DexEvent, EventType, EventTypeFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ShredStreamConfig::low_latency()
        .with_udp_bind("0.0.0.0:8001".parse()?);

    let client = ShredStreamClient::new_with_config(config).await?;
    client
        .subscribe_with_filter_callback(
            Some(EventTypeFilter::include_only(vec![
                EventType::PumpFunCreate,
                EventType::RaydiumCpmmSwap,
                EventType::OrcaWhirlpoolSwap,
            ])),
            |event| match event {
                DexEvent::PumpFunCreate(create) => println!("pump create: {create:?}"),
                DexEvent::RaydiumCpmmSwap(swap) => println!("cpmm swap: {swap:?}"),
                DexEvent::OrcaWhirlpoolSwap(swap) => println!("orca swap: {swap:?}"),
                other => println!("{other:?}"),
            },
        )
        .await?;

    tokio::signal::ctrl_c().await?;
    client.stop().await;
    Ok(())
}
```

## Queue 订阅

适合轮询消费的客户端：

```rust
use sol_shred_sdk::shredstream::ShredStreamClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = ShredStreamClient::new("0.0.0.0:8001").await?;
    let queue = client.subscribe().await?;

    loop {
        while let Some(event) = queue.pop() {
            println!("{event:?}");
        }
        tokio::task::yield_now().await;
    }
}
```

旧版 PumpFun/Bonk 客户端仍可使用 `subscribe_pumpfun`、`subscribe_pumpfun_callback` 和通用 `subscribe_with_parser` API。

## 低层解码器

已经自行管理 UDP 接收循环时，可以直接使用低层 API：

```rust
use std::time::Instant;
use sol_shred_sdk::{RawShredConfig, RawShredDecoder};

fn handle_packet(decoder: &mut RawShredDecoder, packet: &[u8]) {
    for batch in decoder.push_packet(packet, Instant::now()) {
        for entry in batch.entries {
            for transaction in entry.transactions {
                println!("slot={} sig={:?}", batch.slot, transaction.signatures.first());
            }
        }
    }
}

let mut decoder = RawShredDecoder::new(RawShredConfig::default());
```

## 自定义交易解析器

`sol-shred-sdk` 负责网络、Shred 重组和交易遍历。上层 crate 可以通过 `TransactionEventParser` 接入事件解析，无需重新实现 ShredStream 接入。

## 配置说明

- 原生 raw shred 应保持 `RawShredConfig::udp_payload_prefix_skip = 0`。
- `RawShredConfig::forward_slot_watermark` 默认为 `false`，避免丢弃乱序完成的 slot。只有明确选择前向低延迟优先于完整性时才启用。
- `ShredStreamConfig::low_latency()` 使用更短的重组等待时间并加快重启。
- `ShredStreamConfig::high_throughput()` 增加接收缓冲、跟踪 slot 数和队列容量。
- `ShredDecodeMode::JitoGrpc` 保留旧版 Entry gRPC 数据源，但使用相同的统一 `DexEvent` 解析器。
- UDP 接收缓冲通过 `socket2` 请求，操作系统可能限制最终生效值。
- Merkle FEC 恢复遵循 Agave wire 布局和 Jito 的门槛规则：只有收齐至少 `num_data_shreds` 个不重复 data/coding shard 后才尝试恢复。
- Raw UDP 解码器没有 leader schedule，因此不执行 leader 签名验证。接收不可信 UDP 时应使用可信本地转发器，或在调用 `RawShredDecoder` 前完成 shred 验签。

## 解码基准测试

Raw decoder 提供 ignored release-mode microbench，通过 `solana-ledger` 生成真实 Solana merkle shred，再交给 `RawShredDecoder` 解码：

```bash
RAW_SHRED_BENCH_ITERS=10000 cargo test --release --lib bench_decode_generated_shreds -- --ignored --nocapture
```

当前本地参考结果：

```text
packets_per_sec=3276089 slots_per_sec=102378 tx_per_sec=3276089
```

## 许可证

本项目使用 MIT License，详情参见 [LICENSE](LICENSE)。

### 社区

- Telegram: https://t.me/fnzero_group
- Discord: https://discord.gg/vuazbGkqQE

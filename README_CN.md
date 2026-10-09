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
- Raydium LaunchLab / StonkFun、CPMM、CLMM、AMM V4
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

## 指令账户上下文

外层 swap 解析会补全 DLMM、Orca、CLMM、CPMM 的指令账户上下文，并输出交易的
recent blockhash。DLMM 的 `min_amount_out` 表示指令阈值，与执行输出数量分开。
CLMM 的数量模式不能确定 `zero_for_one`，缺少执行数据时方向保留默认值。
CLMM `swap`/`swap_v2` 输出 `ix_name`、`amount`、`other_amount_threshold`、
`sqrt_price_limit_x64`、`is_base_input`。`sqrt_price_x64` 表示执行后的价格，
仅解析外层指令时为 0，不再存放指令价格限制。日志合并分别保留指令参数和执行字段，
旧版/V2 账户布局由 discriminator 确定。内置解析和 gRPC 账户补全读取完整指令尾部，
tick arrays 不再截断为 16 个。`fill_clmm_swap_accounts_with_count` 接收真实账户数量；
仅提供 getter 的兼容入口扫描至多 256 个位置，在第一个缺失/默认地址处停止。
对于 V0 交易，可使用 `parse_transaction_dex_events_with_loaded_addresses`，按消息
lookup 顺序传入调用方从缓存或 RPC 解析的 writable、readonly ALT 地址。该入口在
追加事件前校验地址数量及指令索引，内部不发起 RPC 请求。默认入口使用静态账户，
跳过程序或账户地址未解析完整的指令；PumpFun V2 交易要求与 gRPC 相同的账户布局
（买入至少 27 个，卖出至少 26 个）。同交易中其余完整指令继续解析。
`parse_transaction_dex_events_best_effort` 显式保留旧版按 discriminator 猜测协议、
缺失账户填默认值的行为；该入口的暂定结果不保证与 gRPC 对齐。

可用信息的对齐以 gRPC 外层指令解析为基准，覆盖 PumpFun/PumpSwap/Pump Fees、Raydium
AMM V4/CLMM/CPMM/LaunchLab、Meteora Pools/DAMM V2/DLMM、Orca Whirlpool。
创建首买及 Mayhem 标记从同交易创建指令按 mint 关联；PumpSwap 的 `is_pump_pool`
从费用查询参数读取。日志、执行后的 inner CPI、余额和执行结果不属于 raw shred 数据。
CPMM 初始化从账户 3 读取 pool、账户 0 读取 creator；存取流动性从账户 2 读取 pool、
账户 0 读取 owner。CPMM swap 输出 `ix_name`、精确输入模式的
`amount_in`/`minimum_amount_out`、精确输出模式的 `max_amount_in`/`amount_out`，
以及 payer、authority 和用户 token accounts。指令参数与实际执行的
`input_amount`/`output_amount` 分开，gRPC 合并日志时保留。旧 JSON 缺少新增字段
时使用默认值；Rust 结构体字面量需要补齐字段或使用 `..Default::default()`。
本地差分回归的运行方式和覆盖边界见 [parser-parity](tools/parser-parity/README.md)。

高层 UDP/Jito DEX 订阅也可通过 `ShredStreamClient::with_address_lookup_resolver`
接入调用方 ALT 缓存。回调接收交易和 slot，返回 `message::v0::LoadedAddresses`，
地址按消息 lookup 顺序排列；无 lookup 的交易不调用回调。解析失败或地址数量/索引
不合法时跳过该笔交易并继续接收。回调同步运行，适合读取预热缓存；调用方负责地址
正确性以及对应 slot 的表状态。
`client.address_lookup_stats()` 提供成功解析、解析失败和未配置 resolver 的 ALT 交易
累计计数，克隆客户端共享统计。未配置 resolver 时，仅输出静态地址完整的指令。

Orca Whirlpool 流动性指令从账户 0 读取 pool，position 位于旧版账户 3 或 V2 账户 5。
初始化从账户 0/1/2 读取 config 和 mints，pool 位于旧版账户 4 或 V2 账户 6；
旧版参数先读取一个 bump 字节。两版初始化和 V2 流动性指令均按 IDL discriminator
识别。旧版两个 mint 共用账户 8 的 token program，V2 分别使用账户 10、11。

Orca swap 新增指令参数 `ix_name`、`amount`、`other_amount_threshold`、
`sqrt_price_limit`、`amount_specified_is_input`，以及 `token_authority` 和两个
`token_owner_account_*` 地址。仅解析外层指令时，执行数量和交易前后价格保持 0，
不再用阈值或价格限制代替执行结果。两条 gRPC 合并路径保留指令参数，并以日志的
数量、价格、方向和费用为准；旧版/V2 账户布局按指令 discriminator 选择。新增字段
兼容旧 JSON，不改变 Borsh 事件布局；结构体字面量可用 `..Default::default()`。

Meteora Pools swap 将指令参数 `amount_in`、`minimum_out_amount` 与执行字段
`in_amount`、`out_amount` 分开。外层解析的执行数量和费用保持 0，并提供全部
15 个 IDL 账户，包括 pool、用户 token 账户、vault、LP 账户及 `protocol_token_fee`。
两条 gRPC 合并路径保留指令参数、补齐缺失账户，并保留日志执行数值。新增字段
兼容旧 JSON；Rust 结构体字面量可使用 `..Default::default()`。

Meteora Pools 的 `add_balance_liquidity`、`add_imbalance_liquidity`、
`remove_balance_liquidity` 现按当前 IDL discriminator 识别。事件提供 `ix_name`、
全部 16 个指令账户，以及独立参数：平衡增加的 LP 数量和 token 上限、不平衡增加
的最低 LP 数量和 token 输入量、平衡移除的 LP 数量和 token 下限。外层执行数量
保持 0；两条 gRPC 合并路径保留参数、补齐缺失账户，并保留日志执行数量。旧 JSON
使用新增字段默认值；结构体字面量可用 `..Default::default()`。此前不属于这些 IDL
指令的 discriminator 不再识别。

Meteora Pools 现支持 `remove_liquidity_single_side` 和 `bootstrap_liquidity`。
单侧移除保留 `pool_token_amount`、`minimum_out_amount` 及包含
`user_destination_token` 的 15 个账户，不推测输出 token 方向或执行数量。
Bootstrap 保留两种 token 输入量和全部 16 个账户，执行 LP/token 数量保持 0。
两条 gRPC 合并路径保留指令参数；账户调度按 discriminator 和已知 pool 选择操作，
避免按账户数量取错指令。新增字段兼容旧 JSON，结构体字面量可使用
`..Default::default()`。

Meteora Pools 的 config/config2 常数乘积建池按当前 IDL 识别，账户
0/1/2/3/4 分别为 pool、config、LP mint、两种 token mint。事件提供两种 token
输入量、config2 可选激活点和全部 26 个账户。`set_pool_fees` 提供四个费率参数、
partner 费用分子、pool 和 fee operator。协议费用使用 IDL 字段名，原
`owner_trade_fee_*` 保留为兼容别名；日志同时填充两者，gRPC 合并保留日志费率
及指令中的 partner 参数和账户。账户调度按操作类型和已知 pool 选择指令。
不再接受旧的 `initialize_pool` discriminator，`CREATE_POOL` 常量现指向 config
建池指令。新增字段兼容旧 JSON；结构体字面量可用 `..Default::default()`。

当前 Meteora Pools IDL 的六种建池指令均已支持。Permissioned/permissionless
建池通过 `stable_curve` 保留完整 Stable 曲线参数，包括 token 倍率和 depeg 状态；
费用档位建池保留可选 `trade_fee_bps`。自定义建池通过 `customizable_params`
保留费用分子、激活设置、alpha-vault 标记及全部 90 字节 padding。账户填充按
指令名称选择 24/25/26 个账户的布局，分别保留 admin、payer 和 fee-owner。
gRPC 合并保留这些参数，包括 0 和空的可选值；非法曲线/depeg 枚举、option
标记、bool 字节及截断参数被拒绝。新增字段兼容旧 JSON。

Raydium AMM V4 旧版/V2 的固定输入和固定输出 swap 新增 `ix_name`、
`instruction_amount_in`、`instruction_amount_out`。仅解析外层指令时，执行字段
`amount_in`、`amount_out` 保持 0；`minimum_amount_out`、`max_amount_in`
保留指令限制。两条 gRPC 合并路径保留包括 0 在内的指令参数、补齐缺失账户，
并以日志执行数量为准。新增字段兼容旧 JSON，跳过 Borsh 读取，不改变事件布局；
Rust 结构体字面量可使用 `..Default::default()`。

已同步 CPMM creator-fee 协议分成升级：收集事件要求新增的 PDA/config 账户，
AmmConfig 提供 `creator_fee_share_rate` 且总长仍为 236 字节，支持解码
CreatorFeeShare 账户。`cpmm_creator_fee` 提供 PDA 推导、旧收集调用升级及
精确整数分账工具。Swap、quote 和 LP 路径不变。详见
[迁移指南](docs/cpmm-creator-fee-upgrade.md)。

## 许可证

本项目使用 MIT License，详情参见 [LICENSE](LICENSE)。

### 社区

- Telegram: https://t.me/fnzero_group
- Discord: https://discord.gg/vuazbGkqQE

### 真实 RPC 交易解析示例

见 [examples 使用说明](examples/README.md)：实时 RPC 拉取、11 笔主网样本离线回放，以及原始指令和 token 余额校验。

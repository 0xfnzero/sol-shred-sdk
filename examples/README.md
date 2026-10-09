# 真实主网 RPC 解析验证

这些示例只读 RPC，不签名或发送交易。无需配置钱包。

```bash
# 已冻结样本，离线运行
cargo run --example replay_mainnet_fixtures
cargo test --test mainnet_rpc_replay

# 从 RPC 获取真实交易，验证并保存 result 对象
export SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
cargo run --example rpc_parse_transaction -- \
  fGdvyUkJvK8GYEzExeTKY6gA1i5EQ381PpCCcqEtmuzRskHavXM5kfcqScGbG7nzhFSTHMEvkKz5jdsUoXjHb2k \
  /tmp/pumpswap-mainnet.json

# DLMM + PumpSwap 真实交易
cargo run --example rpc_parse_transaction -- \
  3qyMRXiM5CCEsZHqYHmjxBgpBuzk6AQybr985BgG6Kjqz5jw4bNXWc8LtRpxGjXLvsZ4ZcQCZqpzzSnZrbSLgRN

# version 1 的真实 PumpSwap 交易
cargo run --example rpc_parse_transaction -- \
  2Bhs2661gDgovELbNCJVohrPC7zzyQmtEjt96KNkDXGt7SLdN85r1ea42xt6sbj6nh7x4Ca4fqsRxXmDR7hgTXfq

# CPMM LP deposit：核对指令限额、账户映射与实际 LP mint 数量
cargo run --example rpc_parse_liquidity -- \
  4zWB1WbQH5PSgDZhH325D9RX5TdHbN4h4mnF5Rbu3rZYsvPYeBTKY8REeBRDpuZnUFRmTNMtmfcG8kCJ1Zukzv7S

# CPMM LP withdraw：核对指令限额、账户映射与实际 LP burn 数量
cargo run --example rpc_parse_liquidity -- \
  4H82PLGNuQfH19RSCiGJBhQKpQXXfBdj25yKsr9NddUjcPb3hzDbvBPUDF7HAfSoK7C5aNzCzVtGwFtf73rm4ALa

# 升级后的 creator-fee collection：包含 creator_fee_share
cargo run --example rpc_parse_liquidity -- \
  53DpZyYtWJFF3qp1oooWHUtW4HNYonuTZ6CgE13YZynStArdLDHt27Tf5JRrksTBPBY8aKosCNHhJK7Mi6M9CAFU

# 按池当前状态推导 CreatorFeeShare PDA，再发现该池的 collection 交易
cargo run --example rpc_find_cpmm_collections -- \
  GNNDoaH6fEZ8NHhzYRbZuwGTkTfSZhzUmwJU8j9GbD8r \
  /tmp/cpmm-collection-samples 10

# 同一个解析器回放自己的 RPC 样本
cargo run --example rpc_parse_transaction -- /tmp/pumpswap-mainnet.json

# 按程序或池地址发现真实 swap，验证后保存（公共 RPC 可能限流）
cargo run --example discover_rpc_swaps -- \
  pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA \
  /tmp/pumpswap-rpc-samples 20

# 独立 zero-copy 配置
cargo test --no-default-features --features parse-zero-copy --test mainnet_rpc_replay
```

RPC 使用 `getTransaction`、`base64`、`finalized`、`maxSupportedTransactionVersion: 1`。端点默认使用 Solana 官方主网；公共节点限流或无法查询历史交易时，可使用有历史数据的 RPC。不会输出端点凭据。

## 已验证样本

完整签名、slot、预期 swap 字段及采集来源在 [manifest](../tests/fixtures/mainnet/manifest.json)。所有样本执行成功，包含 v0/ALT 和真实 CPI。

| 文件 | shred 外层事件 | RPC swap legs | token 余额校验账户数 |
| --- | ---: | ---: | ---: |
| cpmm-0.json | 0 | 3 | 9 |
| cpmm-1.json | 0 | 2 | 7 |
| pumpswap-0.json | 1 | 1 | 5 |
| orca-0.json | 0 | 1 | 4 |
| orca-1.json | 0 | 2 | 12 |
| dlmm-0.json | 1 | 2 | 7 |
| pumpswap-v1.json | 0 | 1 | 5 |
| pumpswap-buy.json | 1 | 1 | 5 |
| cpmm-deposit.json | 1 | 0 | 3 |
| cpmm-withdraw.json | 1 | 0 | 3 |
| cpmm-collect-creator-fee.json | 1 | 0 | 1 |

总计 11 笔成功交易、13 个 swap、2 个 CPMM LP 指令、1 个升级后的 CPMM creator-fee collection 指令、61 个账户余额校验。覆盖 CPMM、CLMM、DLMM、PumpSwap sell/buy（buy 为 exact-out）、Orca swap/swap_v2，以及多协议路由。含真实 version 1 交易、v0/ALT、外层与 CPI 调用。

另保存了一笔 version 1 负向样本：`2nYbrhhkGx8YiN4upcAAS8SaLTFRrrJAY98m2nqdyEnZ1nEH7BmGcyjYq1BBLVRvqoQYmw2Ta1ryfvSGVgBiENeM`，slot `453490084`，来源同为官方主网 RPC、finalized。该交易引用 DLMM 地址但日志中没有调用 DLMM，测试必须得到零 swap，避免将地址引用误判为执行。

自动发现示例先查询 `getSignaturesForAddress`，再逐笔获取原始数据；只保存真正执行了指定程序或池 swap、且通过验证的样本。单笔 RPC 错误会报告并继续；没有符合条件的样本会返回失败。不会自动修改冻结清单或覆盖已有文件。

本轮通过 RPC 获取 PoolState，独立读取 LP mint，再查询该 mint 的交易签名，取得真实 deposit/withdraw；collection 则通过 creator 和 amm_config 推导 CreatorFeeShare PDA，查询其签名取得真实样本。新增样本采集日期为 2026-10-06，旧样本保留原采集日期。

`rpc_parse_liquidity` 核对 CPMM、PumpSwap 和 Orca 的已识别 LP 指令，以及 CPMM collection 指令。当前真实 LP 正向样本仅覆盖 CPMM deposit/withdraw；其它 LP 类型和 permissionless collection 仍需补充真实正向样本。CPMM exact-out 和失败交易也尚未覆盖，不能据此宣称全协议覆盖。

## 校验方式和边界

- 解码原始交易，并用 RPC 的 loadedAddresses 解析 ALT；签名、slot 和样本覆盖数量必须符合冻结清单。
- 按独立硬编码的 wire discriminator、账户位置、u64 参数和 bool 字段核对 route 输出，并调用正常指令事件 API，对每个 swap 的事件字段进行同样的验证。
- LP/collection 另按独立 wire 布局核对参数和账户。CPMM LP 样本将实际 mint/burn 的 LP 数量与 lp_token_amount 对比；token0/1 的参数保留指令 max/min 语义。collection 验证新增账户和 CreatorFeeShare PDA，不查询该 PDA 是否已创建。
- 按原始 innerInstructions 的调用深度独立汇总每个 swap 的 token CPI 转账，核对 actual_input_amount/actual_output_amount；未知 Token-2022 扣费保留 None。
- 将整笔交易所有 token 转账汇总为账户净变化，与 RPC pre/postTokenBalances 的整数 amount 比较。跨协议共用账户不会误用单个 swap 的 gross amount 对比整笔净变化。
- 未知 Token-2022 扣费、WSOL fund/sync/close、token mint/burn/初始化/关闭、缺少 pre/post 快照的账户跳过余额校验；未解析经济活动保留在 unknown_invocations 中。
- manifest 的 expected_legs 是经过 wire 与余额校验后的 SDK 回归基线，不是额外独立的链上 oracle。余额核对证明已核对账户的整体净额；各 swap 的金额归属另由原始 CPI 调用层级和转账参数校验。
- RPC 的 innerInstructions 用于补充执行上下文；shred 无法直接取得这些 CPI 或日志。outer_events=0 的路由样本不代表丢失外层 DEX 指令。
- 示例没有通用 RPC→Yellowstone 转换器承诺，只构造 route 分析需要的字段；getTransaction 不提供 block 内 tx_index，传入的 0 是占位值。不会把指令限额当作实际成交额。

RPC 网络请求与 JSON/base64 解码属于验证工具，不进入 SDK 的 shred 解析热路径。这些测试不用于证明超低延迟。

collection 样本涉及 Token-2022，只有 1 个账户满足当前净余额校验条件。历史 CreatorFeeShare/AmmConfig 费率状态未保存，当前链上状态不等于 collection 时的历史状态；因此此样本验证了指令布局、PDA 和可核对的余额，未独立验证协议分成费率及 payout 计算。

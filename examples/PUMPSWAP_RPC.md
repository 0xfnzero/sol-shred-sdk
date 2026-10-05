# PumpSwap 真实 RPC 解析验证

本示例于 2026-10-06 从官方主网 RPC 实际取得七笔成功交易，冻结原始 `getTransaction` result 后完成验证。

| 样本 | slot | 版本 | 方向 | 虚拟 quote 储备 | 有效 quote 储备 |
| --- | ---: | --- | --- | ---: | ---: |
| sell-v0.json | 453451494 | v0 / ALT | sell | 0 | 166240698887946 |
| sell-v1.json | 453491396 | v1 | sell | 17554251914 | 101246387692 |
| buy-legacy.json | 453656764 | legacy | buy | 17584505288 | 421546451406 |
| discovered-0.json | 453659010 | v0 | sell | 0 | 170237391497318 |
| discovered-1.json | 453659010 | legacy | buy | 17584505288 | 281776749582 |
| discovered-2.json | 453659010 | v0 | buy | 0 | 225866806382033 |
| discovered-3.json | 453659010 | legacy | buy | 17583976942 | 241390866532 |

完整签名、采集来源与 SHA-256 在 [manifest](../tests/fixtures/pumpswap_rpc/manifest.json)。其中三个真实签名：

- [sell v0](https://solscan.io/tx/fGdvyUkJvK8GYEzExeTKY6gA1i5EQ381PpCCcqEtmuzRskHavXM5kfcqScGbG7nzhFSTHMEvkKz5jdsUoXjHb2k)
- [sell v1](https://solscan.io/tx/2Bhs2661gDgovELbNCJVohrPC7zzyQmtEjt96KNkDXGt7SLdN85r1ea42xt6sbj6nh7x4Ca4fqsRxXmDR7hgTXfq)
- [buy legacy](https://solscan.io/tx/57mk1HYbtLTgNUbw7LQ9rF8BW4M8aWGReLfgVA7BReiajjT2frBJivYKcfxRzsiCis8U8xt1XiRHbcc7bFfKpqaM)

```bash
# 直接 RPC 查询、验证，可选保存 result（拒绝覆盖已有文件）
export SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
cargo run --example rpc_verify_pumpswap -- \
  57mk1HYbtLTgNUbw7LQ9rF8BW4M8aWGReLfgVA7BReiajjT2frBJivYKcfxRzsiCis8U8xt1XiRHbcc7bFfKpqaM \
  /tmp/pumpswap-verified.json

# 按程序发现近期签名，再取得、验证和保存实际执行的交易
cargo run --example rpc_find_pumpswap -- /tmp/pumpswap-samples 20

# 离线回放冻结样本
cargo run --example rpc_verify_pumpswap -- tests/fixtures/pumpswap_rpc/sell-v1.json
cargo test --test pumpswap_rpc_replay
cargo test --no-default-features --features parse-zero-copy --test pumpswap_rpc_replay
```

RPC 使用 `base64`、`finalized`、`maxSupportedTransactionVersion: 1`。无钱包要求，不签名或提交交易。公共 RPC 的限流与历史保留可能影响重查，可配置自己的 RPC；错误信息不会打印端点凭据。发现器查询的是引用程序地址的交易，只有通过调用栈归属及实际事件验证的交易才保存。失败交易、缺失 CPI、缺失/截断日志和零 PumpSwap 成交均拒绝。

验证过程：

1. 解码真实 VersionedTransaction，按 static / loaded writable / loaded readonly 顺序解析账户，保留签名、slot、版本与 blockTime。tx_index、接收时间使用 0 占位，RPC 不提供它们。
2. 核对每条实际 PumpSwap buy / buy_exact_quote_in / sell 指令的参数顺序、金额和账户映射，包括路由器内部的 CPI 指令。
3. 按运行时调用栈归属过滤 PumpSwap 日志，日志 wire payload 与 event CPI 必须逐字节一一对应，再比较 SDK 两条入口的全部序列化字段。Sell 的派生 is_pump_pool 标志不属于 wire，比较时仅规范化该字段。
4. 两条入口共用解码器，因此另独立读取时间、pool/user、成交金额/限额、base/quote 原始储备与 i128 虚拟储备，检查 SDK 输出。有效 quote 使用 checked i128 求和和 u64 范围检查独立核对；base 储备不加虚拟值。
5. 离线回归检查 SHA-256、冻结签名、slot、版本、事件数及预期储备。篡改 CPI、标记失败或删除日志必须检出错误。

储备数值为 token 最小单位。输出的虚拟储备用十进制字符串，避免 JavaScript Number 丢失精度；RPC 原始数据不改写。

本轮先查询 8 条近期引用并取得其中 7 笔成功交易，发现上述 buy；再实际在线运行 Rust 发现示例，取得另外 4 笔可验证成交。另重新查询两笔已知 sell，共冻结 7 笔成交样本。真实样本目前覆盖零和正虚拟储备，没有捕获负储备；负值、i128 极值和越界计算仍由已有合成回归覆盖。未取得真实 create_pool/deposit/withdraw 样本，不宣称这些类型已经获得主网验证。本工具不核验密码学签名、每个 swap 的净余额或价格模型；它验证所列 wire 字段和解析一致性，不能单凭同解码器一致证明全部经济语义正确。

RPC、JSON、分配和日志验证均属于示例/测试，不进入 SDK 生产解析热路径。

验证记录：七笔样本均有 1 条可验证交易指令、1 对相同日志/CPI wire payload、1 个成交事件。冻结预期储备由独立 Python struct/int 字节读取生成，Rust 测试按 manifest 检查。默认及 zero-copy 回归均通过；RPC 发现示例在线取得并保存 4 笔交易，单签名示例在线验证成功。

# sol-shred-sdk v6.0.2

Released 2026-10-26.

Synchronize Pump.fun create_v2 account filling and ShredStream parsing with the Rust parser core; preserve input-only and unresolved account semantics.

同步 Rust 解析核心的 create_v2 账户填充与 ShredStream 解析修复，保留输入数据与未解析账户的语义。

## Validation / 验证

Native-language regression suites and replay/build checks passed for the synchronized changes. Four trade implementations completed curve buy/sell and pool buy/sell unsigned mainnet simulations (16 successful simulations); these small-amount interface checks do not replace fresh production quotes. No transaction was broadcast.

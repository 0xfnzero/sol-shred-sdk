# 版本记录

## v6.0.0 — 2026-10-22

同步 SDK、examples、版本导航及 AI 发布规则。请阅读 [发布与迁移说明](RELEASE_NOTES.md)，尤其注意事件精简、V3 默认行为和 Go /v9 模块迁移。

以下保留旧版 README 的发布与集成记录，其中接口、依赖和验证结果描述当时版本，不代表当前行为。当前用法见 [README](README_CN.md) 和 [发布与迁移说明](RELEASE_NOTES.md)。

## v5.0.0

详见 [发布与迁移说明](RELEASE_NOTES.md)。

## v4.0.9 — Shred 元数据透传与顺序范围

保留并验证 `tx_index`、接收时间从解析入口传到原始指令回调。明确 Entries 消息内序号与 Yellowstone 节点 index 的区别：本地序号不是全 slot 索引；跨消息去重使用交易签名。此版本完善文档和回归保护，不改变现有字段或编号规则。

## v4.0.9 — RawInstruction 协议过滤与 DEX 枚举

修复原始指令绕过 DEX 订阅范围的问题。按单条指令引用的官方程序识别 DEX，使用 `RawInstructionDexType` 枚举；JSON 字符串保持不变。未知 router 不推断池子地址、买卖方向或方法版本。

## v4.0.9 — Shred 原始指令透传

兼容入口 `PumpfunEventParser` 和 `ShredStreamGrpc` 也透传 `PumpfunEvent::RawInstruction`。修复普通交易被丢弃、无关交易被合并到创建事件的问题；旧日志解析器按调用栈保留嵌套 Pump 交易腿，并丢弃失败父调用的事件。ShredStream gRPC 传输的是执行前交易，不具备 Yellowstone 执行元数据。

运行 `cargo run --example pump_raw_replay` 回放 2026-10-19 新抓取的 Delta 交易（slot 458542546）。只输出 Shred 可见的原始指令账户和 data，不猜内部买卖。共享事件解码器已兼容官方 Anchor CPI 前缀，但 Shred 本身不包含执行元数据。

默认 Shred 入口将未适配的第三方程序（包括 Rapid/Delta）作为 `RawInstruction` 返回，不再推断买卖方向、金额或内部 buy 版本。System、Token、ATA、ComputeBudget 等基础程序默认不透传；已支持的官方 DEX 指令继续正常解析，官方 Pump/PumpSwap 多跳仍返回 `PumpRouteIntent`。

事件包含 `program_id`、`instruction_index`、`accounts`、`account_indices`、`instruction_data` 和交易 metadata。账户保留原顺序与重复项；未解析 ALT 地址为 null/None，原始索引仍保留。使用缓存 loaded-addresses/resolver 补齐，热路径不请求 RPC。data 是完整原始字节，不要求已知布局。

运行 `cargo run --example raw_instruction_filter` 对比同一组已签名交易的 PumpFun、Raydium CPMM 与全量原始指令订阅；输出包含 `dexes`。

原始事件保留调用方传入的 `metadata.tx_index` 和 `metadata.grpc_recv_us`（streamer 对应 `metadata.recv_us`）。默认 Entries 客户端的交易序号仅在单条接收消息内递增，下一条消息从 0 重新计数，不等于已确认的整个 slot 交易序号。`instruction_index` 表示交易内外层指令顺序；接收时间是本地微秒时间戳，不保证链上执行顺序。独立回放 API 可能默认填 0，调用时应传入上游已有的序号与接收时间。

无过滤器时返回未知第三方指令。宽泛交易/Swap 订阅（如 `PumpFunTrade`、`RaydiumCpmmSwap`）也返回关联该 DEX 的原始指令；买/卖方向订阅不推测方向。显式 `RawInstruction` 仅在未限制协议时返回全部外层指令，streamer 的协议限制始终生效；显式排除 `RawInstruction` 优先。`dex_type` 使用公开枚举 `RawInstructionDexType`（Go 为命名字符串类型与常量）；JSON 仍为 `"pumpfun"`、`"raydium_cpmm"` 等原有字符串。`dexes` 数组包含各关联 DEX 的 `{ dex_type, program_id, pool }`，只识别本条指令直接调用或账户中引用的 DEX 程序，不借用同笔其他指令的账户；多 DEX 包装保留所有关联。未知或未解析 ALT 中的程序不能匹配 DEX 订阅。`pool` 当前为 null：未知包装布局无法确定池子账户。program 引用不证明执行了 CPI，原始回调不是成交。

已有 wrapper 适配函数仅作为显式调用的兼容接口保留，不参与默认 Shred 解码。以下已发布版本的历史说明描述旧行为。

## v4.0.6 — Pump 路由事件完善

路由意图新增 `is_buy` 和 `ix_name`，沿用交易事件字段名和官方 IDL 方法名。RapidBuy 为买入意图，方法为 `buy_exact_quote_in_v2`；BuyPumpDelta 仅确定买入方向；多跳方向与未知方法保持为空。币种仍从 `hops[].base_mint` / `quote_mint` 获取。这些信息不代表已成交，过滤器也不会将路由意图转换为交易事件。

Streamer 现在通过 `DexEvent::PumpRouteIntentEvent` 转发完整路由字段并支持过滤。旧 JSON 缺少新字段时保持未知；Rust 结构体字面量需补齐新字段，穷举 streamer 事件的代码需处理新变体。

## v4.0.5 — Pump routes and offline replay

Adds native Pump route-intent parsing for official multi-hop instructions and the exact RapidBuy/BuyPumpDelta wrapper layouts captured in reported transactions. Adds locally cached ALT resolution and rejects unresolved account identities by default, with explicit best-effort compatibility APIs where supported. Route intents expose instruction constraints and hop identities; confirmed execution amounts require execution metadata.

Includes the latest complete implementation, signed transaction fixtures, native offline-bank regression examples and reproducible local hot-path benchmarks. Unknown wrapper layouts are not inferred from embedded bytes. Local tests do not measure production network latency.

## v4.0.4 — Pump depth-2 compatibility

Adds signed Pump depth-2 create-event replay coverage, including ALT-loaded accounts and executed logs. Existing event layouts and public parsing interfaces remain unchanged.

Validation includes official Pump SDK 4.1.0 instruction comparisons, signed local LiteSVM simulations and SDK regressions. Local simulation state/config is synthetic; no funded transactions were broadcast and production latency was not measured.

## v4.0.3 — Signed transaction and hot-path hardening

Hardens signed shred/FEC replay, rejection recovery, loaded-address and wire boundaries. Aligns DEX instruction/account layouts, compact Pump/PumpSwap events, CPMM creator fees, route attribution and liquidity/reward parsing. Malformed protobuf program keys remain unknown rather than being classified as SystemProgram. Adds signed transaction and offline bank regression fixtures.

Validation includes local CPU benchmarks and offline signed-bank scenarios. Measured hot paths use cached inputs without RPC. Benchmarks do not establish production network or transaction-landing latency. No funded mainnet transactions were broadcast.

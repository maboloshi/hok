# 阶段 3b 性能基准：细粒度事件发射开销可忽略，推迟源头发射门控

> 2026-08-29

**背景**：输出系统重设计阶段 3b（源头发射门控 + 进度节流）此前因「limited benefit / high risk / lack of performance evidence」被推迟。本次先做决策基准再决定是否落地（提示词 `.agents/docs/reports/prompts/phase-3b-source-gating-prompt.md`）。

**现状核实（读了实际代码）**
- 下载进度已节流：download.rs 的 ProgressThrottle（100ms，`PROGRESS_THROTTLE`）+ `send().await` 背压 + `try_send` 丢满即丢，分片/单流两路都接。
- 事件总线容量 `EVENT_BUS_CAPACITY = 20`；细粒度事件低频（每包数十个、由磁盘 I/O 拉开间隔），打不满总线。
- shim/shortcut/env/persist/psmodule/symlink 的 Start/Progress/Done 全部被 scoop_handler 路由到 `tracing::debug!`（默认/-v 过滤、仅 -vv/-vvv 可见）→ 发射后必被 CLI 丢弃。
- **纠正提示词一处事实**：extract 进度（`PackageExtractProgress`）是 **stdout 实时进度行**（scoop_handler 里 `print!` + flush），**不是** tracing 路由，**不能门控**。

**基准（新增 `crates/libscoop/benches/event_emission.rs`，`cargo bench -p libscoop`，bench profile = 优化）**
- `event/construct_traced_progress`：~260 ns/事件（构造 `Event::PackageShimAddProgress(String)`）。
- `event/traced_progress_try_send`：~223 ns/事件（构造 + `try_send` 进容量 20 的 `tokio::sync::mpsc`，与真实 EventBus 同型同容量，含 drainer 代表事件循环）。

**外推**：每包几十个 traced-only 事件 × ~250 ns ≈ 每包 10–20 µs；100 包约 1–2 ms，相对秒级安装开销 <0.1%，纯噪声。即便极端 1000 事件/包也才每包 ~250 µs。

**结论与决策**：**推迟完整 3b**。下载（唯一可能打满总线的源）已节流；细粒度事件低频、被 CLI 丢弃、构造+发送开销微秒级，无性能证据支撑 ~30 个发射点的门控改动；惰性格式化（改约 6 个事件变体签名为结构化字段）收益同样可忽略且违反 3a 刚定型的事件形态。设计 §10 阶段③ 3b 标记为「评估后推迟」，保留基准文件作为可复跑的证据与 guardrail。若未来要落地，最小可行片 = `Session::granular_events` 开关 + CLI 低于 -vv 关闭 + 门控 traced-only 发射点，不改事件签名、不门控 extract/download。

---

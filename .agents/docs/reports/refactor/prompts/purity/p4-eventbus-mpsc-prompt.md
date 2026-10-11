# P4 开工提示词：事件总线 flume→tokio::mpsc + 事件循环线程→task（新会话）

> 用法：新开对话后，把本文件整段贴给 agent。配合 `.agents/docs/reports/refactor/tokio-purity-plan.md` 与 `.agents/docs/reports/refactor/plan.md` 使用。
> 分支：`refactor/edition-2024`。前置：P1–P3 已完成 ✅（P3 后入口已 `#[tokio::main]`，全链路 async）。**这是纯净化最后、最高风险的一步。**

## 目的
事件总线仍是 flume（同步信道）、事件循环仍是专用线程（`src/eventloop.rs`）。这是纯净化里唯一残留的非 tokio 部分。改为 `tokio::sync::mpsc` + 事件循环 task，用 `tokio::select!` 同时等「后端事件」和「前端输入」，达成端到端纯 tokio。

## 背景与现状（P1–P3 已完成）
- `EventBus`（`crates/libscoop/src/event.rs`）：两个 `flume::bounded(EVENT_BUS_CAPACITY)`（outbound + inbound），`#[non_exhaustive]`
- `Session::emitter()`/`receiver()`（`session.rs`）：返回 flume `Sender/Receiver`；事件产生走 `tx.send(Event)`（同步、可忽略错误）
- 事件循环 `src/eventloop.rs`：`std::thread::spawn` 读 `rx.recv()`，匹配进度/提示/确认，其余交 `EventHandler`；`CursorGuard` 收光标
- 测试直接拿 flume receiver：`shim.rs:787`（conflict_count）、`sync.rs:440`（setup_confirm_test）
- flume 依赖在 `libscoop/Cargo.toml`

## 开工必读
1. `.agents/docs/reports/refactor/tokio-purity-plan.md`（完成后更新 P4 状态 ✅）
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`、`no-patchwork-fixes`、`hok-report-archive`
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`
4. gpg：提交前 `$env:HOME=$env:USERPROFILE`

## 目标与验收
- [ ] `EventBus` 换 `tokio::sync::mpsc`（容量保留 `EVENT_BUS_CAPACITY`）
- [ ] `Session::emitter()/receiver()` 适配 async send（`try_send` 保留「可忽略错误」语义）
- [ ] 事件循环线程 → `tokio::task`，用 `tokio::select!` 同时等后端事件与前端输入
- [ ] 测试里直接拿 flume receiver 的（`shim.rs:787`、`sync.rs:440`）适配 tokio mpsc
- [ ] 移除 `flume` 依赖
- [ ] **行为等价**：进度事件顺序、prompt/response 往返、确认/候选选择、幂等完全不变
- [ ] 每步 `cargo build + cargo test --workspace` 全绿；最终 checkpoint

## 行为（具体改动）
1. `event.rs`：`EventBus` 两个 `tokio::sync::mpsc::bounded(EVENT_BUS_CAPACITY)`；`sender()/receiver()/inner_sender()/inner_receiver()` 返回 tokio 类型。
2. `session.rs`：`emitter()` 改 `tokio::sync::mpsc::Sender<Event>`；所有 `tx.send(Event)` 调用点改 `tx.try_send(Event)`（保留 `let _ = …` 忽略错误）。`try_send` 在满时返回错误——确认事件循环 task 消费足够快不丢事件（或容量足够）。
3. `eventloop.rs`：线程改为 `tokio::spawn` 的 async task；`rx.recv().await` 与前端输入（`stdin` 读、prompt）用 `tokio::select!` 编排；`CursorGuard` 收尾逻辑保留。注意：`std::io::stdin()` 读是阻塞，进 `spawn_blocking` 或 `tokio::io::stdin`，别阻塞 runtime。
4. 同步测试适配：`shim.rs:787` `conflict_count`、`sync.rs:440` `setup_confirm_test` —— 用 `tokio::sync::mpsc` 的 `try_recv` 或在 `block_on`/`tokio::test` 里收。
5. 删 `flume` 依赖（root/libscoop 全删），核对 `Cargo.lock` 无残留。
6. 事件产生点全部 grep `.send(Event` 逐一改 `try_send`。

## 约束与纪律
- **行为等价是生命线（本阶段重中之重）**：进度/提示/确认的往返顺序、幂等、光标行为分毫不能变；每改一类事件一个 commit，全绿再下一类
- `try_send` 与同步 `send` 语义差异：同步 `send` 会阻塞直到有空间，`try_send` 满即错——若事件量可能瞬时打满容量，评估用 `tokio::mpsc::Sender::send().await`（在 async 上下文）而非 try_send；保持「不丢事件」优先
- 前端输入读取：`stdin` 阻塞读不能直接进 async task（会卡 runtime），用 `spawn_blocking` 或 `tokio::io::stdin`
- 提交规范：一个主题一个 commit；`git -c core.autocrlf=false`；只修本次改动 hunk（禁整包 fmt）；clippy 无新增 warning
- 提交前 review（hok-proactive-review）：官方行为对齐/边界/架构合规/测试/提交内容完整性
- 这是最高风险步骤：若某类事件改完行为不稳，可回退该类改动单独 commit（保持仓库始终可编译可测）

## 已知坑（直接避）
- `tokio::sync::mpsc` 的 receiver 一次只能被一个 task 消费（vs flume 可多 receiver）：确认事件循环是单消费者，无并发消费
- `tokio::sync::mpsc` unbounded vs bounded：选 bounded 保 `EVENT_BUS_CAPACITY` 语义；bounded 的 `send().await` 在满时挂起，别在会被取消的路径里挂死（用 `select!` + 取消分支）
- 事件循环 task 生命周期：`create_app()` await 结束时 task 要正确 join/取消，`CursorGuard` 的 `cursor::Show` 必须执行（对齐 P2 的取消收尾）
- 测试里 flume receiver 换 tokio mpsc 后，`recv` 变 async：用 `block_on` 或 `tokio::test`；`try_recv` 是同步的，适合断言「无更多事件」
- 别为了「纯」引入过度：`eventloop.rs` 若 stdin 交互复杂，保持「stdin 阻塞读放 spawn_blocking + 结果经 mpsc 回事件循环」的最小改动

## 完成时汇报
- 提交清单（commit hash + 主题）+ 每步全绿证据
- flume→mpsc 迁移覆盖（所有事件产生点）、事件循环 task 化、测试适配清单
- 行为等价验证证据（进度/提示/确认/幂等）
- 遗留/已知差异（gap 清单，不悄悄放过）
- 更新 `.agents/docs/reports/refactor/tokio-purity-plan.md` P4 状态 ✅ 并同步 `.agents/docs/reports/refactor/plan.md` 执行日志
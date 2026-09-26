# 提示词：hok 单步进度渲染方向讨论（新开对话用）

> 本提示词自包含，新会话无需上一个会话上下文。目标：**讨论并决定 hok 单步进度渲染的方向**，给出带证据的推荐；若用户要求可落地实现。

---

## 背景与当前状态

仓库：`D:\App\github项目\Scoop\hok`（Rust 2024，五层单向依赖，渲染只在 CLI 层；libscoop 只发 Event/Output、不写 stdout）。分支 `refactor/edition-2024`。

已提交的两个 commit（本主题现状）：

1. **`f09d9e0` — 单步进度统一为 TTY 感知层**（`src/output.rs`）：
   - `status/done/ok` TTY 感知：TTY 用后台线程 spinner，非 TTY 纯文本无 ANSI；TTY 检测 `std::io::IsTerminal`（不加依赖）；快步骤 `MIN_SPIN=300ms` 不画帧；`output::` 公开签名不变。
   - 新增 `status_line()` 承载报表/数据行（search/list/status/checkver/auto_pr/checkup/update/loading_cache），`status/done/ok` 只做真单步对。
2. **`ba7ca09` — TTY 定格改 `✓ <status>`**：
   - `done` 在 TTY 定格成 `✓ <status>`（去尾部 `...`），丢弃冗长的“…完成。”句；非 TTY 保持两行（status + done）。
   - 缓存刷新并入 spinner（`update.rs` 的 `status_line(refresh_cache)` → `status`）；`strip_trailing_ellipsis` + ANSI-strip 测试。

## 关键技术事实（本地 rake 源码：`D:\App\github项目\Scoop\rake`）

rake 的 UI 按场景分三类：

1. **bucket 同步 → spinner-per-bucket**（`MultiProgress` + 每行 spinner，`✓ name` 定格）：`crates/rake-cli/src/cmd/update.rs:260-347`，`Event::BucketSyncProgress{name,state}`。
2. **下载 → progress bars**（`MultiProgress`）：`update.rs:99-180`。
3. **单步/commit → 纯文本行 + `✓`，不用 spinner**：`crates/rake-cli/src/cmd/install.rs:266-303`（`CommitStart`→`Linking main/7zip (1.0)`、`CommitProgress`→`  msg`、`CommitDone`→`✓ main/7zip linked`）。无 spinner、无后台线程。

**结论：rake 只有 bucket 同步用 spinner（下载用 bars）；单步/commit 用纯文本 + `✓`。hok 当前的 `✓ <status>` 定格与 rake 的 commit 风格一致。**

## 待讨论的开放问题

1. **单步是否要“像 update.rs（bucket spinner）”处理？** 用户曾提议撤销 `f09d9e0`+`ba7ca09`，在 `scoop_handler.rs` 像 `update.rs` 那样用事件驱动 UI 组件（`BucketUpdateUI` 风格）承载单步。需评估：rake 自己单步不是 spinner，硬套偏离 rake；且单步进度不只走事件（还有 `cmd/mod.rs` 的 Output sink、reinstall 直接调用），改 `scoop_handler` 会失去统一入口。
2. **`BucketUpdateUI` 非 TTY 是否要干净化？** `BucketUpdateUI` 是 `indicatif::MultiProgress`，非 TTY/管道下会整组 `\r` 重绘出乱码/ANSI（2026-08-21 devlog 已记录同类问题）。是否在 `update.rs` 加非 TTY 分流（TTY 用 BucketUpdateUI，非 TTY 用纯文本 status_line/done）？**不需要改 Event**。
3. **是否改 `Event::BucketUpdateProgress`？** 事件已携带 name+state(Started/Succeeded/Failed)，信息足够；非 TTY 乱序是管道本质，改 Event 不解决。非 TTY 分流只需消费端 `is_tty()` 判断。
4. **组件式架构**：是否把单步 spinner 从 `output.rs` 抽成 `cui.rs` 独立 UI 组件（`BucketUpdateUI` 同层），`output.rs` 只留薄分发入口、行为不变——纯架构重构，值不值得（“命名即文档”权衡）。
5. **是否撤销重做？** 撤销会丢失已落地行为（TTY 感知 + `✓` 定格 + 缓存刷新并入 spinner），需要明确收益才做。

## 约束（如实现）

- 单一签名提交（GPG maboloshi，`commit.gpgsign=true`）；只暂存本次改动。
- `cargo build` + `cargo clippy -p hok`（0 warn）+ `cargo test -p hok` 全绿。
- 改动文件 `rustfmt --edition 2024 --check`（其余文件既有 fmt 债不动）。
- 五层单向依赖；渲染只在 CLI 层；libscoop 不写 stdout，非必要不改。
- 颜色沿用 `force_color_output`（尊重 `--no-color`/`NO_COLOR`）；i18n 沿用 rust-i18n。
- git 命令加 `-c core.autocrlf=false`（仓库 eol=lf）。

## 交付物

1. 明确的方向推荐（含理由，引用 rake 证据与仓库现状；逐条回应上述开放问题）。
2. 若确认要落地：给出改动方案 → 实现 → 验证 → 单一签名提交。
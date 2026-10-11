# P2 开工提示词：CancellationToken + Ctrl-C 取消传播（新会话）

> 用法：新开对话后，把本文件整段贴给 agent。配合 `.agents/docs/reports/refactor/tokio-purity-plan.md` 与 `.agents/docs/reports/refactor/plan.md` 使用。
> 分支：`refactor/edition-2024`。前置：P1（spawn_blocking）已完成 ✅。

## 目的
当前无取消传播（grep 无 `ctrl_c`/`CancellationToken`）。下载/解压/脚本/等待中途按 Ctrl-C 时，`tokio::runtime` 直接退出，不能干净收尾（临时文件残留、终端光标不恢复、脚本子进程滞留）。引入 `tokio_util::sync::CancellationToken` + `tokio::signal::ctrl_c()`，让操作可协作式取消，UI task 收尾恢复光标。

## 背景与现状
- `create_app()`（`src/lib.rs:140`）建 runtime + `block_on(cmd::start())`
- `checkver.rs` 的脚本执行有 `try_wait` 超时轮询循环（`std::thread::sleep` 改 async 后已可替换）
- 事件循环 `src/eventloop.rs` 用 `CursorGuard`（RAII）隐藏/恢复光标；操作被中断时线程可能未被 join，光标残留
- 依赖：需要加 `tokio-util`（`CancellationToken`）

## 开工必读
1. `.agents/docs/reports/refactor/tokio-purity-plan.md`（完成后更新 P2 状态 ✅）
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`、`no-patchwork-fixes`、`hok-report-archive`
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`
4. gpg：提交前 `$env:HOME=$env:USERPROFILE`

## 目标与验收
- [ ] 加 `tokio-util` 依赖（`CancellationToken`）
- [ ] `create_app()` 里 `tokio::signal::ctrl_c()` 触发取消
- [ ] `CancellationToken` 贯穿下载/解压/脚本/等待链路；取消时干净收尾（临时文件、光标、子进程）
- [ ] `checkver.rs` 脚本执行 `try_wait` 循环 → `tokio::time::timeout` + 取消
- [ ] UI 事件循环 task 取消时恢复光标（CursorGuard 收尾）
- [ ] 补取消路径测试（中途取消断言无残留临时文件、光标恢复）
- [ ] 每步 `cargo build + cargo test --workspace` 全绿；最终 checkpoint

## 行为（具体改动）
1. 依赖：`Cargo.toml` 加 `tokio-util = { version = "1", features = ["rt"] }`。
2. `create_app()`：`let token = CancellationToken::new();` `tokio::spawn(async move { tokio::signal::ctrl_c().await; token.cancel(); });` 把 `token` 传进 `cmd::start()` → 各 async 操作。
3. 取消点（每处一个 commit，保持全绿）：
   - `download.rs / download_fragmented.rs`：`buffer_unordered` 的每个 future 前 `token.cancelled().await` 或 `select!`；取消则清理 `.download`/.parts 临时文件。
   - `script.rs` 的脚本执行（powershell 子进程）：`tokio::time::timeout` + 取消时 `child.kill()`。
   - `checkver.rs`：`try_wait` 循环换 `tokio::time::timeout` + 取消。
   - `eventloop.rs`：UI task 取消（token 触发）时执行 `cursor::Show` 收尾。
4. 取消语义：与「异常退出」不同——取消是协作式、可清理的；错误分类新增或复用（先规划再动，别 patchwork）。

## 约束与纪律
- **取消≠异常**：取消路径要走清理（临时文件/光标/子进程），不能退化成直接退出；别用 patchwork 掩盖
- `CancellationToken` 是 `Clone + Send + Sync`，可跨 task 共享；不要用全局 static，走函数参数传递
- 取消点不要放得太密（性能）也不要太疏（响应慢）；下载/脚本这种长操作必放
- 提交规范：一个主题一个 commit；`git -c core.autocrlf=false`；只修本次改动 hunk（禁整包 fmt）；clippy 无新增 warning
- 提交前 review（hok-proactive-review）

## 已知坑（直接避）
- `tokio::signal::ctrl_c()` 在非交互/CI（无 TTY）下可能立即返回或不可用：给取消触发留 fallback（仅当有 stdin 或由事件触发）
- `CursorGuard` 是全局终端状态，取消时 UI task 的 Drop 链要确保 `cursor::Show` 一定执行（用 `JoinHandle` + 收尾，或 `select!` 的取消分支）
- 取消后 `buffer_unordered` 已 poll 的 future 不会再跑清理——清理逻辑放在「取消分支」而非「future Drop」
- `try_wait` 改 `tokio::time::timeout`：超时与取消都要 `kill` 子进程，别漏

## 完成时汇报
- 提交清单（commit hash + 主题）+ 每步全绿证据
- 取消传播链路（入口→各取消点）+ 取消测试证据
- 遗留/已知差异（gap 清单）
- 更新 `.agents/docs/reports/refactor/tokio-purity-plan.md` P2 状态 ✅
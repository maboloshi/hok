# P3 开工提示词：入口 #[tokio::main] async main（新会话）

> 用法：新开对话后，把本文件整段贴给 agent。配合 `.agents/docs/reports/refactor/tokio-purity-plan.md` 与 `.agents/docs/reports/refactor/plan.md` 使用。
> 分支：`refactor/edition-2024`。前置：P2（取消传播）已完成 ✅。

## 目的
当前入口是 `src/bin/hok.rs` 的同步 `fn main()` → `create_app()` 内部手动建 `Runtime` + `block_on(cmd::start())`（`src/lib.rs:140`）。虽然是真运行时，但写法非地道。改为 `#[tokio::main] async fn main`，`create_app()`/`cmd::start()` 转 async 直接 `await`，删手动 `Runtime` + `block_on`，统一 tokio 入口心智。

## 背景与现状
- `src/bin/hok.rs`：`pub fn main()` → `hok::create_app()`，末尾 `std::process::exit` + 恢复光标
- `src/lib.rs`：`pub fn create_app() -> bool`，内部 `let runtime = Runtime::new()`; `runtime.block_on(cmd::start())`
- `cmd::start()` 已 async（阶段 4 已转 async），37 个 `execute()` 已 async
- 错误处理：`create_app()` 里 `.inspect_err(report).is_err()` → `report` 打印错误链，返回 bool

## 开工必读
1. `.agents/docs/reports/refactor/tokio-purity-plan.md`（完成后更新 P3 状态 ✅）
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`
4. gpg：提交前 `$env:HOME=$env:USERPROFILE`

## 目标与验收
- [ ] `src/bin/hok.rs` 的 `main` 改 `#[tokio::main] async fn`
- [ ] `create_app()` 转 `async fn`，内部直接 `cmd::start().await`，删手动 `Runtime` + `block_on`
- [ ] 退出路径（`process::exit` + 光标恢复 + 错误报告）语义保持不变
- [ ] `cargo build + cargo test --workspace` 全绿；最终 checkpoint

## 行为（具体改动）
1. `src/bin/hok.rs`：
   ```rust
   #[tokio::main]
   pub async fn main() {
       use crossterm::ExecutableCommand;
       let ok = hok::create_app().await;
       let _ = std::io::stdout().execute(crossterm::cursor::Show);
       std::process::exit(ok as i32);
   }
   ```
2. `src/lib.rs`：`pub async fn create_app() -> bool`，删 `use tokio::runtime::Runtime`，`cmd::start().await`，保留 `.inspect_err(report).is_err()`。
3. 确认 `cmd::start()` 与 `create_app()` 之间无同步 `block_on` 残留。
4. root `Cargo.toml` tokio 已含 `macros`（`#[tokio::main]` 需要），核对无需增 feature。

## 约束与纪律
- **行为等价**：退出码、光标恢复、错误报告、stdout 输出顺序不因入口写法改变
- `#[tokio::main]` 默认多线程 runtime——确认与原先手动 `Runtime::new()` 的线程模型一致（多线程），不要意外变成 current_thread
- 提交规范：一个主题一个 commit；`git -c core.autocrlf=false`；只修本次改动 hunk；clippy 无新增 warning
- 提交前 review（hok-proactive-review）

## 已知坑（直接避）
- `#[tokio::main]` 的返回值类型：`process::exit` 用 `i32`，保持 `ok as i32`；若改 `Result` 返回，`main` 得处理错误，别破坏退出码语义
- `create_app()` 若被测试直接调同步 `create_app()`（如有），转 async 后调用点要补 `.await`（grep 调用点）
- 不要在这阶段引入 P2 的 `CancellationToken`（那是上一阶段的事），只做入口形态

## 完成时汇报
- 提交清单（commit hash + 主题）+ 全绿证据
- 入口改动 diff 说明（bin + lib.rs + 任何被影响调用点）
- 遗留/已知差异（gap 清单）
- 更新 `.agents/docs/reports/refactor/tokio-purity-plan.md` P3 状态 ✅
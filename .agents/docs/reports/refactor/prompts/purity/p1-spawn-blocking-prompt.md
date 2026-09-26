# P1 开工提示词：blocking 库包 spawn_blocking（新会话）

> 用法：新开对话后，把本文件整段贴给 agent。配合 `.agents/docs/reports/refactor/tokio-purity-plan.md`（决策：全面纯净化）与 `.agents/docs/reports/refactor/plan.md` 使用。
> 分支：`refactor/edition-2024`（阶段 0–5 已完成，tokio 迁移已交付；本阶段是纯净化 P1）。

## 目的
libscoop 已全面 async（Network/Git/Archive trait async + reqwest），但 **blocking 库（git2 / 解压 / rusqlite）的阻塞调用仍直接跑在 async fn 里、占用 runtime worker 线程**。并发场景（bucket 并行 pull、多包解压）下会饿死 runtime、卡住其它任务。把阻塞调用统一包进 `tokio::task::spawn_blocking`，让 runtime worker 不被阻塞 I/O 占死。

## 背景与现状（上一会话已完成）
- 阶段 0–5 已全部完成并提交（GPG 签名 G）：edition 2024 / domain 纯化 / infra trait 化 / tokio 全面 async（方案 B + reqwest）/ 护栏
- `internal/git.rs`：`RealGit::clone_repo/pull`（git2/libgit2，阻塞）在 async fn 内直接执行
- `internal/archive.rs`：`RealArchive::extract`（zstd/bzip2/sevenz/zip 解压，CPU+IO 密集、阻塞）
- rusqlite（manifest_cache 等）：阻塞 SQLite 调用在 async 上下文
- 当前 `libscoop` 无任何 `spawn_blocking`（grep 无命中）
- 工作树干净（仅 `.agents/docs/`、`AGENTS.md`、`scripts/` 等未跟踪文件，不入库）

## 开工必读
1. `.agents/docs/reports/refactor/tokio-purity-plan.md`（决策/路线/执行日志；完成后更新 P1 状态 ✅）
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`、`no-patchwork-fixes`、`hok-report-archive`
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`
4. gpg：提交前 `$env:HOME=$env:USERPROFILE`

## 目标与验收
- [ ] `RealGit::clone_repo/pull` 的阻塞 git2 调用包进 `spawn_blocking`
- [ ] `RealArchive::extract` 的阻塞解压调用包进 `spawn_blocking`
- [ ] rusqlite 阻塞调用包进 `spawn_blocking`（或确认在 async 内不阻塞 runtime）
- [ ] 调用点**零改动**（trait 层内部封装，签名不变）
- [ ] 补测试：并发下 blocking 不占用 runtime（例如同时多个 extract/pull 仍能并行、主任务不被饿死）
- [ ] 每步 `cargo build + cargo test --workspace` 全绿；最终全量 checkpoint

## 行为（具体改动，trait 层封装）
1. `internal/git.rs`：`RealGit::clone_repo/pull` 内部把 git2 调用包 `tokio::task::spawn_blocking`。注意 clone 的代理等参数是 `Send` 的可拷贝，闭包内 move；`&Path` 转 owned `PathBuf`。
2. `internal/archive.rs`：`RealArchive::extract` 内部包 `spawn_blocking`；进度回调（emitter，工具层唯一依赖 event 的例外）保持语义——在阻塞闭包内发事件仍安全（Sender 是 Send）。
3. rusqlite（manifest_cache 等）：包 `spawn_blocking`，闭包内打开/查询连接。
4. 若阻塞调用内部还调用 async 进度/事件，拆两段：阻塞计算 + 结果回来后再发 async 事件。
5. 所有默认实现行为逐字节等价（`RealNetwork` 已 async 无需动）。

## 约束与纪律
- **行为等价是生命线**：包 `spawn_blocking` 不允许改语义/输出/错误/幂等；发现问题记 gap，别悄悄带进
- `spawn_blocking` 闭包要求 `'static + Send`：Session 引用不直接进闭包（Session 是 Sync，但避免持锁/持引用跨 await），先拷 owned 值
- 提交规范：一个主题一个 commit；`git -c core.autocrlf=false`，eol=lf；只修本次改动 hunk（禁整包 fmt）；clippy 无新增 warning
- 提交前 review（hok-proactive-review）：官方行为对齐/边界/架构合规/测试/提交内容完整性

## 已知坑（直接避）
- `spawn_blocking` 闭包内不能用 async（会编译错）；异步进度上报在阻塞段结束后用 `tx.send`（flume/tokio Sender 均 Send）
- git2/archive 的 context 若含非 Send 借用（如 `&Path`），需 move owned；`spawn_blocking(move || …)` 且返回 owned 结果
- `internal::git::tests::pull_merge_commit` 在 full workspace 并行跑时偶发失败（libgit2 环境抖动，isolated 重跑即绿）——别当回归去修
- 不要对已 async 的 `RealNetwork` 包 `spawn_blocking`（那会倒退成阻塞，反向收益）

## 完成时汇报
- 提交清单（commit hash + 主题）+ 每步全绿证据
- 每处 blocking 的封装点（文件:函数）+ 并发不卡 runtime 的测试证据
- 遗留/已知差异（gap 清单）
- 更新 `.agents/docs/reports/refactor/tokio-purity-plan.md` P1 状态 ✅
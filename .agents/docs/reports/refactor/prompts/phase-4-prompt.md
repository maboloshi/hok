# hok 重构阶段 4 开工提示词（新会话）

> 用法：新开对话后，把本文件整段贴给 agent。本文件由上一会话（2026-08-21）生成，配合 `.agents/docs/reports/refactor/plan.md` 使用。
> 分支：`refactor/edition-2024`（阶段 0–3 已完成，阶段 3 的 7 个 commit 均已 GPG 签名 G）。

## 任务
继续 hok 重构计划的**阶段 4：operations 异步化（tokio）**——按 I/O 密集程度分批把并发密集的操作交给 tokio 并发执行（checkver/checkup/search/下载 → install/upgrade/cleanup），本地快操作保持同步。每步保持 `cargo test --workspace` 全绿，按分组提交规范逐主题提交。

## 背景与现状（上一会话已完成）
- 阶段 0/1：edition 2024 全量升级（已完成，见 plan 文档）
- 阶段 2：domain 纯化（已完成，4 个 commit）：Manifest/InstallInfo/Config/Package 四类型零 I/O、零 async、零框架依赖
- 阶段 3：infra trait 化（已完成，7 个 commit）：
  - network/git/archive 抽 trait + 零尺寸默认实现（RealNetwork/RealGit/RealArchive）+ Session 挂载（`OnceCell<Arc<dyn …+Send+Sync>>` + `network()/git()/archive()` + `set_*` 注入钩子）
  - 全部 I/O 调用点改走 session trait 对象；test_utils 新增内存 `FakeNetwork`/`FakeGit`，checkver/download_fragmented/bucket update 单测已离线化
  - trait 定义在工具层 internal/，无反向依赖；`Arc<dyn Send+Sync>` 已能让 worker 线程共享 fake
- **阶段 3 的 infra trait 化是阶段 4 的前提**：所有 I/O 已收敛到 Session 挂载的 trait 对象，阶段 4 只需在并发层考虑这些 trait 的 async 化或 spawn_blocking 包装
- 工作树干净（仅 `.agents/docs/`、`AGENTS.md`、`scripts/` 等未跟踪文件，不入库）；阶段 3 的 7 个 commit 均为 G 签名

## 开工必读
1. `.agents/docs/reports/refactor/plan.md`（阶段路线/风险保障/执行日志；完成后更新日志）——阶段 4 状态 ☐
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`、`local-scoop-source-path`、`no-patchwork-fixes`、`hok-report-archive`、`hok-gpg-signing`（新会话先查根目录报告类文件）
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`，需要核对语义时直接读
4. gpg 签名：提交前先 `$env:HOME=$env:USERPROFILE`，正常 `git commit`（commit.gpgsign 全局已 true）；不要用 `-c commit.gpgsign=false` 绕过

## 阶段 4 目标与验收
- [ ] 引入 tokio runtime；并发边界清晰（见下「async 边界决策」）
- [ ] `Session` 变 `Sync`（RefCell/Cell/OnceCell → 并发安全），能在 tokio 任务/spawn_blocking 间共享
- [ ] 首批 I/O 密集命令并发化：checkver / checkup / search / download（跨 package/URL 并行，而非单命令链变 async）
- [ ] 次批状态写入命令并发化：install / upgrade / cleanup（本地快操作保持同步）
- [ ] 每步 `cargo test --workspace` 全绿；最终全量 build+test 全绿 checkpoint
- [ ] 行为等价：并发化不改变单命令的可观测语义（输出/进度/错误/幂等）

## async 边界决策（先出计划再动手，本阶段最关键的架构决策）
plan「风险与保障」明确：**全异步 ≠ 快**——tokio 价值在并发场景（checkver/checkup 批量、并行下载），单命令链路保持同步更简单。据此推荐两种边界，二选一并写进计划：

**方案 A（推荐，改动小、复用阶段 3 成果）**：libscoop 保持**同步库**（零 tokio 依赖）；只在 hok bin（CLI）crate 引入 tokio runtime；并发密集命令把每个 package/URL 的同步 I/O 用 `tokio::task::spawn_blocking` + future 并行编排。阶段 3 的同步 trait/fake 原样复用，零改动。
- 优点：行为等价风险最小；fakes 仍是同步，测试离线化不破坏；不用改 Network/Git/Archive trait 为 async
- 代价：I/O 仍走阻塞线程池（spawn_blocking），不是原生 async I/O；但对磁盘/网络批量场景并发收益足够

**方案 B（大改）**：libscoop 全面 async（async fn in trait 或 async-trait），Network/Git/Archive 变 async，FakeNetwork/FakeGit 同步重建为 async。
- 优点：真 async I/O，单线程高并发
- 代价：阶段 3 的 trait/fake 全部重写，Session Sync 化 + 所有调用点 async 化，行为等价验证面大很多

**无论如何都必须先做**：`Session` 变 `Sync`（当前 `RefCell<Config>`/`Cell<bool>`/`OnceCell` 非 Sync，无法进 tokio 任务）。这是阶段 4 的地基，先单独一个主题提交。

## 现状盘点（已扫描，新会话可直接引用）
**无任何 async 运行时**：root 与 libscoop 的 Cargo.toml 均无 tokio/async-std；事件总线是 flume（同步 channel），eventloop 用专用线程读事件（`src/eventloop.rs`）。
**Session !Sync**（`crates/libscoop/src/session.rs`）：`RefCell<Config>`（`config()`/`config_mut()` 运行时借用检查）、`Cell<bool>` global、`OnceCell` event_bus/user_agent/output/network/git/archive。文档自称 Sync+Send 但实际 RefCell 非 Sync。
**并发现状（都是 std::thread / rayon，非 async）**：
- `bucket.rs:498` std::thread::spawn（bucket update 并行 pull；已用 Arc<dyn Git+Send+Sync> 共享 fake）
- `package/checkver.rs:347` std::thread::scope（worker 并发下载/跑脚本；已用 Arc<dyn Network+Send+Sync> 共享 fake）
- `package/download_fragmented.rs:40` std::thread::scope（range 并行分片）+ 重试 sleep
- `internal/archive.rs` 多处 std::thread::spawn（7z/zip 并行解码）+ available_parallelism
- `package/query` 用 rayon 并行扫 bucket（Session 不可共享，走 `internal::path::bucket_dir(root, name)` 传 root 参数）
**阶段 3 infra trait**（`crates/libscoop/src/internal/{network,git,archive}.rs`）：同步 `&self` 方法，`Arc<dyn + Send + Sync>`，可进 spawn_blocking。
**命令全集**：alias, ci-auto-pr, bucket, cache, cat, checkhashes, checkup, checkurls, checkver, cleanup, completions, config, create, depends, download, export, formatjson, hold, home, import, info, install, list, missing-checkver, prefix, reinstall, reset, search, shim, status, unhold, uninstall, update, upgrade, virustotal, which

## 建议实施步骤（每步一个 commit，先出计划再动手）
1. **async 边界决策 + Session Sync 化**：先定方案 A/B（推荐 A）；把 `Session` 的 `RefCell<Config>`/`Cell`/`OnceCell` 换成并发安全结构（`Mutex/RwLock` 或拆共享/独占），`config()`/`config_mut()` 借用语义对齐；`Session` 变 `Sync + Send` → build+test 全绿
2. **引入 tokio runtime（纯增量）**：root bin 的 `main` 建 tokio runtime（`#[tokio::main]` 或手动 block_on），命令分发保持同步调用；先只搭骨架不迁任何命令 → 全绿
3. **首批并发命令**：checkver / checkup / search / download 改用 tokio 并发编排（方案 A 用 spawn_blocking 并行 package 级工作，方案 B 则 async 化对应调用链）；每迁一个命令一个 commit → 每步全绿
4. **次批状态写入命令**：install / upgrade / cleanup 按需并发化（注意共享状态写入顺序与进度事件），本地快操作（cat/list/status/which/config/alias 等）明确保持同步并记录
5. **收尾**：更新 plan 文档执行日志；全量 `cargo build`+`cargo test --workspace` checkpoint

## 约束与纪律
- **行为等价**：并发化不许改变单命令语义/输出/进度/错误/幂等；发现问题记 gap 清单，别悄悄带进
- 提交规范：一个主题一个 commit（见 grouped-commits-preference；跨主题同文件用 `git apply --cached` 拆 hunk）；git 一律 `-c core.autocrlf=false`，eol=lf；提交前设 `$env:HOME=$env:USERPROFILE`（gpg 签名）
- fmt：只修本次改动 hunk（仓库大量文件有既有 fmt diff，**禁整包 fmt**）；clippy 无新增 warning
- **提交前 review**（hok-proactive-review）：官方行为对齐/边界/架构合规/测试/提交内容完整性，逐项过完才 commit
- 阶段 4 期间 libscoop 若走方案 A 仍是零 async 库；若走方案 B 则 async 从 trait 到底层逐层推进，同样每步全绿
- 测试真实性：fake 只用于单测路径，默认实现不变；方案 A 下阶段 3 的同步 fakes 原样可用

## 已知坑（直接避）
- **Session !Sync 是硬前提**：RefCell 不能进 tokio 任务/rayon 闭包；Sync 化时 `config()`/`config_mut()` 的运行时借用错误（`ConfigInUse`）语义要保留，别用锁死锁或放宽校验
- **事件/进度**：flume 事件总线是同步 channel，eventloop 用线程读；并发化后事件产生顺序/进度聚合（`PackageDownloadProgress` 的 dlnow 聚合、extract 进度）不能乱——参考 download_fragmented 的 AtomicU64 聚合模式
- **rayon 并行扫盘**：Session 共享受限，`internal::path::bucket_dir(root, name)` 传 root 参数的模式保留；并发化沿用
- **`internal/` 工具层禁依赖 package/ 或领域服务**：async 化若动 trait，定义位置不能让依赖反向（同阶段 3 约束）
- **gpg 签名**：本沙箱 gpg 需 `$env:HOME=$env:USERPROFILE` 才能运行；校验 `git log --format="%h %G? %s"` 全 G 才是真签名（未设 HOME 时全显示 N 是假象）
- **archive→event 是工具层唯一例外**（提取进度上报），async 化时保留该进度回调语义
- 已知 flake：`internal::git::tests::pull_merge_commit` 在 full workspace 并行跑时偶发失败（libgit2/环境抖动，isolated 重跑即绿）；不要把它当回归去修

## 完成时汇报
- 提交清单（commit hash + 主题）+ 每步全绿证据
- async 边界选型（A/B）+ Session Sync 化方案
- 哪些命令已并发化（文件:函数/测试）、哪些保持同步（本地快操作清单）
- 遗留/已知差异（gap 清单，不悄悄放过）
- 更新 `.agents/docs/reports/refactor/plan.md` 阶段 4 执行日志（含状态 ✅）
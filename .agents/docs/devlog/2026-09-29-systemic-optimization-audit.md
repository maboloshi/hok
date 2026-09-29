# hok 系统性优化点审计：先更正上一版框架报告，再补全五个维度

> 2026-09-29 ｜ 提交 `0205083` ｜ worktree 主树（`D:\App\github项目\Scoop\hok`）｜ 分支 `refactor/edition-2024`

## 为什么重做

用户判定 [`hok-overall-framework-analysis.md`](../active/audit/hok-overall-framework-analysis.md)
「广度与维度不够」。复核后同意，并先修它的失真：它把已退役的 `crates/innospect` fork 当现存单元、
引用的 `build-hok.ps1:286-304` / 根 `build.rs:88-117` 行号早已失效（winsqlite3 断言实际在
`crates/libscoop/build.rs:25-118`）、F2 的「11 处 `.buffer_unordered()`」是拍的不是数的（实测 4 文件 8 处
`futures::` 引用）。该文改为 `Status: superseded-by`，并在 `docs/README.md` 索引里标注。

## 新报告（五个维度）

[`systemic-optimization-audit-2026-09.md`](../active/audit/systemic-optimization-audit-2026-09.md)，
每条结论都带可复跑命令（§10）。核心发现：

- **结构**：`Event` 枚举 **87 个变体**、两处风格不同的分发（`eventloop.rs` 10 臂 + `scoop_handler.rs`
  265 行穷举）、`scoop_handler` 里 12 个空臂，而约 28 个变体的唯一消费者是 `tracing::debug!`；
  libscoop 公开 627 项，CLI 只用 45 条路径 / 10 个 `Session` 方法。仓库自带的
  `benches/event_emission.rs` 正是为判这笔成本而写，却从未据此决策。
- **代码冗余**：两个自研检测器（精确重复段 + 近似重复函数），141 段精确重复（生产 42 / 测试 99）。
  最锐的生产项是架构顺序 `64bit→32bit→arm64` 在 `manifest.rs` 一处手写 4 遍；测试侧最贵的是
  7 份 CLI harness（已漂移，漏一处会静默读开发机真实 `~/scoop`）与 9 份 `fn plain()`。
- **逻辑冗余**：14 个重复决策，12 个 designated 单入口里 6 个干净、6 个有旁路。
  `resolve_current_version`/`run_hook`/`bucket_dir`/`strip_bom`/`ps_command` 已完全收口——说明
  「单入口 + 调用点收敛」在本仓可复制；`is_nightly` 零生产调用、`current_meta_dir` 仅 1 个调用点
  是最廉价的两处修补。
- **crates**：release 图 255 crate。`zstd`/`git2`/`rusqlite`/`async-trait` 均不能换（各有实测理由）；
  真杠杆是 `git2` 关 `ssh`（`libssh2-sys` + 一处 openssl 分支）与 `criterion` 关 `plotters`。
- **feature**：全仓 `cfg(feature)` **0 处**——当前无法编译期裁剪任何能力，可选性靠目标平台、
  环境变量+链接器、独立 crate/bin 三种机制表达；`[features] default = []` 两处是空壳。

## 未做的事（等拍板）

六个方向明确留给用户/后续会话决定：是否放弃 ssh、libscoop 是否保留对外承诺、事件模型折叠还是下沉、
`SyncArgs` 是 flatten 还是纯数据化、`tokio` 的 `rt-multi-thread` 能否从库裁掉（需确认库内有无 `tokio::spawn`）、
`sxd-xpath` 是否替换。全部候选改动**未执行**。

## 方法与诚实边界

逻辑/代码两个维度的明细由两个只读子代理分头实测（自研检测器，脚本留在系统临时目录未入库），
关键项（`is_nightly` 零调用、`current_meta_dir` 单调用、`fn plain` 9 份、`#[command(flatten)]` 全仓 1 处、
架构顺序 4 处）由主树逐条复核。子代理的漂移判断是**结构性**结论，没有 `cargo test` 观测到的失败作后盾——
报告 §2 已写明这一点。

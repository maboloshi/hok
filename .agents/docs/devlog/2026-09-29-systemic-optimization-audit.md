# hok 系统性优化点审计：先更正上一版框架报告，再补全五个维度

> 2026-09-29 ｜ 提交 `0205083` `0834850` ｜ worktree 主树（`D:\App\github项目\Scoop\hok`）｜ 分支 `refactor/edition-2024`

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

## 追加：libscoop「能否独立给第三方用」——成立，本轮的致命项

用户提出这可能才是致命问题，验下来成立。三条阻断**全部实测**，且互不重叠（修一条还有下一条）：

1. `cargo package -p libscoop` **exit 101**：`hok-i18n`（`Cargo.toml:48`）与 `hok-shim`（`:95`）是无版本
   path 依赖，Cargo 拒绝打包；而 `crates/hok-i18n/Cargo.toml:11` 自带 `publish = false`。
2. `cargo package -p hok-i18n --list` 产物只有 **6 个文件**、没有 `locales/`——因为 `locales/*.yml` 在仓库根，
   而 `crates/hok-i18n/build.rs:33` 读的是 `manifest_dir.join("../../locales")`，读不到即 panic
   ⇒ 即便发布出去也不可构建。
3. 把 `target/` 移走（模拟全新 clone）后 `cargo build -p libscoop` **panic 在 `crates/libscoop/build.rs:204`**
   （找不到 `hok-shim.exe`）。**反证很关键**：只把 `CARGO_TARGET_DIR` 指向空目录**会成功**——因为
   `candidate_target_dirs`（`build.rs:239-250`）最后回退到本仓 `target/`；也就是说"构建成功"依赖的是
   本仓曾经构过 shim。`[build-dependencies] hok-shim`（`Cargo.toml:94-95`）是空壳，`build.rs` 内零引用。

最刺眼的是**决定只执行了一半**：`c99f07e`（2026-09-11）在 hok-i18n 里写下 "libscoop is no longer treated
as a publishable crate"，但 libscoop 自己既没有 `publish = false`，`README.md` 与 `crates/libscoop/README.md`
也还在挂 crates.io / docs.rs 徽章（crates.io 上最新仍是上游 `0.1.0-beta.7`，2024-12-10）。下一个人读 README
会以为能用。报告 §1.5 给出互斥的两条路线：A 登记"不可独立使用"（当前架构下唯一自洽），B 真做成可独立
引用（要动构建架构、把 locales 搬进 crate、先发布 i18n 两 crate，且 docs.rs 在 Linux 上永远构建不了）。

## 追加二：能力面——"能构建也没用"（用户追问后实测）

用户追问："第三方可能拿到很低级 API 需要自己造轮子，可能还没直接调用 hok 方便。"同样成立，
且比 B3 更难绕：**libscoop 给的是内核（领域操作 + 事件协议），应用外壳整块留在 CLI 里。**

- `Session::emitter()` 是 `pub(crate)`（`session.rs:538`），库内 28 个发射点用它；第三方公开面只有裸 `EventBus`。
- 自己驱动事件循环是硬要求：`resolve.rs:269-288` 与 `sync.rs:380-401` 都 `try_send(Prompt…)` 后
  `while let Some(..) = session.receive_response().await`。**完全不接总线**有安全兜底（`emitter()` 为 `None`
  ⇒ 自动批准 / 选首个候选）；**"接了总线但不驱动"**（最容易犯）⇒ `try_send` 成功 ⇒ **永久挂死**。
- `EVENT_BUS_CAPACITY = 20`（`constant.rs:22`）+ 发射点几乎全用 `try_send` ⇒ 满即 `Err` ⇒
  被当作"没有前端" ⇒ **静默跳过事件甚至自动批准**。"部分实现"比"完全不实现"更危险（行为不一致且无报错）。
- 要复刻的外壳规模：`eventloop.rs` 348 + `scoop_handler.rs` 345 + `output.rs` 484 + `cui.rs` 452 =
  **1,629 行生产代码**，全部在 CLI crate；libscoop 只提供 `EventHandler` trait（`lib.rs:82`）。
- 结论：只读状态（装了什么/能否升级）今天的 API 够用且优于解析 `hok` 文本输出；要执行装/卸/update，
  就必须自己接协议 —— **"直接 shell out 调 hok 更省事"这个判断是对的**。这条把路线 A 的论据从
  "构建不了"升级为"能构建也不划算"，同时说明路线 B 的真正缺口不是更多 `pub fn`，而是
  **库内建默认事件消费者**（例如 `libscoop::headless`）。已写入报告 §1.6 与 pending-tasks。

## 未做的事（等拍板）

六个方向明确留给用户/后续会话决定：是否放弃 ssh、libscoop 是否保留对外承诺、事件模型折叠还是下沉、
`SyncArgs` 是 flatten 还是纯数据化、`tokio` 的 `rt-multi-thread` 能否从库裁掉（需确认库内有无 `tokio::spawn`）、
`sxd-xpath` 是否替换。全部候选改动**未执行**。

## 方法与诚实边界

逻辑/代码两个维度的明细由两个只读子代理分头实测（自研检测器，脚本留在系统临时目录未入库），
关键项（`is_nightly` 零调用、`current_meta_dir` 单调用、`fn plain` 9 份、`#[command(flatten)]` 全仓 1 处、
架构顺序 4 处）由主树逐条复核。子代理的漂移判断是**结构性**结论，没有 `cargo test` 观测到的失败作后盾——
报告 §2 已写明这一点。

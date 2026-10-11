# hok「真 tokio 版」现状核查与剩余差距

> 状态：**核查完成（2026-08）** · 分支：`refactor/edition-2024`
> 说明：此前一份「tokio 迁移方案」（.agents/docs/reports/refactor/tokio-purity-plan.md）基于**旧分支（edition-2021 同步代码）**撰写，因切换分支后代码已大改，**作废并由本文替换**。

## 结论（先给答案）

当前 `refactor/edition-2024` 分支**就是「真正的 tokio 版」**——即你此前选定的**方案 B + reqwest**（libscoop 全面 async、HTTP 用异步 reqwest 兑现真非阻塞 I/O）。**不是半残。** 阶段 0–5 已全部完成并提交（git log 全 G 签名）。

## 现状证据

| 项 | 实现 | 证据 |
|---|---|---|
| 入口 | `create_app()` 建**多线程 tokio runtime** + `block_on(cmd::start())` | `src/lib.rs:140` |
| `Session` | `Sync+Send`：`RwLock<Config>`/`AtomicBool`/`OnceLock` | `session.rs`（编译期断言测试） |
| 网络 | `Network` trait 全 async + **reqwest**（rustls），`ureq` 已删 | `internal/network.rs`、commit `4c2d30a` |
| 库层 | `libscoop` 全 async：sync/install/remove/reset/download/checkver/checkurls/checkhashes/create/extract/script/bucket/auto_pr/virustotal/github | async fn 清单见下 |
| 并发 | checkver + download_fragmented 用 `buffer_unordered`；bucket update 用 `tokio::spawn` | `checkver.rs:402`、`download_fragmented.rs:80` |
| 编译 | `cargo check --workspace` 全绿（25.86s） | 实测 |

async 化范围（grep `async fn`）：`sync.rs / sync_install.rs / sync_remove.rs / download.rs / download_apps.rs / download_verify.rs / download_fragmented.rs / checkver.rs / checkver_hash.rs / checkurls.rs / checkhashes.rs / create.rs / manifest_source.rs / extract.rs / script.rs / bucket.rs / auto_pr.rs / virustotal.rs / github.rs / internal/network.rs / internal/git.rs / internal/archive.rs`。

## 对照「真正的 tokio 版」7 条判定标准

| # | 标准 | 现状 |
|---|---|---|
| 1 | 入口 `#[tokio::main]` | ⚠️ 用 `create_app()+block_on`，非 `#[tokio::main]`（**是真实运行时**，仅写法不地道） |
| 2 | `libscoop` `async fn` 链贯穿 | ✅ |
| 3 | future 组合并发（`join_all`/`Semaphore`/`select!`） | ✅ `buffer_unordered` / `tokio::spawn` |
| 4 | HTTP 客户端 async（reqwest） | ✅ |
| 5 | 取消传播（`CancellationToken` / `ctrl_c`） | ❌ **未实现**（grep 无命中） |
| 6 | 事件总线 `tokio::mpsc` + 事件循环 task | ✅ **P4 已完成**：`tokio::sync::mpsc`（`EVENT_BUS_CAPACITY` 保留）+ 事件循环 tokio task（`tokio::select!` + stdin `spawn_blocking`） |
| 7 | 阻塞环节被 `spawn`/`spawn_blocking` 调度 | ⚠️ 有 future 组合，但 **blocking 库（git2/archive/rusqlite）未包 `spawn_blocking`** |

## 剩余差距（4 项）——成本 / 收益 / 建议

### 差距 1：blocking 库未走 `spawn_blocking`（值得做）
git2 / 解压（zstd/bzip2/sevenz）/ rusqlite 是阻塞库，当前在 async fn 内**直接阻塞 runtime worker 线程**。多任务并发（如 bucket 并行 pull、多包解压）时可能饿死 runtime。
- **改法**：把 `RealGit::clone_repo/pull`、`RealArchive::extract`、rusqlite 调用包进 `tokio::task::spawn_blocking`（trait 层封装）。
- **成本**：中。**收益**：并发场景不卡 runtime。**建议：做。**

### 差距 2：无 Ctrl-C / `CancellationToken` 取消（值得做）
当前没有取消传播（grep 无 `ctrl_c`/`CancellationToken`）。下载/解压/脚本中途 Ctrl-C 不能干净收尾（临时文件、光标）。
- **改法**：`create_app()` 里 `tokio::signal::ctrl_c()` → 全链传 `CancellationToken`；脚本执行用 `tokio::time::timeout` 替换 try_wait 循环。
- **成本**：中。**收益**：体验（中断干净）。**建议：做（可选）。**

### 差距 3：事件总线 flume→mpsc + 事件循环 task（不建议）
事件总线仍是 flume（同步信道），事件循环仍是专用线程。**这是有意的简化**——事件消费是单消费者，不需要 async，改造无性能收益。
- **改法**：flume→`tokio::sync::mpsc`，事件循环线程→`tokio::task`。
- **成本**：大（改 session.rs/eventloop.rs/所有 emitter 调用）。**收益**：无。**建议：保持现状。**

### 差距 4：入口 `#[tokio::main]`（可选）
当前 `create_app()` 手动建 runtime + `block_on`，是**真实 tokio 运行时**，只是非地道的 `#[tokio::main]`。
- **改法**：`main()` 改 `#[tokio::main] async fn`。**收益**：写法统一。**建议：可做可不做。**

## 重新定位（回应"半残 还是 真 tokio"）

- **并发 I/O 层**：已经是**真 tokio**（reqwest 非阻塞 + buffer_unordered/tokio::spawn 并发）。**不是半残。**
- **整体纯度**：是**务实的混合体**——对「异步无收益」的部分（事件总线、入口写法）保持简化，符合 plan「全异步 ≠ 快」原则。这是合理取舍，不是"半残凑合"。

## 决策（2026-08）：全面纯净化

用户决定：**教科书式纯净 tokio 仍要做**——虽然收益不大，但既已重构至此，求彻底一致（统一 tokio 心智、移除混合体、解锁端到端取消与 `select!`）。据此，此前的「差距 3/4 保持现状/可选」建议**废止**，全部纳入路线。

## 纯净化路线（P1→P4，每步 `cargo build + cargo test --workspace` 全绿 checkpoint，按分组提交规范逐主题提交）

| 阶段 | 内容 | 风险 | 顺序 | 状态 |
|---|---|---|---|---|
| P1 | blocking 库包 `spawn_blocking`（RealGit::clone_repo/pull、RealArchive::extract、rusqlite） | 低 | 1 | ✅ |
| P2 | `CancellationToken` + Ctrl-C（`tokio::signal::ctrl_c`）取消传播；下载/脚本子进程取消 + kill 清理 | 中 | 2 | ✅ |
| P3 | 入口 `#[tokio::main]` async main（`create_app()` → async，`cmd::start()` 直接 await） | 低 | 3 | ✅ |
| P4 | 事件总线 flume→`tokio::sync::mpsc` + 事件循环线程→tokio task（**最高风险，最后做**） | **高** | 4 | ✅ |

### 各阶段要点
- **P1**（2026-08 完成）：trait 层把阻塞实现包 `spawn_blocking`（`RealGit::clone_repo/pull`、`RealArchive::extract`），调用点零改动；补并发不卡 runtime 的测试。rusqlite（manifest_cache）经确认**非并发热点**（`query` 每命令单次同步调用，非 tokio::spawn 扇出），不在 async 阻塞 runtime 饥饿路径，保持同步、未包 spawn_blocking。
- **P2**（2026-08-22 完成）：`create_app()` 起 `tokio::signal::ctrl_c()` → Session 挂载 `CancellationToken`；分片下载（buffer_unordered 逐 chunk select!）+ 单流下载（select!）取消时**保留** `.parts`/`.download` 续传状态（清理只删真失败产物）；`run_script`/`run_checkver_script` 子进程取消时 `kill` + `wait` 收尾；checkver `try_wait` 循环换 async poll。**单流续传**（`aa51fb3`）：`RequestOptions.range_start` + `download_stream` Range 续传（206 append / 200 全量重写），单流 `.download` 跨次续传。事件循环光标恢复由 CursorGuard RAII + 全路径 done-event + bin `cursor::Show` 保障（复核确认无需额外代码）。
- **P3**：`src/bin/hok.rs` 改 `#[tokio::main] async fn main`，删手动 `Runtime` + `block_on`；`create_app()``cmd::start()` 转 async 直接 await。
- **P4**（2026-08 完成）：`EventBus` 两信道换 `tokio::sync::mpsc`（容量保留 `EVENT_BUS_CAPACITY`）；`session.rs` 的 `emitter()` 返回 tokio Sender、新增 `receive_response()` async 等待入站响应；所有事件产生点 `tx.send`→`tx.try_send`（保留「忽略错误」语义）；`confirm_transaction`/`select_candidate`/`resolve_dependencies` 转 async；`eventloop.rs` 线程→`tokio::task`（主循环 `tokio::select!` 等后端事件，prompt/确认的 stdin 阻塞读包 `spawn_blocking` 防卡 runtime，响应 `tx.send().await` 不丢）；CLI 调用点 `handle.join()`→`handle.await`；同步测试（`shim.rs` `conflict_count`、`sync.rs` `setup_confirm_test`、`resolve.rs` 测试）适配 `tokio::test`+`try_recv`/`recv().await`；`flume` 依赖删除（Cargo.toml/Cargo.lock 无残留）。

### 纪律
- **行为等价是生命线**：每步 checkpoint 全绿；进度事件顺序、prompt/response 往返语义、幂等不变。
- P4 在 P1–P3 已全绿、行为被测试钉死的基础上动手，把最高风险的最后做。
- 提交规范：一个主题一个 commit；`git -c core.autocrlf=false`，eol=lf；提交前 `$env:HOME=$env:USERPROFILE`（gpg 签名）；只修本次改动 hunk（禁整包 fmt）。
- 完工后更新本文档「决策」状态为 ✅，并同步 `.agents/docs/reports/refactor/plan.md` 执行日志。

## 纯净化 P5（已完成，2026-08，`b70f160`）
- `bucket.rs::update` 遗留 5 处 `let _ = tx.send(...)` 非绑定 future（clippy `let_underscore_future`）改 `send().await` 收尾——空桶 `BucketUpdateDone`（479）、并发 pull spawn 内 `BucketUpdateProgress` start/succeeded/failed（513/520/526）、收尾 `BucketUpdateDone`（549），对齐下载路径 `d967c7f` 的背压纪律。`let _ =` 保留（满/关静默忽略）；低频事件直接 await，无死锁。行为等价（事件顺序/last_update/并发 pull/幂等不变），clippy 0 warning，测试全绿。
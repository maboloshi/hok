# 纯净化 P5：bucket update 事件 send() 收尾

> 2026-08-23 ｜ commit `b70f160`
>
> 原引用 `1e01967`，2026-08 重构后等价提交为 `b70f160`。

**背景**：P4 把事件总线从 flume 迁到 `tokio::sync::mpsc`（`7d294dc`）后，下载路径已在 `d967c7f` 对齐 rake 背压（100ms 节流 + 单流 `send().await`），但 `bucket.rs::update` 遗留 5 处 `let _ = tx.send(...)` 非绑定 future——clippy `let_underscore_future` warning 既有债，且语义上丢弃 future 等于放弃背压。P5 收尾：把这 5 处改 `send().await`，消除 warning、对齐下载路径的背压纪律。

**改动（单文件 `crates/libscoop/src/bucket.rs`，5 处，7+/5-）**：

| 行号 | 事件 | 上下文 |
|---|---|---|
| 479 | `BucketUpdateDone` | `update()` 空桶分支 |
| 513 | `BucketUpdateProgress(Started)` | 并发 pull `spawn` 内、`git.pull` 之前 |
| 520 | `BucketUpdateProgress(Succeeded)` | pull `Ok` 后 |
| 526 | `BucketUpdateProgress(Failed)` | pull `Err` 后 |
| 549 | `BucketUpdateDone` | 全部 pull 收尾后 |

- `let _ =` **保留**（总线满/关闭时静默忽略，与 P4 语义一致）；`send().await` 在低频事件（每桶几条）下容量 20 足够，不会挂死，**直接 await 未引入 `select!` 过度设计**（对齐 prompt 约束：低频事件直接 await）。
- 513 行因超过 100 列被 rustfmt 拆三行 `.send(...).await`；其余 4 处单行即可。

**行为等价（生命线，逐项核对）**：
- 事件顺序：每桶 start→pull→result 保持；`for handle in handles { handle.await }` 编排未动，`BucketUpdateDone` 仍在全部 pull 完成后发出；
- `last_update` 写回（541–546）未改，仍无条件写盘后再发收尾 Done；
- 并发 pull：`tokio::spawn` + `handle.await` 循环、`emitter.clone()` 所有权结构原样，只加 `.await`；
- 幂等：纯事件发出时序，无状态变更。

**验证**：`cargo clippy -p libscoop --all-targets` 0 warning（5 个 `let_underscore_future` 清零）；`cargo build -p libscoop` + `cargo test -p libscoop --lib -- --skip internal::git` 全绿（**439 passed, 0 failed**，既有 `bucket_update_uses_injected_git_fake` 覆盖该流程）；改动 hunk `rustfmt --edition 2024 --check` 合规（仅剩 657/668 行既有 test 格式债，非本次，未动）；`git log --format="%h %G?"` 全 G（gpg 签名）。

**坑/纪律**：
1. `bucket.rs` 里其它既有债（import 排序、let-chain、test 尾部格式）**混不进本主题**——只改这 5 处 `send().await`；`rustfmt --check` 报告 657/668 行 diff 是既有 debt，判定后不动；
2. `internal::git::tests::*` 是环境级 libgit2 抖动（本步 `--skip`），非本主题回归；
3. 低频事件直接 await 即可，别为"纯"引入 `tokio::select!` + 取消分支的过度设计——容量 20 + 事件循环 task 持续 poll，不会死锁。

**文档同步（均不入库，仅工作树）**：`.agents/docs/active/plan/refactor/plan.md` 新增「纯净化 P5」执行日志段；`.agents/docs/active/plan/refactor/tokio-purity-plan.md` 补 P5 状态段。

---

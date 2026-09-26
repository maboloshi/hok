# 下载进度对齐 rake：100ms 节流 + 单流 send() 背压

> 2026-08-23 ｜ commit `8009f1c`

**背景**：P4 把事件总线从 flume 迁到 `tokio::sync::mpsc::bounded(20)`（`f2b2262`）后，所有事件产生点改 `try_send`（保留 `let _ =` 忽略错误）。下载进度是**高频**事件（单流逐 chunk、分片逐 range），事件循环消费慢于产生时 `try_send` 满即丢 → 进度条偶发跳变（P4 记录的已知差异）。rake 用 `THROTTLE_DURATION=100ms` 节流 + flume 阻塞 send 背压解决同类问题，本步对齐。

**改动（4 文件，164+/22-）**：
- **`ProgressThrottle`**（`download.rs`，`Mutex<Option<Instant>>`，`Sync` 可共享给并发分片 chunk）：至少 `PROGRESS_THROTTLE=100ms` 才发一条；`force` 参数让最终事件（`dlnow>=total`）绕过窗口必达，进度条到 100%。分片 + 单流两路径都接入。
- **`Network::download_stream` 回调改 async**（`network.rs`）：`&mut (dyn FnMut(u64)+Send)` → `&mut ProgressCallback<'_>`（`Pin<Box<dyn Future<Output=()>+Send>>`）。**`AsyncFnMut` 实测非 dyn-compatible**（含 GAT `CallRefFuture`），改用 boxed future；`RealNetwork`/`FakeNetwork` 内 `.await` 调用。
- **单流背压**：闭包改 `move` + 返回 boxed future，`tx.send(...).await`（节流后逐事件不丢）；`let _ =` 保留（总线关闭时静默忽略）。
- **分片仍 `try_send`**（对齐 plan「分片仍只能 try_send」，但已节流降低丢概率；分片在 `buffer_unordered` async 上下文，理论上也可 await，为最小改动保持 try_send）。
- 适配：`download_fragmented.rs` 测试 `test_download_stream_resumes_from_partial` 的闭包改 boxed future + `Arc<AtomicU64>`（原闭包 `reported = n` 在 `move` 下不写回外部变量）；`test_utils.rs` 补 `ProgressCallback` import。

**行为等价（生命线）**：只影响进度事件频率与送达，下载正确性/幂等/续传（Range resume）不受影响；节流 force 保证最终 100% 事件仍发出，`PackageDownloadDone` 后续事件逻辑未动。

**验证**：`cargo build` ✅；`cargo test -p libscoop --lib` 排除 git flake 后 **439 passed, 0 failed**（含 4 个节流单测：首达/窗口拦截/force 绕过/超间隔重开）✅；`cargo clippy -p libscoop --all-targets` 本次改动 0 warning（剩 5 个 `bucket.rs` `let_underscore_future` 是既有债，P5 `1e01967` 已清）✅；本次改动 hunk `rustfmt --edition 2024 --check` 合规（import 排序/let-chain 是既有债，未动）✅；GPG `Good signature`。

**坑/纪律**：
1. **`AsyncFnMut` 非 dyn-compatible**——网络层 trait 对象回调必须 boxed future，别再用 async-trait 直接透传 async 闭包；
2. `move` 闭包返回 boxed future 时，async 块内若想写外部变量要 `Arc<AtomicU64>`/`Cell`（`move` 捕获为副本，`reported = n` 静默不生效——测试因此先红后改）；
3. 节流 force 判断 `dlnow >= total`：单流 estimated（`remote_size=0`）时恒真→退化为不节流，但 estimated 包字节少可接受；分片 `use_fragments` 前置已保证 `remote_size>0`，无此问题；
4. 低频/高频分流：下载进度是高频（逐 chunk），bucket update（`1e01967`）是低频——前者节流+await 背压，后者直接 await 即可，别一刀切。

---

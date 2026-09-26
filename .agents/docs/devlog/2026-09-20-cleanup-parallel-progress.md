# `cleanup` 并行化 + 逐版本进度

> 2026-09-20 ｜ 4 提交 `3bca476` `54a87d0` `2ad6e9d` `e8c09f1`，worktree `.worktrees/cleanup-parallel`，分支 `feat/cleanup-parallel-progress`

**起因**：用户反馈 `hok cleanup` 不是逐条输出、且删除很慢。实测（`.agents/docs/research/cleanup-delete-perf.md`）：Windows **没有**递归删目录的 API（`RemoveDirectoryW` 只删空目录，"删整个文件夹"= 逐条删），删除成本 ≈**0.87 ms/文件**（3.6 万文件串行 31.7 s）；并行近线性（4 线程 4.2–4.4×）；换删除 API（`FileDispositionInfoEx` + POSIX 语义）**零收益**（15.47 s vs 15.66 s）；唯一 O(1) 的是 rename（12 个目录 39 ms），但会把空间回收推迟到后台 ⇒ 用户明确**不做 rename**，只做并行 + 进度呈现。

**改动**：① `CleanupResult.failed: usize` → `failures: Vec<CleanupFailure { version, error }>`，**lib 不再自己 `session.output().error()`**（错误由结果携带，前端才能给出行级且顺序确定的呈现）；② 删除改为 `std::thread::scope` + 共享队列（单位 = 旧版本目录，并发 `min(4, 核数)`），结果写进按位槽位 ⇒ **返回顺序 = 请求顺序**（并行不污染可复现输出）；③ CLI 改用 `cui::RowUI`：每个旧版本一行 spinner，失败就**原地收尾成 `✗ app：ver (err)`**，`✓ 一切正常！` 只在零失败时打，失败改末尾一条 `WARN ... 未能移除：app（请关闭占用这些文件的程序后重试；相关目录可能已被部分删除）`；管道模式从结果渲染同一文本（按请求顺序）；④ 删掉误导性的 `cleanup_removed_failed`（「重启后可重试」——全仓从无 `MoveFileEx(..., DELAY_UNTIL_REBOOT)`，且非提升进程该调用只会 `err 5`）。

**语义决定（用户 2026-09-20 拍板）**：K1 = C1（每个失败版本一行）、K2 = **退出码保持 0**（对齐 hok `update` 的 WARN 口径；官方是抛异常中断整批 + exit 1，hok 有意更宽容并登记偏离）。K3–K5 按推荐（管道与 TTY 同文本、收尾行列 0、`-g` 不加阶段头）。

**验证**：`cargo test --workspace` 全绿（libscoop lib **762 passed / 0 failed / 8 ignored**）、`clippy --workspace --all-targets -- -D warnings` 干净、改动文件 `rustfmt --edition 2024 --check` 干净；新增单测 3 条（并行结果顺序稳定 + 观察者事件配对、被占用版本作为失败明细返回、strict 模式报错）；隔离 HOME/scoop 根端到端：占用场景 stdout 2 行 `✓`、stderr `✗ app2：0.9.0 (…os error 32)` + `WARN` 汇总、**exit 0**、`app1` 两个旧版本删净、`current` 保留、`✓ 一切正常！` 未出现；释放占用后重跑即清干净。

**TTY 实测（同日补，harness 里 `is_terminal()` 恒 false，故另造真控制台）**：用 ConPTY 复验——`conpty-capture.rs`（**GUI 子系统**宿主，源码在 `.agents/scripts/cleanup-bench/`）+ `vt-screen.js` 回放，拿到运行中「4 行 spinner 同时在转（= 4 个 worker）」与两种终帧：全成功 `✓ alpha：0.9.0 …` + `  ✓ 一切正常！`；含占用时 `✗ foxtrot：0.9.0 (…os error 32)` + `WARN … 未能移除：foxtrot（请关闭占用这些文件的程序后重试；相关目录可能已被部分删除）`，均与 `.agents/docs/active/spec/cleanup-parallel-spec.md` §3.9 规格一致。唯一外观瑕疵：收尾行仍带 2 空格缩进而版本行从列 0 起（K4 未做）。三个坑：① ConPTY 宿主**必须无控制台**（普通控制台宿主会把自己的 std 句柄交给子进程 ⇒ 子进程用宿主管道、`IsOutputRedirected=True`、捕获为空）；② `CreateProcessW` 的 `DllImport` 漏 `CharSet=Unicode` 会把命令行编组成 ANSI → `CreateProcessW err 2`；③ 回放器必须实现 **CJK 双宽 + 终端自动折行**（ConPTY 每行补空格到列宽再折行）。

**教训**：① 「删整个文件夹更快」是错觉——没有那个 API，唯一 O(1) 的是 rename；② 计时会被系统睡眠污染（首轮 bench 量出 `std_serial=11182559ms` 的 3 小时假数据），长跑基准要 `SetThreadExecutionState` 保活；③ 并行化的前提是"错误文本由结果携带"——lib 边删边 `eprintln` 的话，前端永远做不出行级、确定顺序的呈现。

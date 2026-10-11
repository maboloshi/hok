# cleanup 删除耗时与文件占用语义（2026-09-20 实测）

> **这是什么**：`hok cleanup` 删除性能与「文件被占用」行为的实测记录，作为
> `.agents/docs/active/plan/pending-tasks.md` §2 那条「失败汇总文案与语义冲突」待办的证据来源。
> **复跑**：`.agents/scripts/cleanup-bench/{bench.rs,bench2.rs,probe3.rs}`
> （`rustc -O --edition 2024 <file> -o <exe>`，bench2 会自己阻止系统睡眠）。
> **环境**：本机 D: NTFS、release `hok.exe`；hok 的删除实现 = `std::fs::remove_dir_all`
> （`crates/libscoop/src/internal/fs.rs:44-45`），所以 bench 用同一实现即是真实成本。

## 1. 前提事实：Windows 没有「递归删除整个目录」的原子 API

`RemoveDirectoryW` 只能删**空**目录，任何「删文件夹」都是遍历 + 逐条删。实测同批 1.8 万文件：

| 实现 | 耗时 |
|---|---|
| `std::fs::remove_dir_all`（hok 今天） | 15.66 s |
| 手写 `FindNextFile` + `DeleteFileW`/`RemoveDirectoryW` | 20.03 s（还多一次 per-entry `symlink_metadata`） |

⇒ **单线程 ~0.87 ms/文件（≈1150 文件/s）**，成本由条目数决定、与字节数无关。
唯一 O(1) 的目录操作是 **rename**：12 个旧版本目录 0.039 s。

## 2. 并行：唯一有效的算法杠杆（近线性）

单位 = 旧版本目录，动态取任务，`std::thread::scope`（**不需要 rayon**）：

| 场景 | 串行 | 2 线程 | 4 线程（版本粒度） | 4 线程（app 粒度） |
|---|---|---|---|---|
| 12 app × 1 旧版 × 3k（36k 文件） | 31.7 s | 14.8 s（2.13×） | **7.37 s（4.30×）** | 7.35 s |
| 2 app × 6 旧版 × 3k（36k 文件，偏斜） | 32.0 s | 15.3 s | **7.37 s（4.34×）** | 14.99 s（仅 2×） |
| 8 app × 1 旧版 × 10k（80k 文件） | 76.6 s | 31.7 s | **18.2 s（4.19×）** | 19.1 s |

**并行单元必须是「旧版本目录」而不是「app」**：偏斜场景（少 app、多旧版）按 app 并行只有 2 个任务，速度掉一半。

## 3. 换删除 API：零收益

| 变体（1.8 万文件） | 耗时 |
|---|---|
| `std::fs::remove_dir_all` | 15.66 s |
| `FileDispositionInfoEx` + POSIX 语义（省掉经典删除的 rename-to-delete-pending） | **15.47 s** |
| POSIX + 2 / 4 线程 | 7.24 s / 3.52 s |
| `std` + 4 线程 | 3.66 s |

⇒ POSIX 语义**没有**速度收益，不值得为它写 unsafe FFI（占用场景下也无用，见 §4）。

## 4. 文件被占用时的真实行为（probe3 + 端到端实跑）

| 场景 | 实测结果 |
|---|---|
| 文件被别的进程以**不共享删除**方式打开 → `DeleteFileW` | `err 32` 共享冲突 |
| 同上去开 `DELETE` 句柄（POSIX 语义删除的前提） | **`err 32`** —— POSIX 语义救不了这种情况 |
| `rename` 整个版本目录（内含被占用文件） | **`err 5` 拒绝访问**（同资源管理器「文件夹或其中的文件已在另一程序中打开」） |
| 目录里只剩被占用文件时删目录 | `err 145` 目录不是空的 |
| `MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT)` 非提升进程 | **`err 5` 拒绝访问**（该路径需要管理员，写 HKLM `PendingFileRenameOperations`） |
| 文件被以**共享删除**方式打开 | 立即成功（经典删除 / POSIX 都行） |
| 版本目录里有**正在运行的程序**（实测 cmd.exe） | **rename 成功**；进程退出后删除也成功 |
| junction 指向被占用目录 | 删链接成功，目标不受影响 |

**hok 现状（release 版实跑，`hok cleanup -a`，旧版本目录里 200 个文件 + 1 个被占用）**：

```
ERROR 无法移除 app1 v0.9.0：另一个程序正在使用此文件，进程无法访问。 (os error 32)
      app1 已清理 0 个旧版本，1 个失败（重启后可重试）
      ✓ 一切正常！          exit=0
```

- 版本目录**被部分删除**（200 个里删掉 199 个，只剩被占用那个），目录留存；
- **exit 0**（`ignore_failures: true`，`src/cmd/cleanup.rs:50-53`、`crates/libscoop/src/package/cleanup.rs:242-250`）；
- **不登记任何「重启后删除」**：全仓无 `MoveFileEx(...DELAY_UNTIL_REBOOT)`。文案里那句「重启后可重试」只是提示再跑一次；
- 释放占用后再跑一次即清干净（实测 run 2 成功、current 版本保留）。

## 5. 对「延迟删除（rename 到同卷 staging）」方案的影响

- ✅ 能救**程序仍在运行**（最常见）：rename 成功，进程退出后后台删掉；
- ❌ 救不了**被别的进程以无 share-delete 方式打开的普通文件**：rename 同样 `err 5`；
- ❌ 「重启删除」对非提升的 hok 不可行（`err 5`），且即便提升也是逐文件注册、空间要拖到下次启动才释放；
- ⇒ 失败路径只能设计成：**rename 失败即退回今天的报错 + 留待下次重试**，staging 里删不掉的目录留到下次清扫重试，绝不假装成功。

## 6. 未开工的待决策项（未获批准，勿当已定方案）

1. 并行删除（版本目录粒度、并发 4）+ 逐 app 实时输出：用户已选「每 app 一条、TTY 走 `cui::RowUI` spinner 行原地收尾、hok `✓` 风格」，但**范围未拍板**；
2. 落地并行时必须同时处理：渲染回主线程保序、`session.output()` 并发安全
   （`crates/libscoop/src/output.rs:132-136` 是多线程各自 `println!`）、删除前对 (app, version) 单元校验
   （历史上出过删错版本目录的坑，回归见 `crates/libscoop/src/package/cleanup.rs:528-584`）；
3. 环境层变量未测：杀软/Defender 实时防护对本机 ~0.87 ms/文件 的影响（改动 Defender 设置需用户授权）。

## 7. 计时坑（复跑必读）

本机中途自动睡眠会把挂起时间算进 `Instant`：第一次 bench 量出 `std_serial=11182559ms`（≈3.1 小时）的假数据。
`bench2.rs` 已用 `SetThreadExecutionState(ES_CONTINUOUS|ES_SYSTEM_REQUIRED|ES_DISPLAY_REQUIRED)` 保活；
`bench.rs` **没有**这段，长时间跑之前先确认不会睡眠。

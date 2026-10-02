---
name: hok-audit-fix-history
title: 审计修复沉淀的通用教训（事件闭环 / 语义组合 / 基准纪律）
description: 多轮审计修复沉淀的通用教训：对标改动先枚举三类读取（解析/mtime/枚举）、上游同一行为常拆在多个提交里、事件必须闭环否则 join 挂死、改 sync 语义先查 SyncOption 组合、基准必须 --release、前置过滤优于事后同步、进度渲染降频。动 install/uninstall/update、做上游对标或写基准前先读。
metadata:
  type: project
---

**用途**：动 install / uninstall / update 流程、改事件、写性能基准**之前**先过一遍这份清单。
**历史**（三轮审计各修了什么、uninstall 怎么对齐官方）见
[`reports/audit-fix-history-2026-08.md`](../docs/reports/audit-fix-history-2026-08.md)——本文只留可复用的教训。

## 对标修复（改命名 / 路径解析 / 落地形态之前）

- **先把「读取」枚举成三类，再动手**：①**元数据解析**（`parse_install_info` 那类）②**mtime / 排序**
  （`Get-InstalledVersion` 按 LastWriteTime 选版本那类）③**枚举 / glob**（列版本、列清单）。
  对标改动最常漏的是 ②③——它们不做内容解析，`grep 读文件` 时看不出来。
  判例：上游 Scoop 0.6.0 把 `install.json`/`manifest.json` 改成 `scoop-install.json`/`scoop-manifest.json`，
  第一轮读点清单列了 11 处**解析/存在性**读点，却漏了 5 处 **mtime 类**（`list`/`export` 的 `Updated` 列、
  `newest_version_dir`、`info` 的版本目录选择、坏包的 best-effort bucket）。**这类漏点不会报错**，
  只会静默回落到"app 目录的 mtime"或"该版本目录不被列为候选"。
- **别只信审计/清单里的行号**：清单是静态读码的产物，收尾前用 `grep` 对目标符号重跑一遍并逐条勾选。
- **上游同一行为常拆在多个提交里**（本轮 checkver GitHub 模式是 4 个提交**一次**变更）：改动前先按
  "行为"而不是"提交"归组，否则会移植到一半、新旧语义混在一起。

## 事件闭环（最容易挂死的地方）

- **任何流程结束/错误都必须发对应的 Done 事件**（`PackageSyncDone` / `DownloadDone` …）：事件循环靠它们
  `break`（`src/eventloop.rs:238,241`），不发则 CLI 主 `await` 永不返回（早期实现是 `handle.join()`）；
  **空包与提前返回路径同样要发**（空包死锁就是这么来的）。
- confirm 被拒 ⇒ 返回 `UserAborted`（exit 0，不打 `ok_all`）；**sync 的错误路径也要发结束事件**。

## 语义改动

- 改 sync 语义**先查 `SyncOption` 组合**（`Force` / `OnlyUpgrade` / `AssumeYes` 会互相影响）；
  `Force` 时 `upgradable_version` filter **不能滤掉包**（曾致 force no-op，已有回归测试钉住）。
- **`OnceCell` 型 transaction 组装后不可再改** ⇒ **前置过滤**（先算清要装/要跳过的集合）优于事后同步。
- `ignore_running_processes` 口径：默认**跳过**继续；开键则**警告后继续**；held 跳过与被依赖拒绝是**有意偏离**。

## 性能与基准纪律

- **所有基准必须 `cargo test --release`**：`lzma-rust2` 的 `optimization` feature（unsafe 汇编）只在 release
  生效，debug 慢 3–5 倍——我们曾用 debug 数字下过错误的性能结论。
- 交互式进度渲染的开销主要在 **stdout flush**（不是事件通道）；应**降频**（每 N 项或定时）而非逐项。
- 7z 大包优先 `for_each_entries` / `BlockDecoder`（块级），**禁 `read_file` 逐文件**（solid 归档 O(n×块) 灾难）；
  `thread::spawn` 闭包需 `'static` ⇒ 用 Arc 共享 data/filter/dest。

## 提交与工具

- PowerShell 内联提交信息的展开坑（`$` / 反引号）与替代写法（`git commit -F <文件>`）见 [[hok-worktree-parallel]]。
- rustfmt 单文件会递归格式化子模块 ⇒ 见 [[hok-rust-gate-practice]]（不在本文重复）。
- 同一文件混多个主题、拆不出干净 commit 时，在 message 里注明。

## 测试与并行

- **"偶发红"先怀疑测试自身**：进程级 once 闩锁 + 并行测试会产生假 flaky（重跑即过）。闩锁要挂在**会话**
  （`Session::mark_stale_warned` / `mark_mismatch_warned`，key 用 `bucket/name`）而不是进程 static，
  并行跑同一测试才稳定。

## 相关

[[scoop-gap-audit]]（对标方法与现状表）、[[hok-code-organization]]（分层与去重门槛）、
[[hok-shim-dual-variant]]（shim/shortcut 专题）。

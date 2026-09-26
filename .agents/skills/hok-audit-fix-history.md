---
name: hok-audit-fix-history
title: 审计修复沉淀的通用教训（事件闭环 / 语义组合 / 基准纪律）
description: 多轮审计修复沉淀的通用教训：事件必须闭环否则 join 挂死、改 sync 语义先查 SyncOption 组合、基准必须 --release、前置过滤优于事后同步、进度渲染降频。动 install/uninstall/update 或写基准前先读。
metadata:
  type: project
---

**用途**：动 install / uninstall / update 流程、改事件、写性能基准**之前**先过一遍这份清单。
**历史**（三轮审计各修了什么、uninstall 怎么对齐官方）见
[`reports/audit-fix-history-2026-08.md`](../docs/reports/audit-fix-history-2026-08.md)——本文只留可复用的教训。

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

- PowerShell 里传含 `$` 的 commit message 用**单引号**（双引号会把 `$fname` 展开成空——已踩过）。
- rustfmt 单文件会递归格式化子模块 ⇒ 见 [[hok-rust-gate-practice]]（不在本文重复）。
- 同一文件混多个主题、拆不出干净 commit 时，在 message 里注明。

## 相关

[[scoop-gap-audit]]（对标方法与现状表）、[[hok-code-organization]]（分层与去重门槛）、
[[hok-shim-dual-variant]]（shim/shortcut 专题）。

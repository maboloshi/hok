---
name: local-scoop-source-path
title: 本地 Scoop 源码位置
description: 需要对照官方 Scoop 行为时读：官方源码的本地副本在 hok 仓库的兄弟目录 ../Scoop/，直接读 lib/ 与 libexec/，不必联网。
metadata:
  type: reference
---

官方 Scoop（Windows PowerShell 实现）的本地副本在 **hok 仓库的兄弟目录** `../Scoop/`。
需要参照官方行为时直接读它的源码（`lib/core.ps1`、`libexec/scoop-install.ps1` 等），不必联网拉取。

> 路径是相对 **hok 仓库根**的；`.agents/` 已入库，故此处不写本机绝对路径（见 [`.agents/AGENTS.md`](../AGENTS.md) 硬约束 5）。

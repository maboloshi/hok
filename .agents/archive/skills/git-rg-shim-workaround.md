---
name: git-rg-shim-workaround
title: git/rg shim 报错,用完整路径调用
description: 本机 git/rg 的 Scoop shim 会报错,直接用完整路径 D:\App\Scoop\apps\git\current\cmd\git.exe
metadata:
  type: reference
---

本机 `git`/`rg` 的 Scoop shim(`D:\App\Scoop\shims\*.exe`)会报错 "Shim: Could not create process" 导致命令失败(如 `git diff`、`git status`、ripgrep 的 grep 工具)。**绕过方式:直接用完整路径调用**。

- git: `D:\App\Scoop\apps\git\current\cmd\git.exe`(用法示例:`& "D:\App\Scoop\apps\git\current\cmd\git.exe" diff --stat`)
- rg: 同理可用 `D:\App\Scoop\apps\rg\current\rg.exe`,或改用 PowerShell `Select-String` 代替 grep

判断 shim 是否挡路:先 `Get-Command git -All` 看 Source,若指向 `shims\` 就换完整路径。

**Why:** 2026 会话中多次遇到 shim 报错导致 git diff/status、rg grep 失败;用户明确要求记录路径以便后续直接使用。

**How to apply:** 本机执行 git/rg 命令失败且报 "Shim: Could not..." 时,直接用上述完整路径;写 shell 命令时优先用完整路径或 Select-String。

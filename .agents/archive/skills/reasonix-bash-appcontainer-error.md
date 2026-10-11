---
name: reasonix-bash-appcontainer-error
title: reasonix bash 沙箱 AppContainer 错误与临时解法
description: bash 沙箱 appcontainer 错误（CreateAppContainerProfile 失败）：临时改 reasonix.toml bash=off + 重启才生效
metadata:
  type: project
---

reasonix 的 bash 工具在 Windows 上启用 `[sandbox] bash = "enforce"` 时会报 `appcontainer: get SID: CreateAppContainerProfile: The parameter is incorrect.`——AppContainer profile 创建失败（系统级问题），导致**所有 bash 命令被拒**（cargo/git/任何命令都跑不了）。

**Why:** reasonix 在 Windows 用 AppContainer 沙箱执行 bash 命令，宿主 CreateAppContainerProfile 失败时无优雅降级，直接阻断。

**How to apply:** 临时解法是把项目根 `reasonix.toml` 的 `[sandbox] bash` 改为 `"off"` 并**重启 reasonix 进程**（配置只在启动时读取，改了不重启不生效）。恢复方法：改回 `"enforce"` 重启。注意 reasonix.toml 通常 untracked（不进 git 提交）。2026-08-11 遇到并临时关闭。

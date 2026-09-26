---
name: hok-audit-fix-history
title: Hok 审计修复史（三轮）
description: Hok 三轮审计修复史（2026-08-12/13 全部已提交）：重构残留/隐藏问题/回归边界；防踩坑=事件结束必发 Done、update -f 用 Force、UserAborted 配套
metadata:
  type: project
---

Hok 三轮审计修复史（2026-08-12/13，全部已提交，commit 见 git log e96a3ec..HEAD）。用户驱动：三轮 explore 审计 + review 复核，每轮用户选「高+中+低全部」修复。按轮分组（避免原三条命名混淆：historical 的「两轮」是重构残留轮，非 second-round）。

**第 1 轮 · 历史重构残留**（旧大重构+小修小补遗留）：
- URL 文件名两语义：`internal::url::url_filename`（fragment `#/dl.7z` 参与 → "dl.7z"，Split-Path 语义）vs `url_remote_filename`（原始名 "foo.exe"）——**checkhashes 缓存命名用 remote，installer fallback 用 filename**；create.rs 勿统一（尾斜杠 fallback "download" 被测试锁）
- 进程枚举 fail-closed（running_processes_under 失败不再当无进程）
- Manifest.hash 死字段删除（假注释 cache validation 从未读取）
- Cmd trait 脚手架删除（36 个零调用 impl；组 import 删 `, Cmd` 保留 validate_dir/ensure_global）
- 死函数清理（os_is_arch64/scoop_arch/buckets_dir/walkdir_files/sysinfo 注释块）
- **审计误判保留**：manifest_cache::refresh 有调用者、dag 未用方法有测试、auto_pr split 非 bug

**第 2 轮 · 隐藏问题**（用户实测撞出）：
- **非交互卡死全家桶**：prompt_yes_no EOF/空行 → 保守 false；uninstall 补 -y；空 remove 事务不弹确认；候选选择尊重 AssumeYes（内置选排序后第一个）；**候选循环 EOF → break 0**
- download 失败 exit 非零（先发 DownloadDone 再报错；**空包/错误路径也必须发 DownloadDone**，否则 handle.join() 挂死——空包死锁是第 3 轮发现的漏网）
- running-check 提前 + 过滤后重建 PackageSet（运行中包不再下载）
- 事件闭环：ResolveDone/SizingDone 补发；outdated 文案归 update 命令层（事件循环 PackageSyncDone 只 break）
- ignore_failures 默认 false（对齐 Scoop）；manifest_cache 序列化失败跳过条目
- 光标恢复（exit 前 Show）；桶枚举失败 warn 不误报 PackageNotFound

**第 3 轮 · 修复回归边界**：
- download 空包死锁（packages.is_empty 也发 DownloadDone）
- **update -f 语义**：`SyncOption::Force`（保留 OnlyUpgrade 已安装限制 + 重装当前版本）；注意 **upgradable_version filter 在 Force 时不能滤掉包**（曾致 force no-op，有回归测试 only_upgrade_force_reinstalls_current_version）
- 预查不误判缺失包（missing_target fall through 报 PackageNotFound）
- **UserAborted**（confirm 拒绝返回它，exit 0 不打 ok_all）；**关键：sync.rs 错误路径也发结束事件**（否则事件循环 break 不了 join 卡死）
- 低项：reinstall ok_all、cui 死参、shim 排序无 panic、validate_dir 失败 exit 非零

**通用防踩坑**：
- 事件循环：任何流程结束/错误都必须发对应 Done 事件（PackageSyncDone/DownloadDone），否则 CLI handle.join() 挂死
- sync 语义改动先查 SyncOption 组合（Force/OnlyUpgrade/AssumeYes）
- rustfmt 单文件会递归格式化子模块（package/mod.rs → manifest.rs 既有空行被误改）——提交前 checkout 恢复
- 同文件多主题无法拆 commit 时 message 注明

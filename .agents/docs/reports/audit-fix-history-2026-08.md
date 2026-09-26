# 审计修复史（2026-08-12/13 三轮 + uninstall 对齐收官）

> **冻结记录**（按 [`.agents/docs/AGENTS.md`](../AGENTS.md) R5 归档，正文不改）。
> **通用教训**已提炼进技能 [[hok-audit-fix-history]]；**逐项对标现状**见
> [`active/audit/scoop-gap-status.md`](../active/audit/scoop-gap-status.md)。
> 五段 2026-08-15 的修复记录（`$fname` / running 过滤 / 解压进度条 / 7z 性能 ×2）
> 已并入 devlog [`2026-08-15-nsis-extraction-and-7z-perf.md`](../devlog/2026-08-15-nsis-extraction-and-7z-perf.md)，此处不再重复。

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

## uninstall 对齐收官（2026-08，已提交）官方参照 `../Scoop\libexec\scoop-uninstall.ps1`（本地副本）：
1. `unlink_current` 位置：persist 之后移到 shim::remove/shortcut_remove 之后、psmodule::remove/env::remove 之前（官方 `$refdir = unlink_current $dir` 位置）
2. `post_uninstall` 时机：移到删版本目录之后，包进 `if let Some(version_dir)` 块（官方 `if ($version)` 结构）；版本目录在 unlink_current **之前**解析（select_current_version 依赖 current\manifest.json）
3. 删除粒度：`remove_dir(app_dir)` 一次删整个 → ①persist_unlink+remove_dir(version_dir) ②遍历 app_dir 下非 current 目录逐个删（官方 old-version loop）③删残留 current 链接 ④仅当 app_dir 空时删它；目录不存在继续、被占用返回 Error::Custom（对应官方 continue）
4. `persist::unlink(session, package, version_dir)` 改为按版本目录解除（官方 unlink_persist_data $manifest $dir）
5. `unlink_current` 容忍 current 不存在（NotFound 静默，对齐官方 Test-Path 保护/NO_JUNCTION）
- 残余 nit（未改）：check_not_running 在 pre_uninstall 之前（官方先 hook 后 test）；pre_uninstall/uninstaller/shim/shortcut 无条件运行（官方在 `if ($version)` 内）；post_uninstall working_dir 是 &app_dir（官方 $refdir）；persist.rs::unlink 的 session 参数未用


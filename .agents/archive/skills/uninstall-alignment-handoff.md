---
name: uninstall-alignment-handoff
title: uninstall 对齐交接(已收官)
description: uninstall 对齐收官:unlink_current/post_uninstall 时机与删除粒度已对齐官方,剩余仅 nit
metadata:
  type: project
---

**目标**:继续对齐 hok 的 uninstall 流程与官方 Scoop。官方参照:`D:\App\github项目\Scoop\Scoop\libexec\scoop-uninstall.ps1`(本地副本,勿联网)。

**历史完成**(分支 `copilot/refactor-rust-workspace`):
- `5d92660` feat(libscoop|env): env_set/env_add_path 安装应用、remove 对齐(大小写不敏感 + 双 target PATH/isolated + 空值删变量)、升级前清旧版本 env、reset 重新应用、global(HKLM)+ REG_EXPAND_SZ 值类型保留
- `af5abfe` fix(libscoop): persist/psmodule/shortcut/checkup 路径改 `session.effective_root_path()`(global 感知)
- 本次(未提交):uninstall 流程对齐剩余 3 项差异,全部完成

**本次已完成(uninstall 对齐收官)**:
1. `unlink_current` 位置:从 persist 之后移到 shim::remove/shortcut_remove 之后、psmodule::remove/env::remove 之前(官方 `$refdir = unlink_current $dir` 位置);psmodule/env 自行解析路径,不需要官方 $refdir 返回值
2. `post_uninstall` 时机:移到删版本目录之后,且包进 `if let Some(version_dir)` 块(官方 `if ($version)` 结构);版本目录在 unlink_current **之前**解析(select_current_version 依赖 current\manifest.json)
3. 删除粒度:`remove_dir(app_dir)` 一次删整个 → ①persist_unlink+remove_dir(version_dir) ②遍历 app_dir 下非 current 目录逐个 persist_unlink+remove_dir(官方 old-version loop) ③删残留 current 链接 ④仅当 app_dir 空时删它;每步目录已不存在时继续、被占用时返回 Error::Custom(对应官方 continue),不再整体失败
4. `persist::unlink(session, package, version_dir)` 改为按版本目录解除(官方 unlink_persist_data $manifest $dir),operations::persist_unlink 同步加 version_dir 参数
5. `unlink_current` 容忍 current 不存在(NotFound 静默,对齐官方 Test-Path 保护/NO_JUNCTION)

**残余差异(nit,本次未改,均为既有行为)**:
- `check_not_running` 在 pre_uninstall 之前(官方是先 hook 后 test_running_process)
- pre_uninstall/uninstaller/shim/shortcut 无条件运行(官方在 `if ($version)` 内);极端情况:version_dir 为 None 且 current 悬空 + manifest 有 uninstaller.file 时 run_gui 会在悬空路径失败 abort
- post_uninstall 的 working_dir 是 &app_dir(官方用 $refdir);persist::unlink 内 remove_symlink 错误仍被 `let _` 吞掉
- persist.rs::unlink 的 session 参数实际未用(`let _ = session;`)

**验证**:`cargo check --workspace` 零警告;`cargo test -p libscoop` 全绿(239 unit + 1 + 9 + 14 + 11 doc-tests,2026-08 复核)。相关背景见 [[hok-shim-shortcut-handoff]]。

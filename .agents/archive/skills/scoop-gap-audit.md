---
name: scoop-gap-audit
title: Scoop 对标差距与进度
description: Scoop 对标差距+修复进度（合并）：P0 全修、P1 剩 9、P2 download 已修、P3 剩 11 项；带 ✅/⏸ 状态标记
metadata:
  type: project
---

方案 C 全面对标 Scoop 的差距审计（2026-08-10，4 个 explore 子代理，file:line 详见阶段报告）+ **修复进度（2026-08 多轮，已提交）**。编号 P0/P1/P2/P3，audit 与 progress 已合并，以下标注 ✅=已修 / ⏸=暂缓 / 剩余。

**P0（不兼容/静默错误）——全部 ✅ 已修：**
1. `hash: ""` 空串 → 官方 warn+继续（download.ps1:719-724）；hok 已改
2. license/version/homepage 缺失 → 官方零校验容错；已修
3. BOM/注释/尾逗号 JSON → 已修（parse_manifest_spec 宽容链）
4. installer.file 退出码丢弃 → 已修
5. `autoupdate.hash` 字符串形式 → 已修
6. bin 对象格式 → 已修（解析期容错）

**P1（CLI 兼容缺口）：**
- ✅ 7. -a/--arch（install/update/download）
- ✅ 8. installer.file+script 并存、args-only fallback
- ⏸ 9. update <app> 先同步 buckets（用户暂缓）
- ✅ 10. scoop 自更新（设计决策：hok 单二进制，不做）

**P2（功能缺口）：**
- ✅ download 独立命令（2026-08-12）：`download_apps`/`DownloadOptions{force,check_hash}`，逐 query 独立（@version/URL/本地/bucket），不下依赖不查已安装；`-f/--force`、`-s/--no-hash-check`、`-a/--arch`；事件闭环用专属 `Event::DownloadDone`；失败 exit 非零；缓存命名用 url_remote_filename。实现细节见 [[hok-audit-fix-history]] 与 [[hok-refactor-guide-checklist]]
- ⏸ install_psmodule / ensure_install_dir_not_in_path / installer.keep / post_install $dir（部分已随重构覆盖，未逐项核对）

**P3（低/风格）约 11 项剩余**：#4 重复安装文案、#9 退出码机制、#10 成功提示、#13 非 git main 转换、#15 -q/--quiet、#16 输出文案、#17 cooldown（hok 特有）、#18 hok 增强、#20 hash 前缀、#21 架构 truthy 降级

**Why:** 对标分轮进行，修复前查本记忆避免重复调研。
**How to apply:** 下一轮建议 P1-9（update 先同步 buckets）或 P3#15（-q/--quiet）。修复后更新本记忆的 ✅/⏸ 标记。官方参照 `D:\App\Scoop\apps\scoop\current\libexec\`（本地 Scoop 安装，勿联网）。

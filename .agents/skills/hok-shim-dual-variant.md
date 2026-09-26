---
name: hok-shim-dual-variant
title: hok shim/shortcut 专题
description: shim/shortcut 专题：console/GUI 双变体与 no_std 坑、lnk/shim 归属字节扫描对齐官方。
metadata:
  type: project
---

hok shim/shortcut 专题（合并自 dual-variant + shortcut-handoff，2026-08-11 全部已提交）。

## shim 双变体架构（console/GUI 按目标 PE subsystem 选择）
- `hok-shim`（console subsystem 显式声明）→ console 目标（git/python/reasonix）：shell 等待、交互 stdin 正常
- `hok-shim-gui`（GUI subsystem）→ GUI 目标（vscode/notepad）：双击无黑框、shell 不等待
- libscoop `shim.rs add()` 用 `internal::pe::is_gui()`（PE Subsystem @ PE+0x5C）选择变体；未知/读失败默认 console
- 核心逻辑在 `crates/hok-shim/src/lib.rs`（no_std，entry/exit_process/write_stderr），两个薄 bin；libscoop/build.rs 嵌入 HOK_SHIM_BYTES + GUI_BYTES

**技术坑（no_std + rust-lld 都踩过）**：
- `#![no_main]` 自定义入口必须显式 `#![windows_subsystem = "console"]`（否则 "subsystem must be defined"）
- lib.rs 提供 `__CxxFrameHandler3` stub（panic handler 是 loop{} 不 unwind，stub 永不调用）
- no_std 测试 `use std::prelude::v1::*;`（core prelude 无 String/Vec）
- package 级 `[profile]` 在 workspace 成员被忽略（需放根）
- PowerShell 管道 → cmd.exe 直接不工作（pwsh 7 怪癖）；验证 stdin 用 python

## 快捷方式/shim 行为对齐官方
- **shortcut 零依赖字节扫描**判断 .lnk 归属（ANSI/NUL IDList + UTF-16LE WorkingDir，三段约束 apps→pkg→版本目录）；同包/无法确定 → 静默覆盖；他包 → PackageShortcutConflict；Start Menu 从不警告（对齐官方）
- **shim add 对齐 warn_on_overwrite**：`shim_owner_package`/`owner_from_content`（文本 tokenize + 三段约束，兼容相对/绝对路径）；同包静默覆盖；他包 → 警告 + 改名 `{name}.{ext}.{旧owner}` 保留；只检查归属载体（Exe→.shim、脚本→.cmd/.ps1）
- **shim remove 对齐 rm_shim**：我的备份存在→只删备份；否则删主+恢复他包最新备份（is_main_shim_variant 排除主变体）；`.shim` 无备份可恢复→连带删 exe stub；备份后缀一律 `{name}.{ext}.{owner}`（= alt_filename 格式）
- **shortcuts-rs 1.1.1 坑（2026-08-16 `5beedb7`）**：`ShellLink::new()` 内部**先 setter 后 `set_link_flags()` 整体覆盖**（赋值非 OR）→ HAS_ARGUMENTS/HAS_NAME/HAS_ICON_LOCATION 被清 → 参数/名称/图标静默丢失（ima-copilot `--force-device-scale-factor=1.5` 实测 Arguments 空）。hok `create_shortcut` **必须在 `new()` 之后重放** `set_arguments/set_name/set_icon_location`；回归测试断言 .lnk 字节含 **UTF-16LE** 参数字符串（只断言 is_ok/exists 抓不住静默丢失）
- Event `PackageShimConflict(String)` + detail.shim_conflict（en/zh）
- 测试：shortcut.rs fixtures/wscript_git.lnk + 字节级单测（含 `test_create_shortcut_persists_manifest_args`）；shim.rs 8 个单测
- **验证快捷方式参数**：用 `WScript.Shell` COM 读 `.lnk` 的 `Arguments` 属性最权威（字节扫描易误读 UTF-16/ANSI 混合段）

**Why:** 双变体选择、零依赖字节扫描、备份后缀=旧包名 都是非显然设计，新会话接手避免重复调研。
**How to apply:** 接手 shim/shortcut 先读本记忆；官方 scoop 源码本地对照（lib/core.ps1 warn_on_overwrite/get_app_name/shim、lib/install.ps1 rm_shim）；涉及 shim 改两个 bin + lib.rs 同步；`internal::pe` 是目标检测唯一入口。

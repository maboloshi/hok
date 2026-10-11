# 快捷方式参数丢失修复

> 2026-08-16 ｜ shortcuts-rs 1.1.1 标志位覆盖 bug，commit `5beedb7`

**用户报告**：ima-copilot（`scoop-private` bucket，`scoop-private/bucket/ima-copilot.json`）的快捷方式没写入 `--force-device-scale-factor=1.5`。

## 排查链（证明不是 manifest 问题）
1. bucket manifest 与安装时快照（`apps\ima-copilot\<ver>\manifest.json`）**都带**参数 → 是"创建方"丢参数，不是 manifest 缺参数；
2. 官方 Scoop `lib/shortcuts.ps1 startmenu_shortcut` 明确 `$wsShell.Arguments = $arguments` → 若官方 Scoop 安装不会丢；
3. 实际 `ima.lnk` 用 WScript COM 读 `Arguments = []`，且 .lnk 结构是 shortcuts-rs 风格（UTF-16 StringData WorkingDir + RelativePath）→ **hok 创建且丢了参数**。

## 根因（第三方库，非 hok 逻辑错误）
shortcuts-rs 1.1.1 `ShellLink::new()` 顺序是：先 `set_arguments/set_name/set_icon_location`（`update_link_flags` 逐个置位 HAS_ARGUMENTS/HAS_NAME/HAS_ICON_LOCATION），**随后 `set_link_flags()` 整体覆盖**（header.rs 是赋值非 OR）成 `IS_UNICODE|HAS_WORKING_DIR|HAS_RELATIVE_PATH|HAS_LINK_TARGET_ID_LIST` → `create_lnk` 按标志写 StringData，参数/名称/图标**全部静默丢失**。hok 的 `create_shortcut` 直接把 args 传给 `new()`，恰好踩中。

## 修复（`5beedb7`，单文件 `crates/libscoop/src/shortcut.rs`）
- `create_shortcut` 改为 `ShellLink::new(target, None, None, None)` 之后**重放** `set_arguments/set_name/set_icon_location`，标志才最终生效；
- 测试先红后绿：新增 `test_create_shortcut_persists_manifest_args`（断言 **UTF-16LE 参数编码出现在 .lnk 字节**，用真实参数 `--force-device-scale-factor=1.5`）+ 强化 `test_create_shortcut_with_args_and_icon` 同样做字节级断言——修复前 2 失败、修复后 shortcut 组 10/10 通过；
- `rustfmt --check` 通过；全量 libscoop 417 通过（仅 `internal::git` 的 `pull_fast_forward`/`pull_merge_commit` 2 个 CRLF 断言失败 = 本机 `core.autocrlf=true` 导致的既有环境问题，与本次无关）。

## 端到端验证
release 构建（13:17:38）→ `hok reset ima-copilot` → WScript COM 读 `ima.lnk`：`Arguments = [--force-device-scale-factor=1.5]` ✓（13:24:27 重建，618→690 字节）；三个 junction（`current` / 版本目录 `User Data` / `LocalAppData\ima.copilot\User Data`）与注册表右键键均完好。

## 教训
1. **依赖库的 setter"生效"不能想当然**：shortcuts-rs 的 setter 与 `new()` 内 flags 覆写顺序相反，必须读库源码（header.rs `set_link_flags` 是赋值非 OR）确认；本次先写字节级回归测试（红）再修（绿），用测试锁定"参数确实落到 .lnk"而非只断言 `is_ok()+exists()`（旧测试就是只断言存在，抓不住静默丢失）；
2. **官方对照先行**：Scoop `startmenu_shortcut` 会写 Arguments，先排除"manifest/安装方"嫌疑，把范围缩到 .lnk 创建方；
3. **沙箱环境验证会制造假故障**：本会话 `reset` 报 `os error 183`（junction 删除被沙箱拒 → `junction::create` 撞"已存在"）与 `Error accessing the registry`（post_install `reg import` 被沙箱拒）都是**沙箱伪影**，非 hok bug；真实终端下 reset 应全绿。判断依据：同操作在受限会话外（原始安装 13:09 的 reg import）成功、且 hok 代码路径的忽略式清理（`let _ =`）正是为容忍这种删除失败而设计的；
4. 验证快捷方式参数用 `WScript.Shell COM` 读 `.lnk` 的 `Arguments` 属性最权威（字节扫描易误读 UTF-16/ANSI 混合段）。

---

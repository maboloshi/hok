---
name: hok-no-junction-implementation
title: Hok no_junction 完整实现（P1-18 配套）
description: no_junction 全链路实现：Session::current_dir_name/versioned_dir，link/shim/metadata/脚本 $dir/query/list/cleanup 均感知。
metadata:
  type: project
---

Hok 的 `no_junction` 配置（config.rs:364，JSON key `no_junction`，alias `no_junctions`）已完整实现（原为半失效：link 无条件建 junction、shim/shortcut/metadata 硬编码 `current`）。

**核心设计**（session.rs）：
- `Session::current_dir_name(version) -> &str`（session.rs:283）：no_junction 时返回版本号、否则 `"current"`——shims/shortcuts/env/metadata 的**运行时入口目录名**，对齐官方 `currentdir`（core.ps1:355）
- `Session::versioned_dir(name, version)`（session.rs:303，原 `version_dir` 改名）：**真实版本目录** `<app>/<version>`——解压目标、link_current target、卸载删除定位。**语义不可改成 no_junction 感知**：升级场景下 current junction 仍指向旧版本，解压/链接必须用真实版本目录

**全链路改动**（no_junction=true 时）：
- `package/operations/link.rs`：`link_current/unlink_current` 加 `no_junction: bool` 参数，no_junction 跳过 junction 创建/删除（对齐官方 link_current，install.ps1:238）
- `sync_install.rs`：metadata（manifest.json/install.json）写入 `app_dir.join(current_dir_name(version))`（:587）；env 旧版本清理用 current_dir_name
- `sync_remove.rs`：pre_uninstall/uninstaller 脚本 `$dir` 用已解析版本目录（:178/:195/:205，官方 scoop-uninstall.ps1:55 `$dir=versiondir`）
- `query.rs`：`load_install_state/query_installed/query_synced(_cached)` 在 no_junction 下经 `select_current_version`（query.rs:229 共享，mtime 回退）读版本目录 metadata
- `list.rs:41`/`cleanup.rs:55`：no_junction 时用 select_current_version 替代 read_link(current)
- `env.rs`/`shortcut.rs`/`shim.rs`：全部改用 `session.current_dir_name(...)`（shim 元数据路径 `~\..\apps\<pkg>\<version_dir>\<real>`，hok-shim-ref 只做 `~\..\` 拼接不受影响）

**注意**：hok 曾无条件创建 junction（现 no_junction 时不建）；select_current_version 是 mtime 回退（no_junction 无 current junction，靠版本目录 install.json mtime）。

**Why**：官方 NO_JUNCTION 语义下 junction 不存在，运行时入口必须直接解析版本目录；不改 link 会导致入口路径失效。
**How to apply**：以后动 no_junction 相关代码，入口统一走 `session.current_dir_name()`，真实版本目录走 `session.versioned_dir()`；不要再引入 `join("current")` 硬编码或 `if config.no_junction()` 手写分支。

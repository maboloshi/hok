---
name: hok-no-junction-implementation
title: Hok no_junction 完整实现（P1-18 配套）
description: 动 no_junction 配置，或改安装/卸载的入口目录解析时读：Session::current_dir_name 与 versioned_dir 的分工，以及 link/shim/metadata/env/脚本 $dir/query/list/cleanup 的全链路接入点。
metadata:
  type: project
---

Hok 的 `no_junction` 配置（config.rs:250，getter :383，JSON key `no_junction`；**没有别名**——`no_junctions` 之类拼写现被显式拒绝，见 config.rs:1012-1019）已完整实现（原为半失效：link 无条件建 junction、shim/shortcut/metadata 硬编码 `current`）。

**核心设计**（session.rs）：
- `Session::current_dir_name(version) -> &str`（session.rs:436）：no_junction 时返回版本号、否则 `"current"`——shims/shortcuts/env/metadata 的**运行时入口目录名**，对齐官方 `currentdir`（core.ps1:355）
- `Session::versioned_dir(name, version)`（session.rs:452，原 `version_dir` 改名）：**真实版本目录** `<app>/<version>`——解压目标、link_current target、卸载删除定位。**语义不可改成 no_junction 感知**：升级场景下 current junction 仍指向旧版本，解压/链接必须用真实版本目录

**全链路改动**（no_junction=true 时）：
- `package/operations/link.rs`：`link_current/unlink_current` 加 `no_junction: bool` 参数，no_junction 跳过 junction 创建/删除（对齐官方 link_current，install.ps1:238）
- `sync_install.rs`：metadata（manifest.json/install.json）写入 `app_dir.join(current_dir_name(version))`（:893-895）；env 旧版本清理用 current_dir_name
- `sync_remove.rs`：pre_uninstall/uninstaller 脚本 `$dir` 用已解析版本目录（解析 :190-193，`script_dir` :211，pre_uninstall 传 :224、uninstaller :272；官方 scoop-uninstall.ps1:55 `$dir=versiondir`）
- `query.rs`：`load_install_state` / `query_installed` 在 no_junction 下经共享解析器 `install_state::resolve_current_version`（install_state.rs:122，mtime 回退 :145）读版本目录 metadata（`query_synced(_cached)` 读的是 bucket manifest / SQLite 缓存，与已安装 metadata 无关）
- `list.rs:46-47` / `cleanup.rs:126-127,316`：no_junction 时用 `resolve_current_version` 替代 `read_link("current")`
- `env.rs`/`shortcut.rs`/`shim.rs`/`psmodule.rs`/`persist.rs`：全部改用 `session.current_dir_name(...)`（hok-shim 本身不做目录解析，不受影响）

**注意**：hok 曾无条件创建 junction（现 no_junction 时不建）；`resolve_current_version` 的 no_junction 分支靠 mtime 回退（没有 current junction）。`join("current")` 的残留只存在于 junction 分支与遗留链清理（session.rs:420-421、install_state.rs:126-174、query.rs:111-117、link.rs:24/42、sync_remove.rs:210/464、cleanup.rs:127），新代码不要引入。

**Why**：官方 NO_JUNCTION 语义下 junction 不存在，运行时入口必须直接解析版本目录；不改 link 会导致入口路径失效。
**How to apply**：以后动 no_junction 相关代码，入口统一走 `session.current_dir_name()`，真实版本目录走 `session.versioned_dir()`，元数据解析统一走 `install_state::resolve_current_version()`；不要再引入 `join("current")` 硬编码或 `if config.no_junction()` 手写分支。

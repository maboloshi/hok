# hyperscoop（`hp`）值得参考的功能盘点

Status: active
Updated: 2026-10-07

> **主题**：同生态的独立 Rust 重实现 `../hyperscoop`（`hp`，`Cargo.toml` = 4.5.1，edition 2024）
> 有哪些功能值得 hok 参考，哪些是反例。
> **方式**：静态读码（2026-10-07）。所有结论带 file:line；**未实测**，动手前按 [[investigate-first]] 复现。
> **结论**：**本会话不立项**。下表是候选池，需要时按项单独立案。
> **配套**：hok 现状以 [`command-deviation-audit-2026-09.md`](../active/audit/command-deviation-audit-2026-09.md)
> 与 [`scoop-gap-status.md`](../active/audit/scoop-gap-status.md) 为准，本文不重复。

## 1. 第一梯队：与「对标官方 Scoop」不冲突、用户价值明确

| # | 功能 | hyperscoop 证据 | hok 现状 | 判断 |
|---|---|---|---|---|
| A1 | **`--dry-run` 贯穿 install / uninstall / cleanup / autoremove** | `../hyperscoop/src/command_args/{install.rs:92,uninstall.rs:55,cleanup.rs:42,autoremove.rs:23}`；`invoke_install` 在演练时连默认 bucket 的 ensure 都跳过 | 全仓 grep 无 `dry_run` | 推荐。安装/卸载是核心流程，`--dry-run` 是成本最低的「先看后做」 |
| A2 | **`uses`：反查谁依赖我**（`-a` 扫全 bucket 清单、`-q` 纯列表、`--installed`） | `../hyperscoop/src/hyperscoop_middle/invoke_uses.rs:12-44`；核心 `../hyperscoop/crates/command/src/depends.rs:56-178`（rayon 并行扫 bucket） | **卸载护栏已有**（`crates/libscoop/src/package/sync_remove.rs:94` 返回 `PackageDependentFound`），缺的是**查询面**：无法事先问「卸了会连带谁」 | 推荐。不是补安全漏洞，是补可视化；`-a` 对清单作者也有用 |
| A3 | **`status --json` / `-q`** 机器可读输出 | `../hyperscoop/src/hyperscoop_middle/invoke_status.rs:120-132` | `status` 只有 `-a`（`src/cmd/status.rs:13`）；仅 `config` 有 `--json` | 推荐。脚本/CI 集成的最小改动 |
| A4 | **`size`：按应用归因磁盘占用**（当前版本 / 旧版本 / persist / cache / 合计） | `../hyperscoop/src/hyperscoop_middle/invoke_size.rs` + `command_util_lib::size` | 只统计 **cache**（`src/cmd/cache.rs:55`），无应用占用视图 | 推荐（中价值）。`cleanup` / `autoremove` 的决策依据 |
| A5 | **`edit`：用 `$EDITOR` / VS Code / Notepad 打开 manifest** | `../hyperscoop/src/hyperscoop_middle/invoke_edit.rs` | 无 | 低成本小工具，可选 |
| A6 | **checkup 补检：shim 不在 PATH / 断链 shim / PATH 过长 / 坏 junction** | `../hyperscoop/src/hyperscoop_middle/invoke_checkup.rs:832,868,927,957` | `checkup` = 环境检查（`crates/libscoop/src/package/diagnostic.rs:193`：主 bucket、长路径、开发者模式、helper、NTFS、Defender）；`health` = 安装记录（`crates/libscoop/src/package/health.rs:51`：install.json / current / manifest） | 推荐。**「shim 与 junction 目标有效性」是两块检查之间的真空** |
| A7 | **跨进程文件锁**（fs2；可重入；被占用时排队等待） | `../hyperscoop/crates/command/src/utils/lock.rs:20-81`（`try_lock_exclusive` 失败 → `lock_exclusive` 阻塞） | 全仓无文件锁（grep `fs2` / `lock_exclusive` 零命中） | 推荐（超出官方 Scoop 的健壮性）。两个并发 install / update 互踩状态是真实风险 |

## 2. 第二梯队：牵动共享状态文件或格式兼容，需单独立案

- **B1 `installed_as_dependency` + `autoremove` / `leaves`**：在 `install.json` 写标记，
  孤儿 = `find_leaf_apps` ∩ 该标记（`../hyperscoop/crates/command/src/depends.rs:377-435`、
  `../hyperscoop/src/hyperscoop_middle/invoke_autoremove.rs`）。
  **张力**：官方 Scoop 不写这个键 ⇒ 动手前先核实官方读 `install.json` 的面
  （多键通常无害，但这是改共享状态文件，不是纯新增命令）。
- **B2 `Hpfile` bundle（dump / install / check / cleanup）**：
  `../hyperscoop/crates/command/src/bundle.rs:31-233`、`../hyperscoop/src/hyperscoop_middle/invoke_bundle.rs`。
  **前置**：hok 的 `export` / `import` 与官方 Scoopfile **schema 互不兼容**是既有 P1
  （偏离审计 §6.4）。**先对齐官方**，再决定要不要引第三种格式；顺序反了就是「用新格式掩盖旧格式的不兼容」（[[no-patchwork-fixes]]）。

## 3. 不建议参考（反例与夸大，价值同样高）

- **`hp service` 是浅实现**：候选服务名**硬编码** 10 个再拼已装应用
  （`../hyperscoop/src/hyperscoop_middle/invoke_service.rs:208-219`），靠 `sc.exe` 输出文本匹配状态，
  `restart` = `stop` + `sleep 800ms` + `start`（`:182-187`）。不是元数据驱动，不宜抄。
- **`hp merge`（删跨 bucket 冗余 manifest）危险**：**直接删除**社区 bucket 工作树里的 manifest
  （`../hyperscoop/crates/command/src/merge.rs:292,366`），版本按**字符串**比较（`:207`），
  `map_container.insert(new_app_versio, merge)` **把版本号当键**（`:208`），
  `insert(app_name, count - 1)` 的判重也不成立（`:367`），且只对硬编码白名单生效（`:171`）。**反模式**。
- **README 宣称与代码不符**（勿据其判断能力）：
  - 无 `memmap`（全仓零命中），与「memory-mapped files」不符；
  - `hp bundle install` 实为**串行** for 循环（`../hyperscoop/crates/command/src/bundle.rs:352-420`），非 parallel；
  - 「自适应带宽」实为**内置 aria2c.exe（5.39 MB）+ 7z（2.9 MB）**，按 100 MB 阈值切
    `--split=5/16`（`../hyperscoop/crates/command/src/install/aria2.rs:226-280`、`resources/`），
    与 hok 不 vendor 第三方二进制的方向相反（见
    [`7zip-switch-investigation-2026-09.md`](../active/audit/7zip-switch-investigation-2026-09.md)）；
  - `CHANGELOG.md` 停在 1.0.4 而 Cargo 是 4.5.1 ⇒ 文档不可当事实源。
- **已有等价物、不必抄**：`switch` ≈ `reset`（hyperscoop 侧也只是列目录 + 调 `reset_specific_version`）；
  `log` ≈ `info` 的 git 历史；`fetch` ≈ `download`；`tap` / `untap` ≈ `bucket add` / `rm`；
  `desc` ≈ `info`；`doctor` ≈ `checkup` 别名；i18n 与并行搜索 hok 已具备。

## 4. 若将来立项的推荐顺序

A6 checkup 补 shim / junction 检测 → A7 跨进程锁 → A1 `--dry-run` → A2 `uses` →
A3 `status --json` → A4 `size`（A5 `edit` 可搭车）；B1 / B2 各自单独立案。

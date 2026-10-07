# hyperscoop（`hp`）值得参考的功能盘点

Status: active
Updated: 2026-10-07

> **主题**：同生态的独立 Rust 重实现 `../hyperscoop`（`hp`，`Cargo.toml` = 4.5.1，edition 2024）
> 有哪些功能值得 hok 参考，哪些是反例。
> **方式**：静态读码（2026-10-07）。所有结论带 file:line；**未实测**，动手前按 [[investigate-first]] 复现。
> **结论**：**本会话不立项**。下表是候选池，需要时按项单独立案。
> **口径**：命令面差异分四种性质（§2），**只借「纯新增命令」**，不改同名命令语义、不引别名、
> 不开前缀推断。
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

## 2. 命令面差异的四种性质（对照官方 `libexec/*.ps1` 的 28 个命令）

「差异大」不是一个量，而是四种性质不同的东西；**只有第一种可以放心借**。

| 类别 | 具体 | 证据 |
|---|---|---|
| **保留同名**（21） | alias / bucket / cache / cat / checkup / cleanup / config / export / home / import / info / install / list / prefix / reset / search / shim / status / uninstall / update / which | `../hyperscoop/src/command.rs:61-115` |
| **官方名降级为别名**（仍可用，主名换了） | `download`→`fetch`、`hold`→`pin`、`unhold`→`unpin`、`bucket`→`tap` | `../hyperscoop/src/command.rs:64,77,80,106` |
| **真改名（官方名不再可用）** | `depends` → `deps`（无 `depends` 别名；`infer_subcommands` 只做前缀推断，`depends` 不是 `deps` 的前缀） | `../hyperscoop/src/command.rs:73`、`../hyperscoop/src/command_args/deps.rs` |
| **直接缺失** | `create`、`virustotal`，以及 `bin/*.ps1` 整套清单维护工具链（checkver / checkhashes / checkurls / missing-checkver / formatjson） | 无对应命令模块 |
| **纯新增**（约 20） | autoremove / bundle / completion / desc / edit / leaves / link / log / merge / missing / reinstall / self_update / service / shellenv / size / switch / unlink / untap / uses / credits | 同左 |

另有一条硬缺口：**`hp help` 不可用**（`../hyperscoop/src/main.rs:74` = `disable_help_subcommand = true`），
而官方 `scoop help <cmd>` 是文档与脚本最常引用的入口。

性质与风险：

- **(a) 纯新增命令** —— 不破坏任何 Scoop 脚本，风险低。**这是唯一该借的部分**（`uses` / `size` /
  `autoremove` / `--dry-run`）。
- **(b) 别名与改名** —— 调用不破，但**文档面与肌肉记忆分流**，等于长期维护两套名字；hok 已是官方名，
  引别名是纯负债。
- **(c) 改同名命令的语义** —— 真正的兼容风险，比「多一个命令」严重得多：`uninstall` 会**拒绝**卸载被依赖
  的包（`../hyperscoop/src/hyperscoop_middle/invoke_uninstall.rs:44-81`，官方直接卸）、`cleanup` 无参 =
  清全部。面向 `scoop` 的脚本、文档、CI 判断会静默失效。
- **(d) 全局解析策略** —— `infer_subcommands = true` + `infer_long_args = true`
  （`../hyperscoop/src/command.rs:55`）是**前向兼容陷阱**：今天唯一的前缀缩写可能因明天新增命令
  变成歧义或被抢走，即**已发布的脚本行为会被后续版本改变**。这一条最不该学。

⇒ **对 hok 的口径**：同名命令的语义以官方为基线，**新增只能"加命令"，不能"改同名命令语义"**
（即 hok 偏离审计 A/B/C/D 分类里的立场）。要动 (c) 类，走「有意偏离（已登记）」流程并进偏离审计，
不得夹在新增功能里。hyperscoop 恰是反过来的：改了 `uninstall` / `cleanup` 的语义（C 类且未登记），
同时砍掉了 hok 的 D 类（清单维护工具链）——两者受众不同，**它做终端用户，hok 兼顾清单作者与 CI**。

## 3. 第二梯队：牵动共享状态文件或格式兼容，需单独立案

- **B1 `installed_as_dependency` + `autoremove` / `leaves`**：在 `install.json` 写标记，
  孤儿 = `find_leaf_apps` ∩ 该标记（`../hyperscoop/crates/command/src/depends.rs:377-435`、
  `../hyperscoop/src/hyperscoop_middle/invoke_autoremove.rs`）。
  **张力**：官方 Scoop 不写这个键 ⇒ 动手前先核实官方读 `install.json` 的面
  （多键通常无害，但这是改共享状态文件，不是纯新增命令）。
- **B2 `Hpfile` bundle（dump / install / check / cleanup）**：
  `../hyperscoop/crates/command/src/bundle.rs:31-233`、`../hyperscoop/src/hyperscoop_middle/invoke_bundle.rs`。
  **前置**：hok 的 `export` / `import` 与官方 Scoopfile **schema 互不兼容**是既有 P1
  （偏离审计 §6.4）。**先对齐官方**，再决定要不要引第三种格式；顺序反了就是「用新格式掩盖旧格式的不兼容」（[[no-patchwork-fixes]]）。

## 4. 不建议参考（反例与夸大，价值同样高）

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

## 5. 若将来立项的推荐顺序

A6 checkup 补 shim / junction 检测 → A7 跨进程锁 → A1 `--dry-run` → A2 `uses` →
A3 `status --json` → A4 `size`（A5 `edit` 可搭车）；B1 / B2 各自单独立案。

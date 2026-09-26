# hok ↔ Scoop 差距分析（2026-08-15，对标 Scoop v0.5.3）

> 方法：双子代理并行实读（Scoop 原版 libexec/lib 源码 + hok src/cmd/libscoop 当前代码）+ P 项状态核实。**结论以当前代码为准**，历史 gap 技能只作线索。

## 1. 命令覆盖

| 维度 | 数量 | 说明 |
|---|---|---|
| Scoop 原版 | 28 | libexec/scoop-*.ps1 |
| hok | 36 | 覆盖 Scoop 全部 28 个（含 help） |
| hok 独有 | 8 | checkhashes、checkurls、checkver、formatjson、missing_checkver、completions、reinstall、upgrade、ci-auto-pr（9 个） |

命令名全覆盖 ✓（Scoop 无 hok 缺的命令）。

## 2. 参数差距（Scoop 有而 hok 缺）

| 命令 | Scoop 参数 | hok 现状 | 差距 | 优先级 |
|---|---|---|---|---|
| **virustotal** | `-a/--all`、`-s/--scan`、`-n/--no-depends`、`-u/--no-update-scoop`、`-p/--passthru` + **组合退出码**（0/1/2/4/8/16） | 仅 app 列表，无选项 | ❌ 缺 5 选项 + 退出码体系 | 中 |
| **shim** | `-g/--global`、子命令 add/rm/list/info/**alter**、`--` 参数终止符 | 仅 list/info | ❌ 缺 -g/alter/add/rm | 中 |
| **depends** | `-a/--arch <arch>` | 无选项 | ❌ 缺 -a/--arch | 低 |
| **export** | `-c/--config`（导出配置，剥离 last_update/root_path/global_path/cache_path/alias） | 仅 -a/--all | ❌ 缺 -c/--config | 中 |
| **status** | `-l/--local`（禁用远程 fetch） | 仅 -a | ❌ 参数不同 | 低 |
| **info** | `-v/--verbose`（显示完整路径/URL/persistdir） | 无选项 | ❌ 缺 -v | 低 |
| **reset** | `-a/--all` | 无（仅可选 version） | ❌ 缺 -a/--all | 低 |
| **hold/unhold** | `-g/--global` ✓ + **`hold scoop` 设 hold_update_until 明天** | -g ✓，无 hold scoop 特例 | ⚠️ 缺 hold_update_until 机制 | 低 |
| **update** | `-q/--quiet`（命令级） | 全局 -q 有，命令级无 | ⚠️ 部分覆盖 | 低 |
| install/update/download | `-u/--no-update-scoop` | 无 | N/A（hok 单二进制无自更新，设计决策 ✓） | — |

**hok 参数增强**（Scoop 无）：search `-e/-B/-D`、list `-H/-V`、download `-a/--arch`、checkhashes `-k/-c`、checkver `-V/-t`、export `-a/--all` 等。

## 3. 配置键差距（Scoop 34 键 vs hok ~23 键）

**hok 缺的键：**

| 键 | Scoop 语义 | hok 现状 | 优先级 |
|---|---|---|---|
| `use_lessmsi` | .msi 用 lessmsi 解压 | 无（hok 解压链无 msi 处理？） | 低 |
| `use_external_7zip` | 外部 7z.exe 优先 | 无显式键（hok 有 7z.exe fallback 硬编码） | 低 |
| `show_manifest` | 安装前打印 manifest + 确认 | 无 | 低 |
| `show_update_log` | 更新后显示 changelog | 无 | 低 |
| `update_nightly` | nightly 按日期比较自动更新 | 无 | 低 |
| `force_update` | 版本不等即视为过期 | 无（hok --force 是 CLI 参数） | 低 |
| `hold_update_until` | hold scoop 到明天 | 无 | 低 |
| `autostash_on_conflict` | git pull 冲突时 stash | 无（hok 用 merge 冲突返回，语义不同） | 低 |
| `debug` | 调试日志开关 | 无（hok 用 RUST_LOG/--detail） | 低 |
| `aria2-warning-enabled` | aria2 警告开关 | 无 | 低 |
| `aria2-retry-wait` | aria2 重试等待 | 无 | 低 |
| `aria2-options` | aria2 透传参数 | 无 | 低 |
| `scoop_branch`/`scoop_repo` | 自更新源 | N/A（无自更新） | — |
| `shim` | shim 实现选择 | N/A（原生 hok-shim） | — |

**hok 独有键**：`editor`（新）、`language`、`no-color`、`ignore-failures`。

**已对齐的键**：aria2-enabled/split/max-connection/min-split-size、cache_path、cat_style、default_architecture、gh_token、global_path、ignore_running_processes、last_update、no_junction、private_hosts、proxy、root_path、use_isolated_path、use_sqlite_cache、virustotal_api_key、alias ✓

## 4. 核心流程差距

### 4.1 install
| Scoop 步骤 | hok | 状态 |
|---|---|---|
| ensure versiondir → 下载 → 解压 | sync_install commit_one_install（extract→script→link→shim→persist→metadata） | ✅ |
| pre_install / post_install 钩子 | operations/script.rs run_hook | ✅ |
| installer.file/args + keep | run_installer_file + keep | ✅ |
| **ensure_install_dir_not_in_path** | env.rs + sync_install.rs | ✅ **已实现**（P2 项覆盖） |
| **persist_permission（全局 ACL）** | 无 | ❌ 缺 |
| show_manifest 交互 | 无 | ❌ 缺 |
| suggest/notes 展示 | suggest 已实现 | ✅ |

### 4.2 update
| 项 | Scoop | hok | 状态 |
|---|---|---|---|
| update <app> 先同步 buckets | ✅（Sync-Bucket 在 app 更新前） | 无 | ❌ **P1-9 未做** |
| cooldown | **3 小时**（is_scoop_outdated） | **15 分钟** | ⚠️ 不一致（hok 特有值） |
| --force 语义 | 强制重装 + 旧目录改名 `_<v>.old` | SyncOption::Force ✓ | ✅ |
| 单应用更新 = 完整重装 | pre_uninstall → 卸载各步 → install_app | 走 sync upgrade 流程 | ⚠️ 需核实 |
| hold 跳过 | install.json hold=true | escape_hold 语义 ✓ | ✅ |
| nightly | update_nightly 配置控制 | nightly 跳过 hash ✓ | ⚠️ 无 update_nightly 键 |
| -q/--quiet | 隐藏"已是最新"等 | 全局 -q | ⚠️ 部分 |

### 4.3 download
- ✅ 已对齐（P2 项）：download_apps 逐 query 独立、-f/-s/-a、事件闭环 DownloadDone、失败 exit 非零、url_remote_filename 缓存命名
- ⚠️ Scoop 有 `-u/--no-update-scoop`（N/A）

### 4.4 checkup 语义差异
- **Scoop checkup**：环境检查（管理员+Defender 性能、main bucket、长路径、开发者模式、7zip/innounp/dark 工具、NTFS 卷）
- **hok checkup**：~~已装包健康检查（install.json/manifest.json 完整性）~~ → **2026-09-13 已修（S5，`5ae994d`）**：`checkup` 对齐官方环境检查（恒 exit 0）；原已装包体检迁为新命令 `health`。调研见 `.agents/docs/reports/s5-checkup-investigation-2026-09.md`

## 5. P 项状态核实（更新 gap 技能）

| 项 | 技能记录 | 本次核实 |
|---|---|---|
| P0-1..6 | ✅ 已修 | ✅ 确认 |
| P0-3 env_set/cookie string 限制 | ⏸ 未修 | ✅ **2026-08-16 复核已修**：env_set×3 + cookie 字段均挂 `deserialize_string_map`（manifest.rs:92-97/170/331/389），number/bool 自动转字符串；测试 test_env_set_coerces_non_string_values（manifest.rs:1784-1800） |
| P1-7/8 | ✅ | ✅ |
| P1-9 update <app> 先同步 buckets | ⏸ 暂缓 | ⏸ 仍未做 |
| P2 download | ✅ | ✅ |
| P2 install_psmodule/ensure_install_dir_not_in_path/installer.keep/post_install $dir | ⏸ 未核对 | ✅ ensure_install_dir_not_in_path 已实现；其余已随 no_junction/uninstall 对齐覆盖 |
| P3 #10 成功提示 / #20 hash 前缀 | 剩 | ✅ 已修（2026-08-13「Everything is ok!」；hash.rs:202-217 format_hash_value 单点实现） |
| P3 #15 -q/--quiet | 剩 | ⚠️ 全局有，命令级无（install/update.rs 均无 quiet） |
| P3 其他（文案/退出码/cooldown 等） | 剩 | ⚠️ 部分（#9 download 退出非零已修、全局仍 0/1；cooldown config.rs:465 仍 900s） |

## 6. 优先级建议

**值得修（真实兼容缺口，中优先级）**：
1. `virustotal` 参数 + 组合退出码（Scoop 明确定义 0/1/2/4/8/16）
2. `export -c/--config`（Scoop 功能，简单）
3. ~~`checkup` 语义对齐（当前与 Scoop 完全不同——需决策：对齐 Scoop 环境检查 or 保留 hok 健康检查语义并改名）~~ ✅ **2026-09-13 已修（S5，`5ae994d`）**：两者并存（`checkup`=官方环境检查、`health`=已装包体检）
4. P1-9 update <app> 先同步 buckets（技能建议项）

**✅ 2026-08-16 已完成**：`depends -a/--arch`（depends.rs，复用 session.set_default_architecture）、`reset -a/--all`（reset.rs，遍历 local+global、跳过 scoop、global 无 admin warn）、`persist_permission`（persist.rs grant_global_permission，icacls 设 Users SID Write+ObjectInherit 到全局 persist 根，挂 install+reset）、P0-3 env_set/cookie 非 string 值（deserialize_string_map）。
**❌ 用户决策不做（2026-08-16）**：`shim alter` 子命令。

**可暂缓（低/设计决策）**：
- config 缺键多为边缘（aria2 细节/show_manifest/update_nightly 等）
- info -v/status -l（小参数补全）
- ⚠️ 已知既有 bug：`hok reset <app>` 对本机 Scoop 混装环境报 os error 183（junction::create 路径已存在；官方 Scoop attrib +R 标记的 junction 非管理员无法删）。待修。

**明确不做**：-u/--no-update-scoop、scoop_branch/scoop_repo、shim 键（hok 单二进制设计）

## 7. 细节核实：config 边缘键 / persist_permission / 小参数（2026-08-15 复核，代码级）

> 本节把 §2/§3/§4 中"低优先"三类缺口落到 file:line 细节，供后续处理直接取用。
> 官方语义出处：本地 `Scoop/libexec/scoop-config.ps1`（键文档）、`Scoop/lib/install.ps1`（persist_permission）、`Scoop/lib/core.ps1`（hold_update_until）等。

### 7.1 config 边缘键（缺失集合 = 15 个；"12"是去 N/A 后的口径）

**hok 侧机制**：`ConfigInner`（config.rs:158-287）只建模支持键；`Config::set()` 未匹配键返回 `ConfigKeyInvalid`（config.rs:693）；测试 `unsupported_keys_rejected`（config.rs:1401-1424）锁定 12 个被拒键（use_external_7zip/use_lessmsi/scoop_repo/scoop_branch/show_update_log/show_manifest/aria2-warning-enabled/aria2-retry-wait/aria2-options/debug/force_update/shim）；另有 `update_nightly`/`hold_update_until`/`autostash_on_conflict` 3 个同样未建模（不在测试内）。首次迁移时 Scoop 的不支持键被 serde 静默丢弃（config.rs:106-115）。修入口 = ConfigInner 加字段 + `set()` match 加 arm + `list_all` 加行，单文件闭环。

| 键 | 官方语义（出处） | hok 现状 | 真实缺口? |
|---|---|---|---|
| `use_external_7zip` | PATH 外部 7z.exe 解压，找不到则 abort（decompress.ps1:89） | 原生解压（internal/archive.rs），无外部 7z 概念 | 是（解压引擎差异） |
| `use_lessmsi` | MSI 用 lessmsi 而非 msiexec | 无 | 低 |
| `show_manifest` | 安装前展示 manifest + 交互确认（scoop-install.ps1） | 无安装前 review 交互 | 是（交互） |
| `show_update_log` | update 时显示 scoop/bucket git 提交日志（默认 true，scoop-update.ps1:62,397-413） | update 流程不展示 git 日志 | 低 |
| `update_nightly` | nightly-yyyyMMdd 仅次日自动更新，否则须 --force（scoop-config.ps1:117-119） | nightly 仅跳过 hash 校验（download_verify.rs:51-52），无更新门禁键 | 低 |
| `force_update` | 配置级强制 app 更新到 bucket 版本 | 仅命令级 -f/--force（checkver/update/install） | 低 |
| `hold_update_until` | hold scoop 自更新到指定日期；scoop hold scoop 设明天（core.ps1:1181-1197） | 单二进制无自更新，无消费方 | 设计 N/A |
| `autostash_on_conflict` | bucket 更新遇 git 冲突自动 stash，否则中止（默认 false） | git pull 无 stash 处理 | 是 |
| `debug` | 详细输出开关 | tracing 无键接线 | 低 |
| `aria2-warning-enabled` | 下载 aria2 警告开关（默认 true，scoop-install.ps1:130-133、scoop-update.ps1:449-452） | 无 aria2 警告概念 | 低 |
| `aria2-retry-wait` | 重试等待秒（默认 2，download.ps1:347） | 分片重试参数内置（download_fragmented.rs） | 低 |
| `aria2-options` | 附加 aria2 选项数组（download.ps1:380） | 原生分片下载，无 aria2 二进制 | 设计 N/A |
| `scoop_repo`/`scoop_branch` | 自更新源 | 无自更新 | 明确不做 |
| `shim` | shim 构建选择（kiennq/scoopcs/71） | 原生 hok-shim | 明确不做 |

### 7.2 persist_permission（全局 persist 目录 Users 写 ACL）

**官方**（`Scoop/lib/install.ps1:522-531`）：当 `$global -and $manifest.persist -and (is_admin)` 时，对全局 persist 根目录（`persistdir $null $global`）Set-Acl 加 **Users 组（SID `S-1-5-32-545`）Write 权限（ObjectInherit）**。调用点：install 流程（install.ps1:67）+ reset 流程（scoop-reset.ps1:91），幂等。目的：ProgramData 下的 persist 目录默认仅管理员可写，普通用户运行全局 app 时写不了自己的持久化数据。

**hok**：`persist::link`（persist.rs:31-92）仅三态逻辑 + junction/hardlink，**无任何 ACL 步骤**；install 管道只调 `persist::link`（sync_install.rs:533-535）。基础件已就位：`Session::is_admin`（session.rs:197-202 → internal/os.rs:55-71，Shell32 `IsUserAnAdmin` FFI）。补齐 = 设 DACL（windows-sys `SetNamedSecurityInfoW` 或 icacls 子进程），挂 install 的 persist 步 + reset 流程。低优先（管理员+全局窄场景）。

### 7.3 小参数（5 个 CLI 缺口）

| 命令 | 缺口 | 官方语义（出处） | hok 现状（file:line） | 工作量 |
|---|---|---|---|---|
| `depends` | `-a/--arch` | 按指定架构解析依赖树（scoop-depends.ps1:10,17，Format-ArchitectureString） | cmd/depends.rs:12-15 仅包名位置参数，架构隐式默认 | 小 |
| `status` | `-l/--local` | 仅检查本地已装 app，跳过 scoop/bucket 远程 fetch（git 缺失自动降级，scoop-status.ps1:16-17） | cmd/status.rs:9-14 仅 `-a/--all` | 小 |
| `info` | `-v/--verbose` | 完整路径（current/version/persist dir）、manifest 文件路径、license URL、已装体积细分、未装时下载大小（scoop-info.ps1:53-59,137,149-196） | cmd/info.rs:9-14 无 `-v` | 中 |
| `reset` | `-a/--all` | `*` 或 `-a` 重置全部本地+全局 app（跳过 scoop、全局需 admin，scoop-reset.ps1:16,22-26,38-41,60-63） | cmd/reset.rs:9-15 仅单 app + 可选 version | 小 |
| `hold` | `hold scoop` 特例 | `scoop hold scoop` 写 `hold_update_until`（明天）而非 install.json held 标记；格式错则报错并移除（core.ps1:1181-1197） | cmd/hold.rs 一律 install.json held 标记，无 scoop 特例 | 中（需 7.1 的 hold_update_until 键联动） |

> 注：`virustotal` 5 参数 + 组合退出码是中优先级独立项（cmd/virustotal.rs:15-19 仅 app 列表），不在本节。
> 归档说明：本会话前已完成/被取代的报告（FINAL-REPORT-2026-08.md、FINAL_AUDIT.md、analysis-report.md、framework-fix-plan.md、framework-review-2026-08.md、manifest-parse-audit.md + 4 个 txt 分析产物）已移入 `.agents/docs/reports/`（索引见该目录 README.md）；本文档为活文档，现随 `docs/` 目录迁入 `.agents/docs/`。

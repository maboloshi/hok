# 上游 Scoop 0.5.3 → 0.6.0 影响评估与本仓处置

Status: active
Updated: 2026-10-02

> **是什么**：本地官方源码 `../Scoop` 于 2026-10-02 从 v0.5.3（`b588a06e`，2025-08-12）快进到 v0.6.0（`e6aa3b36`，2026-09-30），共 **76 个提交**。本文是这些改动对 hok 的影响逐项评估与本会话的处置记录。
> **口径**：官方侧一律引用 `../Scoop` 的 file:line（v0.6.0）；hok 侧引用本仓 file:line。**本节为静态读码 + 少量实测**，未做端到端复现的条目标注为「未实测」。
> **与旧审计的关系**：`command-deviation-audit-2026-09.md`、`scoop-gap-analysis-2026-08.md`、`scoop-gap-status.md` 的对标口径是 **0.5.3**，其结论在其口径内仍有效，但**不再是当前上游**；本文覆盖 0.5.3 → 0.6.0 的增量。

## 0. 一句话结论

P0 一项（元数据文件名）是**单向互操作破坏**，会随 0.6.0 普及自然爆掉；P1（checkver GitHub 模式）是**一个整体的行为移植**，hok 现有测试反而锁定着上游已删除的旧行为；其余为新增键/新增能力缺失与文案级差异。

## 1. P0 — 元数据文件名（必改）

`145381fc` 把落盘元数据改名，并加了「新名优先、旧名回落」的读链：

| 项 | 官方（v0.6.0） | hok（改前） |
|---|---|---|
| 清单写到 | `scoop-manifest.json`（`lib/manifest.ps1:128-131`） | `manifest.json`（`crates/libscoop/src/package/sync_install.rs:898`） |
| 安装信息写到 | `scoop-install.json`（`lib/manifest.ps1:146`） | `install.json`（`sync_install.rs:970`） |
| 读清单 | 新名优先、旧名回落（`lib/manifest.ps1:134-139`） | 只读旧名（`install_state.rs:190`、`query.rs:118`、`manifest_source.rs:256`、`sync_remove.rs:696`） |
| 读安装信息 | 新名优先、旧名回落（`lib/manifest.ps1:149-154`） | 只读旧名（`install_state.rs:189`、`query.rs:119,217`、`manifest_source.rs:209`、`hold.rs:49`、`list.rs:82`、`export.rs:251`、`info.rs:605`、`health.rs:102`） |
| 版本解析 | `current/scoop-manifest.json` 回落 `current/manifest.json`（`lib/versions.ps1:54-56`） | 只读 `current/manifest.json`（`install_state.rs:126`） |
| 版本枚举 | 两名皆 glob（`lib/versions.ps1:99-100`） | 只按 `install.json` 的 mtime（`install_state.rs:153`） |

**方向性**：官方能读 hok 写的旧名（兼容），**hok 读不到官方 0.6.0 写的新名** ⇒ `list` / `status` / `update` / `hold` / `uninstall` 会把该应用当作未安装或损坏。

**实测（2026-10-02，本机 `D:\App\Scoop` 已升级到 v0.6.0）**：74 个已装应用的 `current\` **仍全是 `install.json,manifest.json`** ⇒ 官方**不做存量迁移**，只在新增/升级/重装时写新名。所以现在还没爆，但**任何应用被 0.6.0 重装/升级后**即触发。

**处置（用户 2026-10-02 拍板）**：hok **写新名 + 读新名优先回落旧名**，与官方落盘一致。代价：旧版 Scoop（< 0.6.0）会读不到 hok 新装的包——上游自己也接受这个取舍。

## 2. P1 — checkver GitHub 模式（一个整体移植）

上游四个提交是**一次行为变更**，必须一起移植；hok 现有测试恰好锁定被删掉的旧行为。

| 上游 | 官方行为 | hok 改前 | 判定 |
|---|---|---|---|
| `cbd7c64b!` | API 模式默认 `$jsonpath='$.tag_name'` + 正则 `(?:v\|V)?([\d.-]+)`；HTML 模式默认正则 `/releases/tag/(?:v\|V)?([\d.-]+)`（`bin/checkver.ps1:180-191`） | `checkver.rs:139` 默认正则为 `([\d.]+)`（无 `-`）；`resolve_checkver_source` 从不注入默认 jsonpath、无 API/HTML 分流（`:530-600`） | **偏离**；带 token 时 hok 会拿 HTML 正则解析 API JSON ⇒ 取不到版本 |
| `35334422` | 校验 `$inputGithubUrl`（homepage 或 `checkver.github`）对 `^https://((www\.)?github\.com/…\|api\.github\.com/repos/…)$`，不匹配则报错并**跳过该 manifest**；非 api 才追加 `/releases/latest`；token 重写不再受 `$useGithubAPI` 门控（`bin/checkver.ps1:155-191`） | `checkver.rs:549-556` 只查 `starts_with("https://github.com/")` 且只出非致命 `pre_message`；map 形式完全不校验；`:589-593` 重写仍被 `use_github_api` 门控 | **偏离** |
| `bdf63444` | `checkver.github` 的 `$useGithubAPI` 改为无条件（删掉 `PSObject.Properties.Count -eq 1` 门） | `checkver.rs:560-563` 仍有 `if cv.github_solo` 门；测试 `:1899-1918`、`:3090-3118` 断言这个已被删除的行为 | **偏离（旧门残留）** |
| `d452e8f6` | `$.tag_name` 默认仅在**没有** `checkver.script` 时注入（`bin/checkver.ps1:182-184`） | `checkver.rs:278` `jsonpath = cv.jsonpath.clone()`，本就无默认 | **偏离（与首条同根）** |
| `20a178b1` | fetch 失败但存在 `checkver.script` 时，警告后**仍执行脚本**（`bin/checkver.ps1:307-321`） | `checkver.rs:336-339` 直接返回 `CheckverFetch::DownloadError`，脚本分支（`:340-364`）永远到不了 | **偏离 / 待做** |
| `c9f0728f` | `$originurl = strip_fragment $url` 同时喂 5 个版本变量与 github 模式的 jsonpath 字面量（`lib/autoupdate.ps1:214-218,303`） | `Substitution::with_url`（`checkver.rs:1165-1181`）已去 fragment；但 `checkver_hash.rs` 的 github jsonpath 用**原始 url** | **偏离**（`#/dl.7z` 永不匹配 asset，静默回落下载） |
| `bd2e93df` | `find_hash_in_json` 仅在 URL 匹配 `^https?://api\.github\.com/.*` 且有 token 时加 `Authorization: Bearer` + `X-GitHub-Api-Version: 2022-11-28`（`lib/autoupdate.ps1:121-124`；实际条件以源码为准，CHANGELOG 还提到 HTTPS 要求） | 对**任意** json hash URL 都传 token（`internal/network.rs` 的 opts 覆盖）；从不发 `X-GitHub-Api-Version` | **偏离**（token 可能泄露给非 GitHub 主机） |

**已对齐（无需动）**：`3bee8159`（substitute 最长键优先，`checkver.rs:1107-1121` 已按长度倒序）、`3a79532d`（`$page` 跨迭代；hok 每次取出都是新值，结构上免疫）、`874e8e6f`（脚本无输出时的返回；hok 空页即报错、不返回旧版本）、`79669717`（`hash.mode: github`；`checkver_hash.rs:172` 早已分发，且 hok 不随仓带 `schema.json`）。

## 3. P2 — 新增能力/新键缺失（本会话未排期）

| 上游 | 官方行为 | hok 现状 | 影响 |
|---|---|---|---|
| `63cb8f84` | 新增 config 键 `use_git_history`（默认 `$true`）+ SQLite→git→autoupdate 的历史清单链（`lib/manifest.ps1:214-250`、`libexec/scoop-config.ps1`） | 无该键（`crates/libscoop/src/config.rs` 白名单内只有 `use_sqlite_cache`）；无 git 历史 provider | `hok install app@<旧版本>` 对 HEAD 中已不存在的版本无解 |
| `db8d554f` | bucket pull 改为 `git pull --tags --force -q`（`libexec/scoop-update.ps1:139`） | libgit2 fetch，refspec 只含 heads（`internal/git.rs:146-172`），**从不取 tag** | 与上一条互锁：按 tag 找历史清单天然不可用 |
| `198818ae` | `scoop update -f` 把 `@version` pin 升到 bucket HEAD；`$pin_broken`、`Find-AppBucket` 重解析（`libexec/scoop-update.ps1:270-285,443-451`） | pin/URL 安装即 isolated；`install_state.rs:235-241` 判定其不可升级，`-f` 只是原地重装 | `hok update -f` 不会升到 bucket HEAD |
| `e02633c7` | 配置键 `aria2-fallback-disabled` 反转为 `aria2-fallback-enabled`（`libexec/scoop-config.ps1:142`） | 两个名字都不在白名单 | 用户设置时报 `ConfigKeyUnsupported`（比"未知键"清楚，但不支持） |
| `327040cf` | `scoop download` 任一失败即 `exit 1`（`libexec/scoop-download.ps1` 末尾 `exit [int]$download_failed`） | `download_apps.rs:104-117` 汇总后**恒 `Ok(())`** | 退出码偏离；且 `scoop-gap-status.md` 里「download 失败按官方改为 0」的口径已过期（**更正**：0.6.0 起官方为 1） |

## 4. P3 — 零散差异（本会话在主树做）

| 上游 | 官方行为 | hok 现状 |
|---|---|---|
| `4c55e7ae` | 卸载脚本里 `$bucket = $install.bucket`，URL 安装为 `$null`（`libexec/scoop-uninstall.ps1:62`） | 脚本 `$bucket` 取自 `package.bucket()`（`operations/script.rs:242,291,756`），isolated 时是哨兵 `__isolated__` |
| `7e86336a` | 增删环境变量有可见输出（`lib/install.ps1:341,352`） | 事件存在但只走 `tracing::debug!`（`src/scoop_handler.rs:168-179`），且事件不带 name/value |
| `b592b38a` | notes 前后各有 `Notes` / `-----` 分隔（`lib/install.ps1:355-362`），建议行 `DarkYellow`（`:421`） | 只发 notes 正文（`sync_install.rs:872-880` → `scoop_handler.rs:243-245`）；建议行用普通 `output::info`（`src/cmd/install.rs:138-142`） |

## 5. 判定为无影响 / 已对齐（不必动）

`3bee8159`、`aa3f9aa9`、`25d35007`、`2120c28c`、`ff38652e`、`d9b4d28a`、`2a637361`、`03d831d9`、`a8d4027a`、`a5c3b58e`、`7a537586`、`41ccd2c0`、`ca0506cf`/`0e698ccb`（输出函数化：仅 `Error.`/`Done.` 大小写与 `error` 前缀）、`0f8cd488`（WoW64 路径重写只对 x86 shim 生效，hok-shim 的 stub 是 x64 PE ⇒ 不适用）、`52a035b5`（hok `internal::path::normalize_path` 已解析 `..`，`is_in_dir` 归一化语义等价）。

## 6. 本会话处置与排期

| 优先级 | 内容 | 落点 |
|---|---|---|
| P0 | 元数据文件名（写新名 + 读新名优先回落旧名 + 版本解析/枚举双名） | worktree `.worktrees/p0-metadata-names`，分支 `feat/metadata-scoop-prefix`，base `22b9e68` |
| P1 | checkver GitHub 模式整体移植（上表 7 条 + 改掉锁定旧行为的测试） | worktree `.worktrees/p1-checkver-github`，分支 `fix/checkver-github-mode`，base `22b9e68` |
| P3 | `$bucket` 语义、环境变量输出、notes 分隔/高亮、两个新 config 键登记、`download` 退出码 | 父会话主树直接改（`locales/*` 是必冲突热点，不派分支） |
| 未排期 | P2 全部（历史清单 + `--tags` + `update -f` pin 提升） | 转入 [`pending-tasks.md`](../plan/pending-tasks.md) |

**门禁策略（用户 2026-10-02 选定）**：每条子分支自测一次（`cargo fmt --check` / `clippy -D warnings` / 目标测试），落地后父会话在最终树上跑一次完整门禁；不在每条分支重复跑完整套。

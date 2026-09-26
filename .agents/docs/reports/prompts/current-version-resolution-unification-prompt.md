# 提示词：统一「当前版本解析」单入口（依赖合并 A1）+ 官方 Scoop 对照

> **执行状态（2026-09-12）：已落地。** 落地方式与结论见
> `.agents/docs/active/audit/optimization-audit-2026-09.md` §10（含官方对照表、§3 三处设计修正、§10.4 有意不跟随项）。
> 与本文件原设计的偏离，逐条在 §10.2：① 不加 `CurrentVersionPolicy`（改为返回 `CurrentVersion { version, source }`）；
> ② `Session::current_dir_name` 本就不解析版本，未接入；③ `sync_install.rs` 改走 `versioned_dir` 而非解析器。
> 关键澄清：`NO_JUNCTION` 开关在官方是 `Select-CurrentVersion` **函数内**分支（`versions.ps1:53`），调用方一律无条件调用——
> hok 的缺陷是丢了这个 `if`，不是"不该无条件调"。**§4 的对照实验已实跑并全部固化进测试。**

> **用法**：在新会话里把本文件作为首要输入（`@.agents/docs/reports/prompts/current-version-resolution-unification-prompt.md`）。
> 这是一份**自包含**交接：新会话没有本议题的历史上下文，全部依据写在这里。
> **前置加载的 skill**：`hok-code-organization`、`no-patchwork-fixes`、`local-scoop-source-path`、`hok-no-junction-implementation`、`hok-rust-gate-practice`、`git-commit-practice`、`hok-proactive-review`、`token-efficient-processing`。

---

## 0. 一句话目标

把散落在 5 处的「当前版本 / 当前目录解析」合并为 libscoop 内的**唯一入口**，并且**逐条对齐官方 Scoop 的语义（含它自己的怪癖）**——对齐结论决定合并后的行为，不允许"合并时顺手选一种"。

来源：`.agents/docs/active/audit/optimization-audit-2026-09.md` 之后的依赖归并分析（A1 项）。

## 1. 现状：同一条规则被抄成 4 种写法（已实测，勿重新论证）

规则原文：「junction 模式读 `current`，`NO_JUNCTION` 走 `Select-CurrentVersion`」。

| 位置 | 读法 | 与其他处的差异 |
|---|---|---|
| `crates/libscoop/src/package/install_state.rs:50` `select_current_version` | `current/manifest.json` → nightly 时 `read_link` → `install.json` mtime 兜底 | **无条件**读 `current\manifest.json`（不看 `no_junction`） |
| `crates/libscoop/src/package/list.rs:41-49` | `no_junction` ? `select_current_version` : `read_link(current)` | 自己又写了一遍 `read_link` |
| `crates/libscoop/src/package/cleanup.rs:56-62` | 同上（第三份 `read_link`） | 同上，且是内联闭包 |
| `crates/libscoop/src/package/query.rs:95-110` | `no_junction \|\| !current_ok` → `select_current_version` | 多一个"坏 current"分支 + `current_broken` 标记（`!no_junction && !current_ok`） |
| `crates/libscoop/src/package/sync_install.rs:669-676` | 直接拼 `apps/<name>/<old_version>` vs `session.current_dir()` | 绕开上面所有判定 |

另有 `crates/libscoop/src/session.rs:334` `current_dir()` / `:359` `current_dir_name()`（**构造**路径用，不是解析），以及 `crates/libscoop/src/package/install_state.rs:84` `load_install_state`（:93 内联 `if no_junction`）、`crates/libscoop/src/package/query.rs:209`。

**危害**：三条路径对"同一个包当前是哪个版本"可能给出**不同答案**（`read_link` 看得见 nightly 重定向、`select_current_version` 靠 manifest、mtime 兜底是第三种）。只在边界（nightly、中断升级留下的坏 current、NO_JUNCTION 下的陈旧 `current` 目录）显现 —— 最难查的一类 bug。

## 2. 官方对照（本地副本：`D:\App\github项目\Scoop\Scoop`）

必读四处（**先读源码，再写任何代码**）：

| 官方位置 | 语义 |
|---|---|
| `lib/core.ps1:355-362` `currentdir($app)` | `if (get_config NO_JUNCTION) { $version = Select-CurrentVersion } else { $version = 'current' }` |
| `lib/versions.ps1:31-68` `Select-CurrentVersion` | ① `$currentPath = apps\<app>\current`；② **仅当 `!(NO_JUNCTION)`** 时 `parse_json "$currentPath\manifest.json"`，version 为 `nightly` 时取 `(Get-Item $currentPath).Target \| Split-Path -Leaf`；③ 若 `$currentVersion` 仍为 `$null` → `Get-InstalledVersion` 取 `@(...)[-1]` |
| `lib/versions.ps1:71-105` `Get-InstalledVersion` | `Get-ChildItem "$appPath\*\install.json" \| Sort-Object -Property LastWriteTimeUtc` → 取 `.Directory.Name`，再过滤掉 `current` 与 `-notlike '_*.old*'`（注释明写：**oldest → newest，最后一个是最新**） |
| `lib/install.ps1:238-258` / `:265-280` `link_current` / `unlink_current` | `NO_JUNCTION` 时**直接返回 versiondir**（不建/不删链接）——hok 的 `package/operations/link.rs` 已对齐 |

### 2.1 必须在动手前逐条实验判定的疑点

| # | 疑点 | 为什么重要 |
|---|---|---|
| a | 官方在 `NO_JUNCTION` 下**完全跳过** `current\manifest.json` 读取（`versions.ps1:53` 的 `if`），而 hok 的 `select_current_version` 第 1 步**无条件**读 | 若存在陈旧 `current` 目录（例如手工留下、或从 junction 模式切过来），官方忽略、hok 会读它 → 版本判定分叉 |
| b | nightly 重定向官方**只在 junction 模式**处理（`versions.ps1:55-57`）；hok 无模式区分 | 同上 |
| c | 官方排序键是 `install.json` 的 `LastWriteTimeUtc`（UTC）；hok 用 `std::fs::metadata().modified()`（本地时基） | 排序单调性一致，但必须用测试固定"取最新"的语义，不能靠"应该一样" |
| d | 官方过滤 `-notlike '_*.old*'` vs hok `starts_with('_') && contains(".old")`（`install_state.rs:70`） | 边界名（`_foo.old.bak`、`_fooold`、`_.old`）是否等价需逐例验证 |
| e | 官方 `parse_json` 解析失败时 `warn` 并返回 `$null`（`lib/manifest.ps1:5-12`）→ 继续走 mtime 兜底；hok 用 `parse_file(...).ok()` **静默** | 用户可见差异：要不要 warn 属于"行为对齐"决定，不能夹在 refactor 里偷偷改 |

## 3. 合并方案（要求，命名可调整但必须专一）

原设计（**未采用**，理由见 §10.2 修正 1）：

```rust
pub(crate) enum CurrentVersionPolicy { Default, AllowBrokenCurrent }
pub(crate) fn resolve_current_version(
    pkg_dir: &Path,
    no_junction: bool,
    policy: CurrentVersionPolicy,
) -> Option<String>;
```

实际落地：

```rust
// crates/libscoop/src/package/install_state.rs —— 唯一入口
pub(crate) enum CurrentVersionSource { CurrentManifest, NightlyJunction, VersionDirs }
pub(crate) struct CurrentVersion { version: String, source: CurrentVersionSource }
pub(crate) fn resolve_current_version(pkg_dir: &Path, no_junction: bool) -> Option<CurrentVersion>;
pub(crate) fn current_meta_dir(pkg_dir: &Path, no_junction: bool) -> Option<(PathBuf, CurrentVersionSource)>;
```

- 5 处调用点全部改调它：`list.rs`、`cleanup.rs`、`query.rs`（含 `broken_package`）、`sync_remove.rs`（`uninstall` + `reset` 两处）、`install_state.rs`。`sync_install.rs` 改走 `versioned_dir`（那里是已知版本号的路径构造，不是解析）。
- `no_junction` 开关进**函数内部**（对齐 `versions.ps1:53`）；调用方不得再自己 `if no_junction` 分叉出第二套解析。
- `query.rs` 的"允许坏 current"由 `source == VersionDirs && !no_junction` 判定，不用布尔位/策略枚举。
- `Session::current_dir_name`（构造路径）保留原样；它不解析版本，也无需复用解析逻辑。
- **禁止**（`no-patchwork-fixes`）：给旧函数加 `bool`/开关把两种语义塞在一起。
- 放置位置：`install_state.rs`（`select_current_version` 的家），未新建模块。

## 4. 对照实验（**第一步就做，结果写进提交说明**）

夹具：临时 `apps/<app>/` 目录，5 种形态各一份：

1. junction 正常（`current` 指向 `1.2.3`，两处 `manifest.json`/`install.json` 齐全）
2. nightly junction（`current\manifest.json` 的 `version == "nightly"`，链接指向 `1.2.3-nightly`）
3. 坏 current（`current` 存在但无 `manifest.json`，另有一个 `install.json` 更新的版本目录）
4. `NO_JUNCTION` + **陈旧 `current` 目录**（关键：验疑点 a）
5. `NO_JUNCTION` + 仅两个版本目录（验 mtime 取最新）

官方侧：`powershell.exe`（Windows PowerShell 5.1）下 dot-source 官方脚本后调用 `Select-CurrentVersion` / `Get-InstalledVersion`（`core.ps1` 依赖 `$scoopdir`/`$globaldir` 等全局，需先赋值；若加载链过重，退化为**按源码逐条列出期望值表**并写进测试注释，别硬凑）。

hok 侧：同一夹具跑新入口，逐例比对。**差异不允许"以后再对齐"** —— 要么改 hok，要么在提交说明里写下有意偏离的理由与登记到 `scoop-gap-audit` 记忆。

## 5. 验收条件

1. 上述 5 种夹具形态 **× 官方对照结论** 都有对应测试（现有可复用的：`list.rs`、`query.rs:614`、`sync_remove.rs:812` 一带的 `no_junction` / 坏 current 用例）。
2. `cargo test -p libscoop` 全绿；`cargo test --workspace` 无新增失败（基线 515 passed；`internal::git::tests` 在全局 `core.autocrlf=true` 下有已知偶发失败）。
3. `cargo check --workspace --all-targets` + `cargo clippy --workspace --all-targets`（仅允许 vendored `innospect` 既有 4 条）。
4. 全文无第二处 `read_link(... "current")`（`grep -rn 'join("current")'` 复查）。
5. **行为零变化或按对照结论显式变化**：提交说明必须包含"官方对照表 + 每一项的判定"。

## 6. 陷阱

- **不要顺手改 warning/日志**（疑点 e）：要不要 warn 单独立项，别夹带。
- 本仓 `no_junction` 的既有设计约定见 skill `hok-no-junction-implementation`（运行时入口统一走 `current_dir_name`）——合并时要与它一致，不要造第二套"当前目录"概念。
- **不要整包 `cargo fmt`**：仓库有既有 fmt diff，只 fmt 本次改动文件（`rustfmt --edition 2024 <files>`）。
- git 一律 `-c core.autocrlf=false`；本机 `commit.gpgsign=true`。
- 不要动 vendored `crates/innospect`。

## 7. 建议提交拆分

1. `test(libscoop): pin current-version resolution against Scoop`（先把对照结论固化成测试，此时允许暂时失败或用 `#[ignore]`）
2. `refactor(libscoop): resolve the current version through one entry`（5 处替换 + 删重复 `read_link`）

## 8. 如果判定不做

在 `.agents/docs/active/audit/optimization-audit-2026-09.md` 追加一节，并 `hindsight_ingest_document` 一条更正（写明"四处重复已确认但决定保留 + 理由"），避免下一轮重新论证。

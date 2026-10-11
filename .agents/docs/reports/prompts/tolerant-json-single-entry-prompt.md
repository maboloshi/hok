# 提示词：统一「宽容 JSON 解析」单入口（依赖合并 A2）+ 官方 Scoop 对照

> **用法**：在新会话里把本文件作为首要输入（`@.agents/docs/reports/prompts/tolerant-json-single-entry-prompt.md`）。
> 这是一份**自包含**交接：新会话没有本议题的历史上下文，全部依据写在这里。
> **前置加载的 skill**：`hok-code-organization`、`no-patchwork-fixes`、`local-scoop-source-path`、`scoop-gap-audit`、`hok-write-json-trailing-newline`、`hok-rust-gate-practice`、`git-commit-practice`、`hok-proactive-review`、`token-efficient-processing`。
>
> ⚠️ **已执行（2026-09-12，提交 `7f4a6ae` + `4e7a6c2`）**。本文件保留为历史交接，**执行时不要照抄**：
> §2.1 的「hok 是官方 PS 5.1 的有意超集」结论**是错的**（`{"a":01}` 两宿主都接受、hok 拒绝，实测表见下）
> 且 §6.2 验收条件不可达。已落地的口径、复评修正表（6 处 ❌）与验收证据见
> `.agents/docs/active/audit/optimization-audit-2026-09.md` §11。

---

## 0. 一句话目标

把仓库里 **4 处「去 BOM + 宽容解析 JSON」实现、3 种错误策略** 合并为**唯一入口**（实现唯一、策略显式），并且用**官方 Scoop 的调用点策略**逐一对齐；同时把"宽容到什么程度"从我方注释里的模糊说法（"like PowerShell 7's ConvertFrom-Json"）改成**有对照实验支撑的显式契约**。

来源：依赖归并分析 A2 项（`.agents/docs/active/audit/optimization-audit-2026-09.md` 之后的 A1/A2/A3 议题）。

## 1. 现状：4 处实现、3 种策略（已实测，勿重新论证）

| 位置 | 解析链 | 错误策略 |
|---|---|---|
| `crates/libscoop/src/config.rs:132` `read_json_file` | strip BOM → `serde_json::<Value>` → **回退 `json5`** | 回退也失败时**报严格错误**（:139-141） |
| `crates/libscoop/src/package/manifest.rs:783` `parse_manifest_spec` | strip BOM → `serde_json::<ManifestSpec>` → 回退 `json5` | 报严格错误（有测试 `bom_prefixed_manifest_parses` :2200、`json5_manifest_parses_as_fallback` :2211） |
| `crates/libscoop/src/package/formatjson.rs:55` `format_manifest_file` | strip BOM → **只走 `json5`**（无严格优先） | 报错（带路径） |
| `crates/libscoop/src/package/formatjson.rs:126` `schema_warnings` | strip BOM → 只走 `json5` | **静默返回空 vec**（吞错） |

BOM 剥离同型代码共 **6 处**：上述 4 处 + `crates/libscoop/src/package/operations/psengine.rs:437`、`crates/libscoop/src/package/operations/script.rs:42`。

注释层面的"逻辑近似"已经自我暴露：`config.rs:129-131` 写着 "Tolerant of a leading UTF-8 BOM … and, as a fallback, JSON5 … matching the manifest parse tolerance"，`manifest.rs:774-790` 写着 "notepad UTF-8 with BOM … unlike a JSON5 parse error"。

**隐患**：同一个畸形文件在 `formatjson`（只 json5，宽容）与 `manifest`（严格优先）下可能得到不同结论；`schema_warnings` 还会静默吞错 —— 三种策略没有一处写着"为什么"。

## 2. 官方对照（本地副本：`D:\App\github项目\Scoop\Scoop`）

官方**只有一个解析实现**（PowerShell 的 `ConvertFrom-Json`），宽容度由宿主提供；**差异全在调用点的错误策略上**：

| 官方位置 | 调用形态 | 错误策略 |
|---|---|---|
| `lib/manifest.ps1:5-12` `parse_json` | `Get-Content $path -Raw -Encoding UTF8 \| ConvertFrom-Json -ErrorAction Stop` | catch → `warn "Error parsing JSON at '$path'."` → 返回 `$null` |
| `lib/manifest.ps1:14-32` `url_manifest` | 远端文本 `\| ConvertFrom-Json -ErrorAction Stop` | catch → `warn` → `$null` |
| `lib/core.ps1:82-95` `load_cfg` | `[System.IO.File]::ReadAllLines($file)`（**自动探测编码**，含 BOM）`\| ConvertFrom-Json -ErrorAction Stop` | catch → `Write-Host "ERROR loading $file: …"`（**不返回对象**） |
| `lib/json.ps1:130`（json_path） | `$json \| ConvertFrom-Json -ea stop` | 抛 |
| `lib/buckets.ps1:40` | `Get-Content $json -Raw \| ConvertFrom-Json -ErrorAction stop` | 抛 |
| `lib/database.ps1:175` | `ConvertFrom-Json $manifestRaw -ErrorAction SilentlyContinue` | **静默** |
| `libexec/scoop-search.ps1:96` | `[System.IO.File]::ReadAllText(...) \| ConvertFrom-Json -ErrorAction Continue` | **打印但继续** |

**注意**：官方的宽容度随宿主变化（`ConvertFrom-Json` 在 5.1 与 7.x 上实现不同）——所以"模仿 PowerShell"本身不是一个稳定契约。见下节实测。

### 2.1 上手先做的对照实验（本机已实测，直接引用）

用 base64 传参规避引号问题，在 `powershell.exe`（5.1.26100）与 `pwsh`（7.6.5）上各跑一遍：

| 输入 | Windows PowerShell 5.1（Scoop 原始宿主） | PowerShell 7.6.5 |
|---|---|---|
| `{"a":1,}` 尾逗号 | **FAIL** `Invalid JSON primitive: .` | OK |
| `{"a":1 // c}` 行注释 | **FAIL** | 需换行；本机用例构造不当，请重测 |
| `{"a":1 /* c */}` 块注释 | **FAIL** | OK |
| `{'a':1}` 单引号键 | OK | OK |
| `{a:1}` 无引号键 | OK | OK |
| `{"a":01}` 前导零 | OK | OK |
| `[1,2,]` 尾逗号数组 | **FAIL** `extra trailing ','` | OK |

**结论（写进提交说明与代码注释）**：hok 现在的 `json5` 回退**比官方在 PS 5.1 宿主上更宽容**（5.1 拒绝尾逗号与注释）。因此合并的正确目标不是"更像 PowerShell"，而是：

1. 实现唯一：`internal::json` 一个入口；
2. **策略显式**：枚举 `JsonErrorPolicy::{Strict, Tolerant, Silent}`（名字可调），每个调用点自报，并在注释里写明它对应官方的哪个调用点；
3. **显式声明宽容度**：把 `json5` 回退当作 hok 的**有意超集**（消掉 bucket 里手工 JSON 的常见笔误），并在注释里列出上表作为依据 —— 不再写"matching PowerShell 7"这类不可验证的话。

## 3. 合并方案（要求）

```rust
// crates/libscoop/src/internal/json.rs（新）
pub fn strip_bom(text: &str) -> &str;
pub enum JsonErrorPolicy { Strict, Tolerant, Silent }   // 命名可调，语义必须可读
pub fn parse_tolerant<T: DeserializeOwned>(text: &str, policy: JsonErrorPolicy) -> Fallible<T>;
pub fn parse_file_tolerant<T: DeserializeOwned>(path: &Path, policy: JsonErrorPolicy) -> Fallible<T>;
```

- 4 处调用点替换；`psengine.rs` / `script.rs` 的 BOM trim 也改调 `strip_bom`（它们是行级输出，不是 JSON，可只共享 `strip_bom`）。
- 策略映射（初版，**必须逐条与 §2 表格核对后定稿**）：
  - `manifest.rs` → `Tolerant`（对应 `parse_json` 的 warn→null？注意 hok 是 Err 上抛，是否改 warn 属行为变更，单独立项）
  - `config.rs` → `Tolerant`
  - `formatjson.rs:55` → `Tolerant`
  - `formatjson.rs:126` → `Silent`（对应官方 `-ErrorAction SilentlyContinue` 的那一类"尽力而为"场景）
- **禁止**（`no-patchwork-fixes`）：用一个 `bool strict_first` 参数把两种语义合进一个函数却不说明用途；策略必须是**能读懂的枚举**，让 `grep Silent` 就能找出所有吞错处。
- **不要**在本次顺手改 `json5` 依赖本身或 `serde_json` 的 `preserve_order` 特性。
- 写侧不动：`write_json` 的 CRLF + 末尾换行约定见 skill `hok-write-json-trailing-newline`，本题只处理读侧。

## 4. 迁移清单

1. 新增 `internal/json.rs`（含 `strip_bom` 单测：BOM/无 BOM/仅 BOM/中间 BOM 不动）。
2. `config.rs:132`、`manifest.rs:783`、`formatjson.rs:55`、`formatjson.rs:126` 替换。
3. `psengine.rs:437`、`script.rs:42` 改用 `strip_bom`。
4. 更新 `manifest.rs:774-790`、`config.rs:129-131`、`formatjson.rs:29` 的注释，改为"依据 §2.1 对照表"。
5. `libscoop/src/lib.rs` / `internal/mod.rs` 导出（`internal` 是 crate 内部模块，按既有可见性约定走）。

## 5. 对照实验（必做，结果进提交说明）

1. 用 §2.1 的 7 个片段做**三方对照**：`serde_json` 严格 / `json5` 回退 / 两个 PowerShell 宿主（`powershell.exe`、`pwsh` 各跑一次；脚本模板用 base64 传参，避免引号地狱）。
2. 把结果表落成**一个测试**（`tests/` 或模块内 `#[cfg(test)]`），断言 hok 的宽容度**有意的超集**，防止未来无意收紧/放宽。
3. 现有 oracle 必须继续绿：`config.rs:1595 load_tolerates_bom_config`、`manifest.rs:2200/2211`、`formatjson.rs` 的批量/容错用例（`formatjson.rs:465`、`:603`）。

## 6. 验收条件

1. 4 处实现归 1，`grep -rn "json5::from_str"` 只剩 `internal/json.rs` 一处。
2. `grep -rn "trim_start_matches" ` 全仓只剩下 `strip_bom` 内部一处（`psengine.rs` / `script.rs` 一并改走该函数）。
3. 宽容度对照表有测试固定。
4. 门禁：`cargo check --workspace --all-targets` + `cargo clippy --workspace --all-targets`（仅允许 vendored `innospect` 既有 4 条）+ `cargo test --workspace`（基线 515 passed）。
5. 提交说明含"官方调用点 ↔ hok 策略"对照表。

## 7. 陷阱

- **不要**把"策略"做成默认参数让调用点省略：省略 = 又回到隐性语义。
- `formatjson.rs:126` 的吞错是**有意的**（它只是产出 warning），合并时保留吞错但显式化，别顺手改成 `?`（会让 `formatjson` 因一个坏文件整批失败）。
- **不要**在本次改 `-ErrorAction` 级别的用户可见行为（比如给 manifest 解析加 warn）——那是独立议题，需单独对照官方后决定。
- **不要整包 `cargo fmt`**：只 fmt 本次文件（`rustfmt --edition 2024 <files>`）。
- git 一律 `-c core.autocrlf=false`；`commit.gpgsign=true`。
- 不要动 vendored `crates/innospect`（它自己有 `json5`/`bitflags` 一套，与本题无关）。

## 8. 建议提交拆分

1. `feat(libscoop): add a single tolerant-JSON entry with an explicit error policy`
2. `refactor(libscoop): parse config, manifests and formatjson through that entry`
3. `test(libscoop): pin the JSON tolerance superset against PowerShell hosts`

## 9. 如果判定不做

把"4 处实现 + 3 种策略"的实测表与不做理由回写 `.agents/docs/active/audit/optimization-audit-2026-09.md` 追加节，并 `hindsight_ingest_document` 一条更正。

# 桶扫描类命令的输出调研（checkver 与 4 个兄弟）

Status: closed
Updated: 2026-10-05

> **这是什么**：`checkver` / `checkhashes` / `checkurls` / `formatjson` / `missing-checkver`
> 五个「扫 bucket 出报告」命令的现行输出盘点，为 2026-10 的 `checkver` 输出重做提供基线。
> **范围**：本次只做 `checkver`；另外四个**不处理、不排期**，留作后期专项。
> **方向已定**，规则本体在 [`output-system-design.md`](../active/spec/output-system-design.md) §十一；
> 本文只放证据与差距，**不复述规范**。
> **口径**：官方侧引用 `../Scoop`（v0.6.0）`file:line`；hok 侧引用本仓 `file:line`；
> 数据为 2026-10-05 静态阅读源码所得（未跑实测）。

## 1. 结论

1. 五个命令是**五套结果行语法 + 五套汇总句式 + 两种"发现"去向**（stdout / stderr）。
   统一规范的收敛方向见 spec §十一；本文负责说明"各差多少"。
2. **只有 `checkver` 是并发的**（`buffer_unordered`），另外四个是顺序 `for` 循环。
   因此 spec §11.6 的「手动点名 → 清单序 / 默认 `*` → 完成序」这条规则，
   **在当前实现下只对 checkver 有实际区别**；对顺序命令两者恒等，接上并发那天才生效。
3. `checkurls` 与 `missing-checkver` 的表格形状是**上游逐字复刻**（代码注释直接引上游行号），
   必须排除在统一之外，否则就是新增有意偏离。
4. 顺带发现两处**真实错位**（不是审美问题，见 §4.2 / §4.4）。

## 2. 现状矩阵

| 命令 | 并发 | 结果行 | 明细行 | 汇总 | 发现行去向 |
|---|---|---|---|---|---|
| `checkver` | ✅ | `  name (old -> new)...` + 第二行 `可用 autoupdate 更新...` | — | ❌ **没有** | stderr |
| `checkhashes` | ❌ 顺序 | `✓ name` / `ERROR name` | `ERROR     <msg>` | ✅ | stderr |
| `checkurls` | ❌ 顺序 | `[3][3][0] name` | `        > msg (url)` | ✅ | **stdout** |
| `formatjson` | ❌ 顺序 | `  Formatting <path>...` | — | ✅ | stderr |
| `missing-checkver` | ❌ 顺序 | `[C][A] name` | — | ✅ | stderr |

五种汇总句式（`locales/en.yml`）：

| 行 | key | 文本 |
|---|---|---|
| `locales/en.yml:146` | `checkhashes_summary` | `Scanned %{total}: %{passed} ok, %{failed} failed, %{updated} updated.` |
| `locales/en.yml:159` | `checkurls_summary` | `Checked %{total} URLs: %{valid} valid, %{invalid} invalid.` |
| `locales/en.yml:188` | `formatjson_count` | `Formatted %{count} manifest(s).` |
| `locales/en.yml:240` | `missing_checkver_scan` | `Scanned %{total} manifests: %{missing} missing checkver, %{noauto} missing autoupdate, %{unparsable} unparsable.` |
| — | checkver | **缺** |

## 3. 必须排除：上游逐字复刻的两张表

| 位置 | hok | 上游锚点 |
|---|---|---|
| `checkurls` 的 `[U]RLs [O] [F]` 表头 + `[n][n][n] name` 行 | `src/cmd/checkurls.rs:38-43,54-73` | `../Scoop/bin/checkurls.ps1:41-50,113-140` |
| `missing-checkver` 的 `[C]heckver \| [A]utoupdate` 表头 + `[C][A] name` 行 | `src/cmd/missing_checkver.rs:36-40,59-73` | `../Scoop/bin/missing-checkver.ps1:31-37,44-51` |

两处都带"逐字复刻"的代码注释（`missing_checkver.rs:31-35`、`checkurls.rs:35-37`）。
要换成符号行就是**新增有意偏离**，须先登记进
[`command-deviation-audit-2026-09.md`](../active/audit/command-deviation-audit-2026-09.md)——该审计当前把
`missing-checkver` 记为 A 类（语义不同，含"输出格式不同"）、`checkurls` 记为 D 类（hok 独有命令）。

## 4. 逐命令证据

### 4.1 checkver

- 渲染：`src/cmd/checkver.rs:84-101`。版本行走 `output::status`（dim + 缩进 + 省略号），
  可更新时**另起一行**印 `autoupdate_available`；错误走 `output::err`（stderr）。
- 第二行是冗余：`autoupdate_available` 的判据（`crates/libscoop/src/package/checkver.rs:505-507`）
  与"版本大于当前且有 autoupdate"完全同义，已由结果行本身表达。
- 错误文案缺可操作信息：`cmd.checkver_no_extract` = `could not extract version`
  （`locales/en.yml:158`），而官方给的是
  `couldn't match '$regexp' in $source`（`../Scoop/bin/checkver.ps1:408`）。
- 并发无上限：`buffer_unordered(pending.len().max(1))`
  （`crates/libscoop/src/package/checkver.rs:433`）——整 bucket 几百个请求齐发。官方同样不限，
  但官方没有 30s 超时叠加。
- 排放顺序：Phase 2 用 `collect()` 收齐（同文件 `:434`），Phase 3 按**完成顺序**回放；
  清单顺序来自 `discover_matching` → `discover` 的 `files.sort()`
  （`crates/libscoop/src/package/manifest_walker.rs:52`），命令行给的 app 顺序不参与。
- 静默成功：整轮零可报告项时**什么都不打印**，与"崩了"无法区分。
- `-s/--skip-updated` 已经是"只输出可更新或错误"的语义
  （`crates/libscoop/src/package/checkver.rs:512-516` 在 `ver == current` 时 `continue`），
  与官方 `-SkipUpdated`（`../Scoop/bin/checkver.ps1:419-420`）一致；
  **需求"加个参数只输出可更新的或错误的"无需新增开关**，缺的是帮助文案。

> 官方 `checkver.ps1` 另有一处已知 bug：`function next($er)` 打印的是**全局** `$App`
> 而非局部 `$app`（`../Scoop/bin/checkver.ps1:274-277`），多个位置参数时会出现
> `360zip: wpsoffice-ai-cn (scoop version is …)` 这类张冠李戴。hok 不复制该行为。

### 4.2 checkhashes（有真实错位）

- 渲染：`src/cmd/checkhashes.rs:93-116`。`Passed` → `✓ name`；`Updated` → `✓ name: hash mismatch! updated` + `WARN` 明细；
  `Failed` → `output::err(&item.name)` 后接 `output::err(format!("    {msg}"))`。
- **错位**：`output::err` 会拼 `ERROR ` 前缀，所以明细实际渲染成 `ERROR     <msg>`
  （`src/cmd/checkhashes.rs:109-113`）——4 空格缩进被前缀吃掉，明细比主体更抢眼。
  按 spec §11.1 应为 2 空格缩进 + dim 的**无前缀**明细行。
- 官方对照：`$app: OK` / `$app: Mismatch found`（`../Scoop/bin/checkhashes.ps1:142-147`）、
  `Writing updated <app> manifest`（同文件 `:184`）。
- 汇总：`src/cmd/checkhashes.rs:118-124` 用 `output::info`（dim），**总是打印**——与 spec §11.4 一致，无需改。

### 4.3 checkurls

- 渲染：`src/cmd/checkurls.rs:54-77`，逐 manifest 一行 `[URL数][ok数][fail数] name`，
  错误明细 `        > msg (url)`（8 空格 + `>`）。
- 汇总：同文件 `:82-91`，0 清单时打 `checkurls_no_manifests`。
- 它是**唯一**把发现行走 stdout 的（同文件 `:35-37` 的注释已把理由写清："报告就是该命令的全部输出"）。
  spec §11.3 是把这条规矩推广到其余四个。
- 表格形状见 §3，不动。

### 4.4 formatjson（有真实错位）

- 渲染：`src/cmd/formatjson.rs:34-50`。错误/警告走 stderr；每个被改写的文件走
  `output::status(hok_i18n::t!("cmd.formatjson_writing", path = …))`。
- **错位**：`output::status` 是"进行中"的形状——dim + 2 空格缩进 + 自动补省略号
  （`src/output.rs:326-334`），文案还是进行时 `Formatting <path>...`
  （`locales/en.yml:187`）；而这行打完时文件**已经写完**。
  按 spec §11.1 应为 `✓ <path>`（completed）。
- 汇总：同文件 `:52-59`，0 项时 `formatjson_none` 兜底——已是 spec §11.4 的形状。

### 4.5 missing-checkver

- 渲染：`src/cmd/missing_checkver.rs:59-73`，每行 `[C][A] name`；警告走 stderr。
- 汇总：同文件 `:75-84`，全齐时补 `missing_checkver_all`。
- 表格形状见 §3，不动。
- 已登记偏离：`-s/--supported` 与官方 `-SkipSupported` **语义相反**，
  见 [`command-deviation-audit-2026-09.md`](../active/audit/command-deviation-audit-2026-09.md) §7.4。

## 5. 后期专项的边界

- **要做**：`checkhashes` / `formatjson` 换成 `output::result` 结果行（顺带修 §4.2 / §4.4 两处错位）；
  `checkurls` / `missing-checkver` 只对齐"发现行去向"与"汇总行模板"，**表格形状不动**。
- **不做**：给顺序命令加并发——它们不在本次方向里；真加了，spec §11.6 才生效。
- **共享原语先落**：`output::result` 与 `RowUI::println` 随 checkver 一并实现
  （spec §11.8），兄弟命令在各自专项里改用，本次不预重构。
- 本文为一次性快照，落地后如需更新结论，应新开调研或改 spec 本体，**不要回来改本文**。

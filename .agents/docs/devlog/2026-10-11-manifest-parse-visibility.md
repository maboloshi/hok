# 清单解析失败不再静默消失：显式查询逐条 warn + 全量扫描一行汇总

> 2026-10-11 ｜ 提交 `628645b` ｜ worktree `.worktrees/h3-parse-visibility` ｜ 分支 `fix/manifest-parse-visibility`

计划项 H3（P0-4 难定位的根因）。用户按报告 §7 拍板 **D1–D5 全取推荐值**（O2a+O3 组合）：
①显式查询逐条 warn + 全量扫描仅一行汇总；②退出码保持 1；③缓存侧至少覆盖"缓存行不可用"；
④80 个解析缺口另立工作流；⑤文案用官方原文 + reason。**退出码与既有错误语义不变**，只让失败可见。

## 改法（先在并行段收集纯数据，出循环再 emit）

- **文件扫描**（`query.rs` 原 `:287`）：rayon worker 只搬运 `ParseFailure`；出循环后
  非通配 → **按名探测**（每名每桶一次有界读，不扫全桶）每包一条 warn；通配 → **恰好一行汇总**
  （`cmd.manifest_parse_failed_summary`），逐条路径用 `Output::Detail`（= `-vv`）。
  限流 key `parse:{bucket}/{name}`。
- **缓存回退**（原 `:389-390`）：`entry_to_manifest` 失败→读文件；文件也失败→同套可见性。
  显式查询另按名探测覆盖**缓存无行**的情况（坏清单根本不会入库）。
- **缓存显式命中 verify**：原先"文件存在但坏了"被误报成 phantom / `cache_missing`
  （"文件已不存在"）；现区分坏文件（解析失败 warn）与真删除（仍 Phantom），有单测钉住。
- **`checkver.rs` / `checkurls.rs`**：解析失败由 `Err(_) => continue` 改为
  `session.output().warn(...)`（一次扫描一条）。
- **`manifest.rs`**：非 UTF-8 早退分支补上同款 tracing `warn!`（该分支原先绕过 `:837` 的 warn，属补漏）。
- **`install_state.rs`**：拆出 `bucket_manifest_path`（`pub(crate)`，行为不变），warn 由调用方发。
- i18n 两文件同步加 `cmd.manifest_parse_failed` / `cmd.manifest_parse_failed_summary`。
- **不动**：`internal::json` 的宽容度契约、`parse_file` 仍返回 `Err`（"比 Scoop 更严"是已登记的有意偏离）。

## 官方对照（本机 `../Scoop`）

| 官方 | hok 本次 |
|---|---|
| `lib/manifest.ps1:5-12` `parse_json` warn 后返回 `$null` | 仍 `Err`（**有意偏离**，未夹带改回）；补同款文案 |
| `bin/checkver.ps1:100-103` 解析失败即跳过 | 同，但补一条 warn |
| `bin/checkurls.ps1:37-38,96-136` warn 后仍打印 `[0][0][0] name` | warn 同、**不造** 0/0/0 行（不伪造 report 项） |
| `libexec/scoop-install.ps1:104-114` warn 后再报 not-found | 同结构；**退出码保持 1** |
| `libexec/scoop-search.ps1:62-76,107-121` 全量扫描只 `debug` ⇒ 官方默认静默 | **有意向上偏离**（O3）：一行计数汇总 |
| `lib/database.ps1:199-200` DB 路径 `SilentlyContinue` | 缓存扫描计数不可用行（更响） |

## 证据

- 改后：`search -e broken` → 1 条官方句 warn + `No matches found`，**exit 1**；`info`/`install` 同；
  `search *` → 恰好一行汇总、exit 0；`-vv` 追加路径行；`checkver`/`checkurls` → 官方句；
  **`search -e good`（桶里同时有坏清单）与干净桶逐字节相同**。
- 改前对照：用同一个新测试跑旧代码 → 5 个用例 **4 FAIL / 1 PASS**（旧代码 0 条 warn、0 行汇总、
  0 条 `-vv` 路径行、0 条 checkver 警告），"无关坏清单不改变 healthy 输出"新旧都 PASS。
- 门禁：`clippy --workspace --all-targets` 0、`test --workspace` 0、`test --test manifest_parse_visibility_cli` 5/5、
  9 个改动源文件 + 新测试 `rustfmt --check` 0。

## 未覆盖 / 登记

- **`install_state` 三个点仍静默**（另立条目 MS1）：`resolve_current_version` 坏清单静默回落
  `newest_version_dir`（**可能给出另一个版本**）、`load_install_state` 当"未安装"、
  `maybe_fill_upgradable` 当"不可升级"。要给共享解析器加失败通道并贯穿 `Session`，调用点跨 7+ 模块。
- `hok update` populate 侧的汇总（D3 可选）未做；缓存写入端的 `Err(_) => Ok(None)` 保持，可见性由查询侧承担。
- 逐条路径展开落在 `-vv`（`Output::Detail` 的既有语义），未新增 `-v` 档位；官方句在 zh 下随 reason 本地化
  （en 仍是官方原文，与项目 i18n 约定一致）。
- 报告 S3（已安装侧 `query.rs:121`）**实测不静默**：`list` 恒带 `QueryOption::Broken`，坏安装显示为 install-failed，故不动。

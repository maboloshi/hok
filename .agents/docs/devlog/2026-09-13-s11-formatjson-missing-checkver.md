# S11 第一批

> 2026-09-13 ｜ formatjson + missing-checkver，已闭环，3 提交，分支 `fix/s11-formatjson` → 已线性 ff 进 `refactor/edition-2024`，tip `2a18d5b`

- **formatjson 子命令名真 bug**：`FormatJson` 未写 `#[clap(name)]`，clap kebab 成 `format-json` ⇒ `hok formatjson` unrecognized subcommand（与官方 `bin/formatjson.ps1`、hok 自身 i18n 键、`AGENTS.md` 三处矛盾）。改为 `name = "formatjson"` + `alias = "format-json"`。
- **共享管线**：`internal::string::matches_any_glob` 按官方 `Get-ChildItem -Filter "$App.json"` 语义 = **大小写不敏感 + 忽略 `.json` 后缀**；单入口 `manifest_walker::discover_matching`，`checkhashes`/`checkver`/`checkurls`/`formatjson`/`missing_checkver` 共用。
- **派生宏共享 bug（勿踩）**：`hok-i18n-derive` 用 **kebab 化的变体 ident** 调 `mut_subcommand`，而 clap 按**最终注册名**匹配 ⇒ 一旦变体显式改名，help 阶段直接 abort。改为先读变体 `#[clap(name="…")]`、无则回落 kebab ident；`ci-auto-pr` 是同类潜在坑。
- **missing-checkver 完全对齐官方**：`-s/--skip-supported`（**语义反转，破坏性**：原"只列有 checkver 或 autoupdate 的" → 现"跳过两者都有的"）；默认输出官方三行图例 + 逐行 `[C][A] name` 真值表 **并保留 hok 汇总行**（`Scanned N manifests: x missing checkver, y missing autoupdate, z unparsable.`，N = 成功解析数）；不可解析清单出行 `[ ][ ] name` + 官方 `Error parsing JSON at '<path>'.`（官方会把 `bad.json` **截成空文件**，hok 保持原样）；app 过滤落地为**位置参数 `[APP]...`**（官方 `bin/missing-checkver.ps1:13` 为 positional，且 hok 的 checkurls/checkhashes/checkver/formatjson/virustotal 全部 positional；`-a` 是 usage error，有测试钉住）。
- **有意偏离（保留）**：`formatjson` 的 `-d` 默认 `.` / schema 告警 / 成功计数输出 / **解析失败保持原文件** / 跳过 `package.json`；`missing-checkver` 的 `-d` 默认 `.` / 跳过 `package.json`（**这是与官方真值表唯一的行差异**：官方多一行 `[ ][ ] package`）/ 行序按路径稳定排序（官方 `Get-ChildItem -Recurse` 不排序）。
- **破坏性变更**：`missing-checkver -s` 语义反转；默认输出形态变更；`matches_any_glob` 变宽容后 `checkver`/`checkhashes`/`checkurls` 的 app 过滤同样放宽（仅放宽不收紧）。
- **遗留（按用户使用面定档，2026-09-13）**：`ci-auto-pr`（§7.7）**要做**——用户**在 CI 里用它更新清单**，且它写盘并提交、受 `-d` 写盘护栏约束；`checkurls`（§7.3）**暂缓**（用户用得少，只读扫描、差异多为输出/边界）；`virustotal`（§7.5，含退出位掩码）**不做**（用户基本不用），登记为不修。**余项清单与交接说明见 `reports/command-deviation-fix-plan-2026-09.md` §8**。

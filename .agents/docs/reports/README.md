# 报告归档（Reports Archive）

> **本目录是冻结区**：存放已完成 / 已失效 / 已被取代的报告与扫描产物，**归档后不再编辑**（规则见 [`../AGENTS.md`](../AGENTS.md) R5）。
> 规则与判定见 [`../AGENTS.md`](../AGENTS.md)；活文档入口见 [`../README.md`](../README.md)。
> 对应技能记忆见 `.agents/skills/`（如 `scoop-gap-audit.md`）。

## 归档清单

| 文件 | 内容 | 归档原因 |
|---|---|---|
| `audit-fix-history-2026-08.md` | 审计修复史：2026-08-12/13 三轮（历史重构残留 / 隐藏问题 / 修复回归边界）+ uninstall 对齐官方收官 | **整段历史已闭环**；通用教训已提炼进技能 [[hok-audit-fix-history]]，五段 2026-08-15 记录并入 devlog `2026-08-15-nsis-extraction-and-7z-perf.md`（2026-09-26 归档） |
| `refactor/`（12 篇：`plan.md` + `tokio-purity-plan.md` + `prompts/`） | Edition 2024 重构总计划与各阶段/纯净化 P1–P5 开工提示词 | **重构已完成并合入 `main`**（阶段 0–5、P1–P5 全 ✅）；整目录转为历史 |
| `command-deviation-fix-plan-2026-09.md` | 全命令偏离修复计划 S1–S14（决策门 / 验收标准 / 提交拆分） | 计划已执行到收口；**未落地项（S11 余项 `ci-auto-pr`→H4、`checkurls`、`virustotal`、R6 头替换语义）已于 2026-09-26 转入** [`../active/plan/pending-tasks.md`](../active/plan/pending-tasks.md) |
| `FINAL-REPORT-2026-08.md` | 2026-08 综合报告（框架修复 + 差距分析 + backlog） | 会话性总结，结论已吸收进活文档与技能记忆 |
| `framework-review-2026-08.md` | hok 框架评估报告（2026-08 重构后，3 子代理实读审计） | 已完成，执行状态已闭环（见 fix-plan） |
| `framework-fix-plan.md` | 框架修复执行计划 | 已执行完毕 |
| `FINAL_AUDIT.md` | 架构审计报告（超大文件/依赖/违规扫描） | 被 `framework-review-2026-08.md` 取代 |
| `analysis-report.md` | 项目重构分析报告（大文件拆分方案） | 被取代（重构已落地，方案已执行） |
| `manifest-parse-audit.md` | Manifest 解析对比审计（官方 schema.json vs ManifestSpec） | 结论已合并入 `scoop-gap-audit` 技能记忆 |
| `status-gap-analysis.md` | `hok status` 对标官方 `scoop status`（app_status / ScoopTypes 格式） | 结论已并入 `scoop-gap-audit`（2026-09-26 补登记） |
| `h3-parse-visibility-design-2026-09.md` | H3 设计报告：清单解析失败的可见性策略 | 设计调研已闭环、报告冻结；**代码未落地**——`pending-tasks.md` §1 H3 仍挂着待拍板的 D1–D5（2026-10-05 复核，2026-09-26 补登记） |
| `manifest-runtime-tolerance-scoping-2026-09.md` | hok 解析"过严"的性质 / 范围 / 选型专项 | 已闭环（2026-09-26 补登记） |
| `rars-winsqlite3-handover-2026-09.md` | rars / winsqlite3 依赖替换换会话交接（标记**未完成**） | 交接已消费，任务转入后续会话（2026-09-26 补登记） |
| `s2-alias-investigation-2026-09.md` | S2 `alias` 可用性调研 + 决策与落地记录（方案 A：官方式 `shims\scoop-<name>.ps1` 脚本别名） | 已闭环（`3c90095`），结论已同步 `scoop-gap-audit` |
| `s3-export-import-investigation-2026-09.md` | S3 `export`/`import` 与官方 Scoopfile 互通：调研 + 逐项实测差异 + 落地结果 | 已闭环（`a61a471`/`110e160`/`2dd84b2`/`14b9c29`） |
| `s4-config-investigation-2026-09.md` | S4 `config` 键策略与子命令语义：调研 + 用户决策 + 落地记录 | 已闭环（`38ecfa9`…`cf1baec`） |
| `s5-checkup-investigation-2026-09.md` | S5 `checkup` 语义决策：对齐官方环境检查，原已装包体检迁为 `health` | 已闭环（`5ae994d`） |
| `s6-cleanup-cache-investigation-2026-09.md` | S6 `cleanup`/`cache` 数据安全：`-k` 保留当前版本、`cache rm` 按 app 名锚定、`-g` 管理员门与 `-a -g` 两趟 | 已闭环（`1e6b929`/`33ba858`/`1c1311b`/`cc2c072`） |
| `download-stack-overflow-2026-09-25.md` | `hok download <合法本地 manifest>` 主线程栈溢出（0xC00000FD）：判别式 + 调用链定位 + 栈预算分解 + 方案 A–E 对照 | 已闭环（`6620d69`：链接期预留 8 MiB + CLI 级回归测试） |
| `archive-refactor-retrospective.md` | 复盘：`archive.rs` 反复修改的教训（同一函数 4 个提交改 550+ 行、"加-删-加"重复） | 纯历史复盘，教训已吸收（2026-09-26 归档） |
| `search-output-design.md` | `search` 命令表格输出重设计（常驻/条件列、汇总行、无参数语义） | 设计已落地（`a1f580c` `44a56e0`），内容已落入 devlog `2026-10-05-search-output-redesign.md`（2026-10-05 归档；**2026-10-06 补登记**——归档时漏登索引） |
| `duplicates_report.txt` | 工作区重复代码扫描产物（2026-07-27） | **机器生成产物，不入库**（本机保留）；结论已用，且行号锚定重构前的树 |
| `large_duplicates.txt` | 大重复块扫描产物（2026-07-27） | **机器生成产物，不入库**（本机保留）；结论已用 |
| `full_workspace_report.txt` | 工作区全量分析报告（2026-07-31） | **机器生成产物，不入库**（本机保留）；结论已用，且引用已失效的 `crates/hok-shim-ref` |
| `full_workspace_drywall.txt` | 工作区 drywall 扫描产物（2026-07-31） | **机器生成产物，不入库**（本机保留）；结论已用 |
| `prompts/`（7 篇） | 交给子代理 / 会话的 prompt 原文（base64 单编解码、clap 参数解析损坏、当前版本解析统一、i18n 自研替换、阶段 3b 源头发射门控、单步 spinner、宽松 JSON 单入口） | **非文档**：会话输入，2026-09-26 从 `docs/prompts/` 与 `docs/refactor/prompts/` 分离后归此 |

## 2026-09-26 移出本目录（判定当时就错）

以下 5 篇在旧索引的"归档原因"列里写着「**当前有效**」，说明它们本不该归档，已提回 `active/`：

| 文件 | 现位置 |
|---|---|
| `optimization-audit-2026-09.md` | [`../active/audit/`](../active/audit/) |
| `command-deviation-audit-2026-09.md` | [`../active/audit/`](../active/audit/) |
| `s7-s14-investigation-2026-09.md` | [`../active/audit/`](../active/audit/) |
| `binary-size-analysis-2026-09.md` | [`../active/audit/`](../active/audit/) |
| `7zip-switch-investigation-2026-09.md` | [`../active/audit/`](../active/audit/) |

## 说明

- 以上文件均**已入库**（`.agents/**` 自 2026-09-26 起纳入版本控制）；归档 = 文件移动 + 索引更新 + 一次提交。
- **唯一例外**：4 个机器生成的扫描产物（`duplicates_report.txt`、`large_duplicates.txt`、`full_workspace_report.txt`、
  `full_workspace_drywall.txt`）按根 `.gitignore` 的「Generated analysis reports」约定**留在本机不入库**——
  索引登记它们是为了说明"这里曾有过什么"，不代表它们进版本库。
- 活文档入口：[`../README.md`](../README.md)。前沿计划：[`../reports/command-deviation-fix-plan-2026-09.md`](../reports/command-deviation-fix-plan-2026-09.md) 与 [`../active/plan/pending-tasks.md`](../active/plan/pending-tasks.md)。
- 归档时**正文一字不动**，只在 `Status:` 行下插入 `Archived: YYYY-MM-DD`（历史文件当时无 `Status:` 头，则不加）。

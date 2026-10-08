# 活文档索引（`.agents/docs/`）

> 本文件是 `.agents/docs/` 的**唯一活文档入口**。规则见 [`AGENTS.md`](AGENTS.md)。
> 已冻结归档见 [`reports/README.md`](reports/README.md)；日志索引见 [`devlog/README.md`](devlog/README.md)。
> 新增/移动/归档文档时**必须在同一次改动里更新本文件**（规则 R6）。

## active/spec — 会被修订的规格与设计

| 文档 | 内容 |
|---|---|
| [`cleanup-parallel-spec.md`](active/spec/cleanup-parallel-spec.md) | `cleanup` 并行化 + 逐版本进度的行为规格 |
| [`search-output-design.md`](reports/search-output-design.md) | `search` 命令表格输出重设计（已归档；内容已落入 `devlog/` 条目） |
| [`output-system-design.md`](active/spec/output-system-design.md) | 输出系统分层设计（Output sink / 事件 / 样式 / 阶段进度）；§十一 为桶扫描类命令的结果行规范 |
| [`task-help-i18n.md`](active/spec/task-help-i18n.md) | 命令帮助文本的 i18n 方案 |

## active/plan — 会被执行掉的计划

| 文档 | 内容 |
|---|---|
| [`pending-tasks.md`](active/plan/pending-tasks.md) | 跨主题**未完成项**总表：按状态分节（待拍板 / 未开工 / 进行中 / 暂缓 / 已定不做），每条一行 + 来源链接 |
| [`private-bucket-plan-2026-09.md`](active/plan/private-bucket-plan-2026-09.md) | 私有 bucket 方案与落地步骤 |
| [`manifest-history-index-plan.md`](active/plan/manifest-history-index-plan.md) | 清单历史索引正式方案（表结构 / 种子与增量 / 读取与失效回落 / 提交拆分 / 验收标准；含成本实测归因与并行化理由） |
| [`manifest-seed-tree-cost-plan.md`](active/plan/manifest-seed-tree-cost-plan.md) | **待新对话执行**：种子树 diff 成本拆解与 2.3× 优化方案（记忆化父树 ＋ 自研 merge-join；含已排除项与测量配方） |
| [`cli-additions-plan.md`](active/plan/cli-additions-plan.md) | **执行中**：从 hyperscoop 借鉴的 6+1 项的命令契约、边界与验收；health 补检测已落地（`dd52a4f`），**剩余待做 = `--dry-run` / `update -c` / `reset` 对齐官方**；`edit`、原生 `service`、`update -q` 已暂缓（理由见文档） |

## active/audit — 随修复持续更新的审计基线

| 文档 | 内容 |
|---|---|
| [`scoop-gap-analysis-2026-08.md`](active/audit/scoop-gap-analysis-2026-08.md) | 与官方 Scoop 的差距总表 + manifest 解析单入口细节 |
| [`unsafe-audit.md`](active/audit/unsafe-audit.md) | `unsafe` 全量审计与豁免分类（`unsafe_code = deny` 的依据） |
| [`scoop-gap-status.md`](active/audit/scoop-gap-status.md) | **对标差距的逐项现状**（P0/P1/P2/P3 的 ✅/⏸ 与依据）；方法论见技能 `scoop-gap-audit` |
| [`command-deviation-audit-2026-09.md`](active/audit/command-deviation-audit-2026-09.md) | 36 个子命令 vs 官方 0.5.3 的偏离审计（约 280 条） |
| [`optimization-audit-2026-09.md`](active/audit/optimization-audit-2026-09.md) | 精简 / 合并 / 优化空间审计（clippy + cargo-modules + machete + drywall + jscpd） |
| [`systemic-optimization-audit-2026-09.md`](active/audit/systemic-optimization-audit-2026-09.md) | 系统性优化点盘点（结构 / 代码冗余 / 逻辑冗余 / crates 替代剪裁 / feature 裁剪） |
| [`hok-overall-framework-analysis.md`](active/audit/hok-overall-framework-analysis.md) | ⚠️ 已被上一行取代（广度与深度不足；失真项见新报告 §0） |
| [`s7-s14-investigation-2026-09.md`](active/audit/s7-s14-investigation-2026-09.md) | S7+S14 低风险批量逐项对照与落地进度 |
| [`binary-size-analysis-2026-09.md`](active/audit/binary-size-analysis-2026-09.md) | `hok.exe` 体积归因与压缩分发方案对照 |
| [`7zip-switch-investigation-2026-09.md`](active/audit/7zip-switch-investigation-2026-09.md) | 「适度切换外部 7z」收益调研与方案 A–D 对照 |
| [`rule-gate-anchors-2026-09.md`](active/audit/rule-gate-anchors-2026-09.md) | 规则载体的闸门锚点审计（20 个载体的闸门/知识分层、复述漂移与缺锚点清单） |
| [`upstream-0.6.0-impact-2026-10.md`](active/audit/upstream-0.6.0-impact-2026-10.md) | 官方 Scoop 0.5.3 → 0.6.0（76 提交）对 hok 的影响逐项评估与本会话处置 |

## research — 结论已定的调研（只读）

| 文档 | 内容 |
|---|---|
| [`nsis-research.md`](research/nsis-research.md) | NSIS 纯 Rust 提取的可行性调研 |
| [`zip-sfx-research.md`](research/zip-sfx-research.md) | ZIP SFX 原生解压调研 |
| [`cleanup-delete-perf.md`](research/cleanup-delete-perf.md) | `cleanup --delete` 性能分析与并行化依据 |
| [`manifest-history-index.md`](research/manifest-history-index.md) | 「git 库 → 索引库」清单历史索引调研草稿（存什么/放哪里/怎么建/能否并行，成本实测） |
| [`manifest-scan-output-2026-10.md`](research/manifest-scan-output-2026-10.md) | 桶扫描类命令（checkver + 4 兄弟）输出盘点与差距基线；方向见 `output-system-design.md` §十一 |
| [`hyperscoop-feature-survey-2026-10.md`](research/hyperscoop-feature-survey-2026-10.md) | 姊妹项目 `hp` 的功能盘点：值得参考项（dry-run / status --json / size / checkup 补检 / 跨进程锁）、需单独立案项与反例（service / merge / README 夸大）；命令面差异的四种性质与「只借纯新增命令」口径；**依赖面实测（`depends` 仅 3%、深度 1）⇒ brew 型 `uses`/`autoremove`/`leaves` 降级或排除**；**本会话不立项** |

## 历史与冻结

| 位置 | 内容 |
|---|---|
| [`reports/`](reports/) | 已失效 / 已闭环报告与扫描产物（冻结，含 `prompts/` 会话输入） |
| [`reports/refactor/`](reports/refactor/) | Edition 2024 重构总计划 + `tokio-purity-plan.md` + 各阶段开工提示词（重构已完成，2026-09-26 归档） |
| [`reports/command-deviation-fix-plan-2026-09.md`](reports/command-deviation-fix-plan-2026-09.md) | 全命令偏离修复计划 S1–S14（2026-09-26 归档；**未落地项 S11 余项 / H4 / R6 已转入** [`active/plan/pending-tasks.md`](active/plan/pending-tasks.md)） |
| [`legacy/`](legacy/) | 失效提案与上游 issue/PR 草稿（`nsis-issue.md`） |

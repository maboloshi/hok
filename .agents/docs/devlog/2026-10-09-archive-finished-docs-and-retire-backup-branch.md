# 归档已完成文档并注销 Wave 1 备份分支

> 2026-10-09 ｜ 提交 `a95415e` ｜ worktree 主树 ｜ 分支 `refactor/edition-2024`

`pending-tasks.md` 的 T1 / T2 / T3 三条出清。

## T1 `task-help-i18n.md` → `reports/`

使命已完成：结构性表头（Usage / Arguments / Options / Subcommands）现在都走
`cli_help.heading.*`，en 与 zh 各一份，`tests/cli_help_i18n.rs::cli_help_is_fully_localized`
把它钉住（含「zh 短帮助里不得留英文描述」与「zh 长帮助不得泄漏英文 about」两条断言）。
该文档当年是交接提示词，从未有 `Status:` 行 —— 归档时按 R3 补 `Status: closed` 再插 `Archived:`。

## T2 两个专题计划 → `reports/`

- `manifest-history-index-plan.md`：mhi 全系列在主线上（`62261c5` `615a6bb` `61c3b7d`
  `27ce29e` `ab7c4c2` `9893616`），但 §5 的提交表仍写着 3/4 为 `⏳`、0–2 为「✅ 待落地」，
  头部也还写着「实现中」。**这是「落地会话漏回填计划表」的又一例**（与 `reports/README.md`
  登记过的「先落文件、忘了登索引」同型）。
- `manifest-seed-tree-cost-plan.md`：`6a93ecc` 已落地（§8/§9 有结果），但开头仍是
  「给新对话（唯一入口）……先按 §4 实现、§5 验收」，`docs/README.md` 也还标「待新对话执行」。

两篇归档时**只改状态**（表内状态列换成真实 sha、头部状态句、README 描述），方案与实测一字未动；
其余按 R5「正文一字不动」，只插 `Archived: 2026-10-09`。

## T3 注销 `r1-checkver-hang-backup`

判据是 `git cherry refactor/edition-2024 r1-checkver-hang-backup` = 5 条全 `-`（0 条未合并，
逐 patch-id 等价）。**只删这一条**：`backup/mhi-series-v1|v2`、`backup/pre-p1-rebase`、
`backup/pre-rusqlite-drop` 属其他在飞 worktree 的回滚点，`refs/worktree-backup/*` 按约定保留。

## 顺带

移动产生 13 条断链（audit / research / devlog / plan 各若干），已按 R5-4 在同一次改动里全部修掉；
`doc-links.mjs` 299 → 294 条链接、0 断链，另两个文档门禁同样 exit 0。

## 教训

计划文档的**状态位**必须由落地会话在落地当下回填；一旦留到下一次会话，归档判定就只能靠
「代码在不在主线」重新推一遍——本次 T2 的前提就是在这种状态下写下并一度看起来是错的。

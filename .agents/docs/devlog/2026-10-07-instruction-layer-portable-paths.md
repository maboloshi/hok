# 指令层本机路径清零（全局规则同步收口）

> 2026-10-07 ｜ 提交 `7ce1f7e` ｜ worktree 主树（`D:\App\github项目\Scoop\hok`）｜ 分支 `refactor/edition-2024`

## 背景

上一轮做技能清理时，全局技能包里的自检脚本报出 10 处「指令层出现本机绝对路径」。按硬约束 5，指令层 =
工作区 `AGENTS.md` + `skills/**` + `docs/AGENTS.md` + `docs/README.md` + `docs/active/**`（除 `prompts/`），
这些位置不得出现本机路径。用户要求一并修掉。

## 改了什么

**本仓 `7ce1f7e`（5 文件）**：10 处改成可移植写法——4 个本机 bench 目录下的一次性探针脚本 → "(一次性…未入库)"；
一处构建报错原文里的仓库路径 → `<repo>`；两处本机 `D:\App\Scoop` → `$SCOOP`；
`C:\ProgramData\scoop` → `%ProgramData%\scoop`、`C:\Users\<user>\scoop` → `$HOME\scoop`、
gpg 报错原文里的家目录 → `<home>`。

**全局（机器本地，不入本仓）**：`agent-workspace-conventions` §4.7 补一句"指令层不写本机绝对路径，
环境变量与占位式可移植、允许"——此前这条规则只存在于 hok 的硬约束里，而检查器在全局技能包里，
属"检查器在查一条技能没写的规则"。自检脚本同步识别占位式（`C:\Users\<user>\…` 不再误报）。

## 教训

1. **规则和检查器要同源**：检查器打包在全局技能里，规则本体却只在某一个仓库的 `AGENTS.md` 里，
   换一个仓库跑就会报一堆"本地约定"。
2. **"违规"要分三类再动手**：真本机路径（要改）、Windows/官方固定默认值（改成 `%ProgramData%` 即可，
   信息不丢）、工具输出原文（用 `<repo>`/`<home>` 占位，仍可读）。一刀切删掉证据是另一种损失。
3. **批量改路径前先跑一遍全量体检**：`active/**`+`research/**` 缺 `Status:`/`Updated:` 头块、
   `-audit`/`-research` 冗余后缀各有十余处，但 R3/R2 都写着"只管新建"——**不动**，
   否则会把"按规则豁免的存量"当成违规批量返工。

门禁：`devlog-index.mjs --check` / `doc-links.mjs` / `docs-index-check.mjs` / 工作区自检 全部 exit 0
（自检从 10 问题降到 0）。

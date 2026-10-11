# 机器本地技能根迁到 ~/.agents/skills，并修掉仓内所有旧指针

> 2026-10-04 ｜ 提交 `9411488` ｜ worktree 主树（`D:\App\github项目\Scoop\hok`）｜ 分支 `refactor/edition-2024`

## 背景

为 hok 建「日志约定 + 技能放置」的全局技能时，发现 DSH 的**机器本地**技能根有两个：
`~/.dsh/skills`（rank 400，**只有 DSH 读**）与 `~/.agents/skills`（rank 500，`$DSH_AGENTS_HOME` 默认值，
Cline 等 `.agents` 系工具也读）。用户决定整体迁到后者，并把仓内指向旧根的活文档指针一并修掉。

## 改了什么

全局侧（机器本地，不入本仓）：`~/.dsh/skills` 的 10 个技能 + `worktree-parallel/` 目录包 + 本会话新建的
`agent-workspace-conventions.md`，全部 **Move** 到 `~/.agents/skills`（逐个 SHA256 比对，字节不变；
**不复制**——同名并存时 rank 400 会静默压掉 500）；`worktree-parallel/SKILL.md` 两处自带脚本的调用路径同批更新。

本仓（`9411488`，6 文件）：

- 根 `AGENTS.md`、`.agents/skills/AGENTS.md`：机器本地根的路径；
- `.agents/skills/hok-worktree-parallel.md`、`active/plan/pending-tasks.md`、`active/audit/s7-s14-investigation-2026-09.md`：
  `worktree-merge.ps1` / `test-worktree-merge.ps1` 的调用路径——**照旧文抄下来的命令跑不通**；
- `active/audit/rule-gate-anchors-2026-09.md`：「未核实」清单里的 `~/.dsh/skills/.archive/` 已作废
  （归档现在同级 `~/.dsh/skills-archive/`），`Updated:` 同批刷新。

`docs/reports/**` 的 3 处旧路径**保留原文**（R5 冻结区）；留在原处的 `.dsh/skills` 都是**项目级**根
（rank 100 vs 200），与本次迁移无关。

## 教训

1. **机器本地技能根不止一个，且跨工具不等价**：两个根都在 DSH 扫描表里、rank 小者胜，但只有
   `~/.agents/skills` 是 `.agents` 系的共享路径。放技能前先问"谁会读它"。
2. **换根必须移动、不能复制**：与项目级 `.dsh`/`.agents` 同一个坑，被压那份静默失效，
   日志只留一行 `ignored because a higher-priority skill already exists`。
3. **路径迁移要扫"可执行指针"**：真正会坏的是文档里要照着抄的命令行，不是叙述性提及；
   `grep -r` 之后按"抄下来能不能跑"再筛一遍。
4. **UTF-16 的技能等于不存在**：同批修好的 `~/.agents/skills/ask/SKILL.md` 原是 UTF-16LE + BOM，
   frontmatter 解析失败被静默跳过（见 `file-editing`）。

门禁：`devlog-index.mjs --check` / `doc-links.mjs` / `docs-index-check.mjs` 全部 exit 0。

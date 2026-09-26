---
name: hok-report-archive
title: 报告归档规范（规则本体在 .agents/docs/AGENTS.md）
description: hok 文档归档的开工/收尾动作：规则本体见 .agents/docs/AGENTS.md（生命周期×类型双轴、命名、头块、R4 判定、R5 冻结动作、R6 索引、R7 日志分片）；本技能只记扫哪些目录、收尾改哪些文件。
metadata:
  type: project
---

**规则本体 = `.agents/docs/AGENTS.md`（R1–R9，由 DSH 在触碰该目录时自动加载）。**
本文件只保留「开工扫哪里、收尾改哪些文件」，判定细节一律以规则本体为准，不在此重复。

## 目录速查（2026-09-26 双轴重构后，原 `.dsh/docs/`）

| 类别 | 位置 |
|---|---|
| 活：规格（会被修订） | `.agents/docs/active/spec/` |
| 活：计划（会被执行掉） | `.agents/docs/active/plan/`（`pending-tasks.md` / `private-bucket-plan-2026-09.md`） |
| 死：整目录归档的计划+提示词 | `.agents/docs/reports/refactor/` |
| 活：审计基线（随修复更新） | `.agents/docs/active/audit/` |
| 活：调研（结论已定，只读） | `.agents/docs/research/` |
| 活：日志 | `.agents/docs/devlog/YYYY-MM-DD-<english-slug>.md` + 生成的 `README.md` |
| 活：唯一入口索引 | `.agents/docs/README.md` |
| 死：失效报告（冻结） | `.agents/docs/reports/`（含 `prompts/` 会话输入） |
| 死：失效提案/补丁 | `.agents/docs/legacy/` |
| 过期技能 | `.agents/archive/skills/`（**必须在技能发现根之外**） |
| 项目文档 | 仓库根 `AGENTS.md` / `ARCHITECTURE.md` / `README*.md` / `CHANGELOG.md` —— 永不动 |
| 面向人的参考资料 | 仓库根 `docs/`（**入库**，英文优先，索引 `docs/README.md`） |

## 开工

```powershell
Get-ChildItem .agents/docs -Recurse -File -Include *.md,*.txt
```

## 收尾（会话收官 / 新报告生成时）

1. 按规则本体 **R4** 判定是否归档；归档动作照 **R5**（`Move-Item` 进 `reports/`，正文一字不动，
   只在 `Status:` 下加 `Archived: YYYY-MM-DD`），并修掉 inbound link。
2. 更新 `.agents/docs/reports/README.md` 索引（文件 / 内容 / 归档原因）。
3. 活文档有变动 → 更新 `.agents/docs/README.md`（**R6：同一改动内**）。
4. 新增日志条目 → 新建 `.agents/docs/devlog/YYYY-MM-DD-<english-slug>.md`，再跑 `node .agents/scripts/devlog-index.mjs`（**R7**）。
5. 过期技能 → `.agents/archive/skills/`。
6. **随会话提交**：`.agents/**` 已入库（2026-09-26 起）；只忽略 `.agents/tmp/` 与本机 `*.local.md`。

## 两条容易踩的

- `active/` ↔ `reports/` 是**双向**的：旧索引里写着「当前有效」的条目说明当时判错了，应提回 `active/`。
- **已入库文件不得引用 `.agents/**` 当权威**（**R9**）：`README.md` / `README-zh.md` / `CHANGELOG.md` /
  仓库根 `docs/**` 里写「详见 `.agents/docs/xxx.md`」对克隆者是死链；要么移入 `docs/`，要么改写为不带路径的叙述。

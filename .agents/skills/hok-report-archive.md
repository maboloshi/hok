---
name: hok-report-archive
title: 报告归档规范（规则本体在 .agents/docs/AGENTS.md）
description: 会话收尾归档 .agents/docs/ 的报告、判断文档该进 active/ 还是 reports/、或把过期技能移进 archive/ 时读：开工扫哪些目录、收尾要改哪些索引文件（规则本体是 .agents/docs/AGENTS.md 的 R1–R9）。
metadata:
  type: project
---

**规则本体 = `.agents/docs/AGENTS.md`（R1–R9，由 DSH 在触碰该目录时自动加载）。**
本文件只保留「开工扫哪里、收尾改哪些文件」，判定细节一律以规则本体为准，不在此重复——
**双轴归属表（生命周期 × 文档类型）只在规则本体 R1，本文件不复刻**；要找某类文档放哪，读 R1。

**射程**：本技能管 `.agents/docs/**`，外加过期技能 → `.agents/archive/skills/`。仓库根的 `AGENTS.md`、
`ARCHITECTURE.md`、`README*.md`、`CHANGELOG.md` **不参与归档与改名**；面向人的参考资料在仓库根 `docs/`
（索引 `docs/README.md`，英文优先），那是 R6/R9 的事，不由本技能"归档"。

## 开工

```powershell
Get-ChildItem .agents/docs -Recurse -File -Include *.md,*.txt
```

## 收尾（会话收官 / 新报告生成时）

1. 按规则本体 **R4** 判定是否归档；归档动作照 **R5**（`Move-Item` 进 `reports/`，正文一字不动，
   只在 `Status:` 下加 `Archived: YYYY-MM-DD`），并修掉 inbound link。
2. 更新 `.agents/docs/reports/README.md` 索引（文件 / 内容 / 归档原因）。
3. 活文档有变动 → 更新 `.agents/docs/README.md`（**R6：同一改动内**）。
4. 会话有落地 → 新建 `.agents/docs/devlog/YYYY-MM-DD-<english-slug>.md`，再跑 `node .agents/scripts/devlog-index.mjs`（触发、格式与落哪条提交见 [`docs/AGENTS.md` R7](../docs/AGENTS.md)）。
5. 过期技能 → `.agents/archive/skills/`。
6. **随会话提交**：`.agents/**` 已入库；只忽略 `.agents/tmp/` 与本机 `*.local.md`。

## 两条容易踩的

- `active/` ↔ `reports/` 是**双向**的：旧索引里写着「当前有效」的条目说明当时判错了，应提回 `active/`。
- **引用 `.agents/docs/**` 是合法的**（**R9**）：必须用**仓库相对路径**；但面向**仓库外**读者、
  需长期维护的文档仍放仓库根 `docs/`，不要把 `.agents/` 当成对外权威来引用。

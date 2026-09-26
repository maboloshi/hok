# AGENTS.md — `.agents/skills/` 技能规范

本目录是 hok 的**项目技能根**，已入库——技能是仓库的共享资产，改了就进评审。

## 形态

DSH 支持两种技能形态（`@deepseek-ai/dsh-skill-filesystem/lib/index.js:552-556`）：

1. **扁平**：`<name>.md` —— 本仓库现状，16 个技能全部如此。
2. **目录包**：`<name>/SKILL.md`（可附带 `scripts/`、资源文件）。

**何时升为目录包**：只有当技能需要携带脚本或附属资源时。纯记忆/纪律类技能保持扁平。

## 判据：什么留在技能里（2026-09-26 定）

技能只装**下次还要用到的**。一句话判据：

> **把日期和提交号删掉，这段话还有指导价值吗？**

- **有** → 留在技能，并且**把日期/提交号删掉**（它们只会漂）。
- **没有** → 它是一条**记录**，进 `.agents/docs/`：落地记录 → `devlog/`（一条一文件，见 R7）；
  现状/进度 → `active/audit/` 或 `active/plan/`；已闭环的整段历史 → `reports/`。
- **一段里既有史又有教训** → 只把教训留下（并按上一条去掉日期），史链到报告。
- **收尾时不要往技能里追加落地记录**——技能不是日志。

**反面案例（2026-09-26 清理，留作标尺）**：`scoop-gap-audit` 曾 **96.7 KB / 428 行**，29 章里 **28 章**
是按日期追加的落地记录，是第二名技能的 **8.6 倍**、占技能目录约 60%——每次加载要注入 2–3 万 token，
而它还是收尾四件套的固定目标。拆完后只剩可复用方法，落地记录进 `docs/devlog/` 与
`docs/active/audit/scoop-gap-status.md`——**行数与体积不追记**（写进规则文件就过期）。
`hok-audit-fix-history` 同型。

## frontmatter 契约

```yaml
---
name: hok-<slug>          # 必须与文件名一致（不含 .md）
title: <中文短标题>
description: <一句话，写给"要不要加载它"的决策>   # 会进技能目录，是唯一的发现入口
metadata:
  type: project | feedback | reference
---
```

- `description` 必须自包含：**谁**在**什么场景**该加载它。禁止"详见某文件"式描述。
- 技能正文用中文（与 `.agents/docs/` 一致）；命令、路径、代码块内的标识符保持原样。

## 命名

- 仓库专属：`hok-<topic>.md`
- 跨仓库/外部事实：`<topic>.md`（如 `local-scoop-source-path.md`）
- 不用日期、不用版本号。

## 归档

过期技能移入 **`.agents/archive/skills/`**（技能发现根之外，见 [`.agents/AGENTS.md`](../AGENTS.md)）。
**不要**放在 `skills/.archive/`：DSH 仍会把它当技能包目录去探测 `SKILL.md`，白白污染层级与 watcher。

## 与文档的分工

| 内容 | 归属 |
|---|---|
| 可复用的操作知识、纪律、坑、决策门 | 技能（本目录） |
| 一次性调研、计划、审计、日志 | `.agents/docs/`（见 [`docs/AGENTS.md`](../docs/AGENTS.md)） |

`hok-report-archive` 的判定同样适用于技能：**结论沉淀到技能后，原报告即可归档**。

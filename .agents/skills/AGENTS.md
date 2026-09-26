# AGENTS.md — `.agents/skills/` 技能规范

本目录是 hok 的**项目技能根**，已入库——技能是仓库的共享资产，改了就进评审。

## 形态

DSH 支持两种技能形态（`@deepseek-ai/dsh-skill-filesystem/lib/index.js:552-556`）：

1. **扁平**：`<name>.md` —— 本仓库现状，16 个技能全部如此。
2. **目录包**：`<name>/SKILL.md`（可附带 `scripts/`、资源文件）。

**何时升为目录包**：只有当技能需要携带脚本或附属资源时。纯记忆/纪律类技能保持扁平。

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

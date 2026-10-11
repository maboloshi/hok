---
name: scoop-gap-audit
title: Scoop 对标差距审计（方法论 + manifest 解析单入口）
description: 与官方 Scoop 对标、或要查 hok 差距现状（P0–P3）与偏离审计入口时读：审计怎么跑、方法要点，以及现状表 / 报告 / 修复计划的落点。本文只装"怎么做"，不装"做过什么"。
metadata:
  type: project
---

**用途**：做「hok vs 官方 Scoop」对标前先读本文拿**方法与入口**。

| 要找什么 | 去哪 |
|---|---|
| **逐项现状**（P0/P1/P2/P3 的 ✅/⏸ 与依据） | [`.agents/docs/active/audit/scoop-gap-status.md`](../docs/active/audit/scoop-gap-status.md) |
| **逐次落地记录**（哪次会话改了什么） | [`docs/devlog/`](../docs/devlog/)（一条一文件，按日期） |
| 差距总表（2026-08 审计原文） | [`scoop-gap-analysis-2026-08.md`](../docs/active/audit/scoop-gap-analysis-2026-08.md) |
| 全命令偏离审计（约 280 条） | [`command-deviation-audit-2026-09.md`](../docs/active/audit/command-deviation-audit-2026-09.md) |
| 修复计划（已归档） | [`reports/command-deviation-fix-plan-2026-09.md`](../docs/reports/command-deviation-fix-plan-2026-09.md) |
| 门禁命令 | [[hok-rust-gate-practice]] |

## 怎么跑

- **规模审计**：拆 4 组并行子代理**静态读码**逐命令对照官方（历次审计用的形态），
  输出必须带 **file:line** 与**分类**（语义不同 / 行为缺失 / 默认值不同 / 输出或文案不同 / 退出码不同 /
  边界处理不同 / hok 独有 / 有意偏离）。**静态读码 ≠ 实测**——动手前按 [[investigate-first]] 复现。
- **行为锚**：本地官方源码（路径见 [[local-scoop-source-path]]），**勿联网**。
- **编号**：P0 = 不兼容/静默错误；P1 = CLI 兼容缺口；P2 = 功能缺口；P3 = 低/风格。
  状态 **✅=已修 / ⏸=暂缓或有意不做**。
- **收尾**：每修完一项 → 改状态表那一行 + 写一条 devlog（触发与格式见
  [`docs/AGENTS.md` R7](../docs/AGENTS.md)）。**别再往本文件堆落地记录**（判据见
  [`skills/AGENTS.md`](AGENTS.md)）。

## Manifest 解析专题（单入口架构）
**所有 JSON 读取收敛到单一入口**：`internal::json::{parse_str, parse_str_or_none}` —— `strip_bom`（`internal/string.rs`）→ serde_json 严格 → json5 回退，两者都失败时报**严格错误**。消费者：`Manifest::parse/from_json/parse_str`、`config::read_json_file`、`formatjson::{format_manifest_file, schema_warnings}`，外加 psengine/script 的行级 BOM（非 JSON）。`grep "json5::from_str"` 全仓仅剩该模块一处；唯一吞错入口 = `parse_str_or_none`（仅 `schema_warnings`，官方无对应调用点）。I/O 与字节解码**留在调用方**（官方读 API 本就不同：`ReadAllLines` 探测编码 / `Get-Content -Encoding UTF8` / `ReadAllText`）。
**宽容度契约（实测，勿再凭印象）**：`internal/json.rs::tolerance_contract` 固化 11 条三方裁决（PS 5.1 / pwsh 7 / hok 三宿主实测）。**hok 不是任何宿主的超集**：注释/尾逗号/无引号键比 5.1 宽，但 `{"a":01}` 两宿主都接受、hok 拒绝（JSON5 与 JSON 同禁前导零）；`{"a":1 // c}`（无换行）三者全拒。官方 `parse_json` 是 warn→`$null`（调用方报"找不到 manifest"），hok 直接 `Err`——**有意偏离**（登记的"比 Scoop 更好"项），不夹带改 warn。
改宽容度/加调用点只改 `internal::json` = 全局生效；`formatjson` 仍是 Value 层旁路（不解析语义），但解析入口同一条。
## 相关技能

[[hok-audit-fix-history]]（审计修复的**通用教训**）、[[hok-code-organization]]（分层与去重门槛）、
[[hok-proactive-review]]（提交前对照项）。

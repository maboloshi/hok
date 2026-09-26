---
name: scoop-gap-audit
title: Scoop 对标差距审计（方法论 + manifest 解析单入口）
description: 与官方 Scoop 对标时的方法与入口——怎么跑差距审计、P0–P3 编号与 ✅/⏸ 语义、manifest 单入口解析契约、以及现状表/报告在哪。本文只装"怎么做"，不装"做过什么"。
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
- **行为锚**：本地官方源码 `../Scoop/libexec|lib`（见 [[local-scoop-source-path]]），**勿联网**。
- **编号**：P0 = 不兼容/静默错误；P1 = CLI 兼容缺口；P2 = 功能缺口；P3 = 低/风格。
  状态 **✅=已修 / ⏸=暂缓或有意不做**。
- **收尾**：每修完一项 → 改状态表那一行 + 写一条 devlog。**别再往本文件堆落地记录**（判据见
  [`skills/AGENTS.md`](AGENTS.md)）。

## Manifest 解析专题（单入口架构）
**所有 JSON 读取收敛到单一入口**：`internal::json::{parse_str, parse_str_or_none}` —— `strip_bom`（`internal/string.rs`）→ serde_json 严格 → json5 回退，两者都失败时报**严格错误**。消费者：`Manifest::parse/from_json/parse_str`、`config::read_json_file`、`formatjson::{format_manifest_file, schema_warnings}`，外加 psengine/script 的行级 BOM（非 JSON）。`grep "json5::from_str"` 全仓仅剩该模块一处；唯一吞错入口 = `parse_str_or_none`（仅 `schema_warnings`，官方无对应调用点）。I/O 与字节解码**留在调用方**（官方读 API 本就不同：`ReadAllLines` 探测编码 / `Get-Content -Encoding UTF8` / `ReadAllText`）。
**宽容度契约（实测，勿再凭印象）**：`internal/json.rs::tolerance_contract` 固化 11 条三方裁决（PS 5.1 / pwsh 7 / hok 三宿主实测）。**hok 不是任何宿主的超集**：注释/尾逗号/无引号键比 5.1 宽，但 `{"a":01}` 两宿主都接受、hok 拒绝（JSON5 与 JSON 同禁前导零）；`{"a":1 // c}`（无换行）三者全拒。官方 `parse_json` 是 warn→`$null`（调用方报"找不到 manifest"），hok 直接 `Err`——**有意偏离**（登记的"比 Scoop 更好"项），不夹带改 warn。
改宽容度/加调用点只改 `internal::json` = 全局生效；`formatjson` 仍是 Value 层旁路（不解析语义），但解析入口同一条。
- P0-1 hash 前缀×长度强绑定 → 已宽容修复（非标准长度不再拒收）
- P0-2 checkver.sourceforge 对象强制 path 必填 → 已修（path 可选）
- **P0-4（已修）`autoupdate.hash.mode: github`**：官方 `schema.json:51-62` 只列 8 个枚举值，但**官方运行时不校验 schema**、`lib/autoupdate.ps1:271-302` 的 `switch` 还接受 `github`（`:298-301` 取 Releases API asset `digest`，匿名无 token）。hok 用严格 serde 枚举（`manifest.rs:586`）⇒ `unknown variant` ⇒ **`parse_file` 整条 Err**；又因 `package/query.rs:270,275` 的 `filter_map` **静默丢弃**，`install/search/info` 表现为「**找不到软件包**」（这才是它难定位的原因）。修法 = 加 `#[serde(rename = "github")] Github` + 注释 + 离线夹具（`crates/libscoop/tests/fixtures/0x7eTeamTools.json`）；**提取实现早已在 `checkver_hash.rs:172-178` 在位**（枚举全仓无生产读点，只是 schema 镜像）⇒ 不是补空壳。验收：命中文件全部可解析、`search` 无查询看到的包数**恰好 +220**、其余 bucket 零变化。
- **未做待办（P0 级可见性）**：清单解析失败**不得**在查询/安装路径静默消失（`package/query.rs:270,275`；`:285-313` 解析失败落 `None`）——改它等于改单入口的错误传播策略，影响全部清单扫描语义，需单独立项。
- ~~**未做待办**：checkver 的 `mode: github` 缺"digest 为 null / API 限流 → 回落下载 + 本地算 SHA-256"~~ → **已解决**：`checkver_hash.rs:172-187` 取 `.digest` 失败/为 null 时返回 `None`，随即走 `download_and_compute_sha256`（下载落 Scoop 缓存 + 本地 SHA-256），不再 `Err` 中断该清单。**别再按"未做"来立项。**
- ~~**已知 flaky（与改动无关）**：`package::manifest_cache::tests::serving_a_stale_cache_warns_once`~~ → **已修**：根因是**闩锁放错层**——`STALE_WARNED` / `MISMATCH_WARNED` 曾是进程级 static，而告警发往**每会话**的 `session.output()`。两处闩锁已搬进 `Session`（`mark_stale_warned` / `mark_mismatch_warned`，后者 key 仍是 `bucket/name`）。**别再把它记成"重跑即过"**——once 标志 + 并行测试要先按这条排查（细节见 `.agents/docs/devlog/2026-09-26-manifest-cache-warn-latch.md`）。
- P1 autoupdate 缺字段：官方 13 字段 vs hok 5 → 已补（architecture/extract_dir/hash/notes/url + autoupdateArch）
- P2 无影响不动（hashExtraction.type deprecated、installer._comment、shortcuts 长度、version pattern）
- 已验证兼容：license/psmodule/checkver 全键/mode 8 枚举/installer/uninstaller/architecture/autoupdate.hash 三形式/bin 对象/hook script



## 相关技能

[[hok-audit-fix-history]]（审计修复的**通用教训**）、[[hok-code-organization]]（分层与去重门槛）、
[[hok-proactive-review]]（提交前对照项）。

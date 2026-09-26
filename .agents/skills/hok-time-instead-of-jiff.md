---
name: hok-time-instead-of-jiff
title: hok 时间库 jiff → time 0.3 迁移
description: 时间库已由 jiff 换 time 0.3（internal/time.rs）：local_offset_at 与 offset_hour 取值坑。
metadata:
  type: feedback
---

hok 已于 2026-08-09 从 jiff 0.2 迁移到 time 0.3（两个 Cargo.toml 的 jiff 依赖已移除，Cargo.lock 清除 jiff/jiff-static/jiff-tzdb/jiff-tzdb-platform）。

**Why：** jiff 内嵌 IANA tzdb 全量时区数据（编译成本高 + 一个 proc-macro）；time 0.3 已在依赖树（ureq→cookie 引入），零新增编译成本；Windows 下取本地偏移与官方 Scoop（.NET）同源。时区获取靠系统查询（`UtcOffset::local_offset_at`），对"展示/冷却/last_update"场景无行为差异。

**How to apply：**
- 时间逻辑全在 `libscoop/src/internal/time.rs`（P1-22 产物，见 [[hok-code-organization]] 工具层）：`format_last_update`（Scoop ISO 串 7 位小数+偏移）、`parse_last_update`（Rfc3339 → 裸无偏移两级回退）。
- time 0.3.53 关键 API/坑：
  - `time::util::local_offset` 函数已删（0.3.37+ 改为 deprecated module），本地偏移用 `UtcOffset::local_offset_at(t)`（feature `local-offset`）；`OffsetDateTime::now_local()` 同 gate。
  - format_description **无 `offset_sign` 组件**，正偏移带符号用 `[offset_hour sign:mandatory]`（automatic 默认省略正号）；`%z` 等价 `[offset_hour sign:mandatory][offset_minute]`；strftime `%-m` 等价 `[month padding:none]`。
  - `[subsecond digits:N]` 是截断语义（与 jiff `subsec/100` 一致，有测试锁定）。
  - **可选组坑**：`(.[subsecond digits:1+])` 以字面量 `.` 开头时，组缺失不跳过而报 `InvalidLiteral`；裸无偏移解析用两个 fallback 格式（无小数/有小数）。
- features：libscoop = `formatting+parsing+macros+local-offset`（**没有 `well-known` 这个 feature**——Rfc3339 由 formatting/parsing 提供）；根 crate build-deps = `formatting+macros+local-offset`。
- 调用点：bucket.rs `updated_at`（git 提交秒→本地）、bucket.rs `format_last_update`、config.rs `update_cooldown_remaining`、根 build.rs BUILD_DATE。
- Cargo.lock 里 portable-atomic（indicatif）与 windows-link（crossterm/rust-i18n）保留，是其他 crate 的既有依赖，勿误删。
- 测试：time.rs 6 个单测（format 结构/截断/Zoned 解析/Z 后缀/裸无偏移含小数/roundtrip），libscoop 全量 319 绿。

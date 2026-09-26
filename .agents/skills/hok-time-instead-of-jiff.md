---
name: hok-time-instead-of-jiff
title: hok 时间库 jiff → time 0.3 迁移
description: 改 internal/time.rs 的本地偏移或格式解析，或清理 time/portable-atomic/windows-link 这类依赖时读：time 0.3 的 features 组合、local_offset_at 与 offset_hour 取值坑。
metadata:
  type: feedback
---

hok 已从 jiff 0.2 迁移到 time 0.3（两个 Cargo.toml 的 jiff 依赖已移除，Cargo.lock 里 jiff / jiff-static / jiff-tzdb / jiff-tzdb-platform 全部 0 命中）。

**Why：** jiff 内嵌 IANA tzdb 全量时区数据（编译成本高 + 一个 proc-macro）；time 0.3 已在依赖树（ureq→cookie 引入），零新增编译成本；Windows 下取本地偏移与官方 Scoop（.NET）同源。时区获取靠系统查询（`UtcOffset::local_offset_at`），对"展示/冷却/last_update"场景无行为差异。

**How to apply：**
- 时间逻辑全在 `libscoop/src/internal/time.rs`（P1-22 产物，见 [[hok-code-organization]] 工具层）：`format_last_update`（Scoop ISO 串 7 位小数+偏移）、`parse_last_update`（Rfc3339 → 裸无偏移两级回退）。
- time 0.3 关键 API/坑（Cargo.lock 当前 0.3.55）：
  - `time::util::local_offset` 函数已删（0.3.37+ 改为 deprecated module），本地偏移用 `UtcOffset::local_offset_at(t)`（feature `local-offset`）；`OffsetDateTime::now_local()` 同 gate。
  - format_description **无 `offset_sign` 组件**，正偏移带符号用 `[offset_hour sign:mandatory]`（automatic 默认省略正号）；`%z` 等价 `[offset_hour sign:mandatory][offset_minute]`；strftime `%-m` 等价 `[month padding:none]`。
  - `[subsecond digits:N]` 是截断语义（与 jiff `subsec/100` 一致，有测试锁定）。
  - **可选组坑**：`(.[subsecond digits:1+])` 以字面量 `.` 开头时，组缺失不跳过而报 `InvalidLiteral`；裸无偏移解析用两个 fallback 格式（无小数/有小数）。
- features：libscoop = `formatting+parsing+macros+local-offset`（**没有 `well-known` 这个 feature**——Rfc3339 由 formatting/parsing 提供）；根 crate build-deps = `formatting+macros+local-offset`。
- 调用点：bucket.rs `updated_at`（git 提交秒→本地，走 `format_short_local`，time.rs:91）、bucket.rs `format_last_update`（:842）、package/export.rs（:348/:354）、根 build.rs BUILD_DATE。**冷却（cooldown）机制已整体删除**，别再找 `update_cooldown_remaining`。
- Cargo.lock 里 `portable-atomic`（indicatif）与 `windows-link`（parking_lot_core ← crossterm→parking_lot，以及 windows-sys 0.61.2）保留，是其他 crate 的既有依赖，勿误删。**本仓 i18n 是自研 `hok-i18n`，Cargo 树里没有 rust-i18n**。
- 测试：time.rs 10 个单测（format 结构/截断/Zoned 解析/Z 后缀/裸无偏移含小数/roundtrip 等），全量数以 `cargo test -p libscoop` 实测为准（别照抄历史数字）。

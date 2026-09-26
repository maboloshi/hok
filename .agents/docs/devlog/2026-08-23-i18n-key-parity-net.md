# i18n 一致性安全网

> 2026-08-23 ｜ 方案 A：键对账测试

**背景**：i18n 用 `rust-i18n` 外部化 YAML（`locales/en.yml` 398 叶子键、`locales/zh.yml` 398 叶子键），键是字符串字面量，**无编译期校验**——拼错键/漏键/en-zh 不同步都静默（`t!("typo")` 运行时返回键名本身）。此前唯一防线是 `I18nHelp` derive 宏的 `i18n_cli_long_about` 运行时 `starts_with("cli_help.")` 回退探测，但那只覆盖 help 文本，且是"掩盖缺键"而非"报错"。

## 方案选型（评估过 fluent-rs / gettext）
- **fluent-rs**：能力最强（复数/select/语言回退），但它是**运行时库**，只要进生产运行路径就必须是普通 `[dependencies]`，无法只做 dev 依赖（dev-dep 不进发布二进制）；对 hok 这种 2 语言、短句为主的 CLI 是杀鸡用牛刀。
- **gettext**：工具链最成熟，但依赖 C libintl（Windows 静态二进制不友好）、运行时 `.mo` 查找与"单 exe 静态嵌入"形态冲突，且其"英文原文即 id"哲学与 hok 的抽象键名（`cmd.alias_added`）冲突。
- **结论**：两者都解决不了最痛的"调用点不可读"，且都引入不必要的运行时/工具链负担 → 不引入，走**方案 A：给现有 rust-i18n 加零依赖对账测试**。

## 实现（`tests/i18n_keys.rs`，根 crate 集成测试）
对账三个不变量：
1. **en/zh 叶子键完全一致**（fail）：缺翻译/漏键即失败，杜绝 en 加了 zh 忘加；
2. **源码字面量 `t!("key")` 必须存在于 en 与 zh**（fail）：拼错/漏键即失败，这是主要防线；
3. **死键以警告输出、不阻断**（info）：yml 有键但源码字面量从未引用——不 fail 因为部分键经**变量间接引用**（`cmd.list_summary`、`error.*` 经 `Error` 的 `Display` 使用）或 derive 宏动态生成（`cli_help.*`/部分 `cli_alias.*`），字面量扫描无法静态证明其死亡。

**技术要点**：
- 纯 `std`、不依赖 `regex`，测试自包含、编译快；
- YAML 解析只取**叶子键**（有值或块标量 `|/` 的键），跳过中间层级命名空间（`output`、`status`、`cmd`）——这些无翻译值；块标量内容行不误当键（早期把 `long_about` 块里的 `Subcommands:` 误判为键的坑）；
- Rust 源码扫描先去注释（`//`、`///`、`//!`、`/* */`）再找 `t!("...")}`，基于 `char` 操作避免 UTF-8 字节损坏（中文字符串字节被 `as char` 拆坏的坑）；
- **只剥注释、保留字符串字面量**（早期把字符串也剥掉导致键全丢、误报 211 死键的坑）→ 修正后死键只剩 54 个真实"非字面量引用"。

**当前实测**：en/zh 键完全对账；源码字面量键全部存在于两个 yml；54 个死键均为合法引用（`error.*` 走 Display、`cmd.*` 走变量、`cli_alias.*` 走 derive），无需清理。

## 运行方式
`cargo test -p hok --test i18n_keys`（`cargo test` 全量自动覆盖）。

## 后续可选（非本次范围）
- 若要堵"变量键/derive 键"的死键盲区，可扩展：① 对 `translate_error` 的 Error Display 映射做键对账；② 让 `I18nHelp` derive 宏生成键清单并在编译期校验。当前靠字面量扫描 + 软警告已覆盖主要风险。

---

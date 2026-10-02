---
name: hok-rust-gate-practice
title: hok Rust 改动收尾实操卡（edition=2024 仓库专属）
description: 在 hok 里跑 rustfmt/clippy 收尾时读（通用闭环见全局 rust-deterministic-gate）：edition 2024 独立 rustfmt、含 mod 声明的文件会递归格式化子模块（且 `skip_children=true` 会「看不见」子模块、造成假绿）、既有 fmt diff 时禁整包 cargo fmt、clippy 可整包跑。
metadata:
  type: project
---

本仓库（hok/）专属的 Rust 改动收尾实操卡，只记与全局版本无关规则的**差异**。
版本无关总规则见全局 skill `rust-deterministic-gate`；此卡不重复其通用闭环，仅补充本仓 edition=2024 的落地细节。

## 本仓专属差异（edition=2024）
- 本仓所有 crate（Cargo.toml 实测）均为 **edition = "2024"**。独立 `rustfmt` 默认 edition 2015，
  会对 2024 代码的 `async fn` 报 `async fn is not permitted in Rust 2015`；
  对本仓单文件用 `rustfmt --edition 2024 <file>`，或直接走 `cargo fmt`（读 Cargo.toml，最省心）。
- `cargo clippy --fix ... -- --message-format=short` 的 `--message-format` 会被 build script
  当 rustc 参数报 `Unrecognized option`（exit 101）；把该参数放 cargo 侧（`--message-format=short`
  在 `--` 之前）或用输出过滤，别放 `--` 后。
- 仓库存在既有 fmt diff 时禁整包 `cargo fmt`；先用 `git diff --name-only -- '*.rs'` 圈定本次
  改动文件，只 fmt 本次文件。
- **子模块递归：写入模式防误伤，检查模式防漏检**（两条相反，别混用）。
  `rustfmt` 会递归处理含 `mod` 声明的文件，所以：
  - **写入**（`rustfmt <file>`，无 `--check`）：**不要**传含 `mod` 声明的文件，否则会连带改写子模块——
    本仓 `internal/github.rs`、`internal/network.rs`、`package/checkver_hash.rs` 带既有偏差，曾两次被连带改后回退。
    真要传就加 `--config skip_children=true`（已实测有效，`--skip-children` 不是 CLI 参数）——该 flag 只保证**不碰**子模块。
  - **检查**（`--check`）：**不要加 `skip_children`**，它会让子模块**不被检查** ⇒ "文件级 fmt 干净"是假绿。
    判例：`package/operations/script.rs` 用 `skip_children=true --check` 报干净，整文件 `--check` 才报出其 `mod tests` 未格式化；
    子代理报告写的正是"file-scoped（含 skip_children）干净"，差点以假绿收尾。
    ⇒ 对**本次改动文件**统一用**不带 `skip_children`** 的整文件 `--check`；报出的既有偏差里，只看是否含你改动的 hunk。
    （`rustfmt --check` 不会写盘，所以"误伤别人的文件"这条顾虑在检查模式不存在。）
- 其余节奏（先写完整逻辑 → 收尾整轮一次 → 原始日志不回传 LLM、只回传
  OK/改 N 文件/残余 N 诊断）与全局 skill 一致。

## 门禁命令
- 整包可直接跑：`cargo clippy --workspace --all-targets -- -D warnings`（`--no-deps` 可选，仅提速）。
- ⚠️ 若在**旧记录**里看到 `--exclude innospect`：那是已退役的 vendored fork 时代的写法，现在不该再出现
  （该依赖已改用 registry 版，见 `crates/libscoop/Cargo.toml`）；旧审计/报告里的这类命令行属历史记录，别照抄。

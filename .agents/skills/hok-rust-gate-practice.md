---
name: hok-rust-gate-practice
title: hok Rust 改动收尾实操卡（edition=2024 仓库专属）
description: hok 专属 Rust 收尾差异（补充全局 rust-deterministic-gate）：edition 2024 独立 rustfmt、含 mod 声明的文件勿传、既有 fmt diff 时禁整包 cargo fmt。
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
- **`rustfmt` 会递归子模块**：传入任何含 `mod xxx;` 声明的文件（`internal/mod.rs`、`package/checkver.rs`…）
  都会连带格式化其子模块文件——本仓 `internal/github.rs`、`internal/network.rs`、`package/checkver_hash.rs`
  带既有 fmt 偏差，2026-09-12 两次被连带改写后 `git checkout` 回退。**不要传含 mod 声明的文件**；
  非要传就加 `--config skip_children=true`（已实测有效，`--skip-children` 不是 CLI 参数）。
- 其余节奏（先写完整逻辑 → 收尾整轮一次 → 原始日志不回传 LLM、只回传
  OK/改 N 文件/残余 N 诊断）与全局 skill 一致。

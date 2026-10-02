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
- **`rustfmt` 会递归子模块**：传入任何含 `mod xxx;` 声明的文件（`internal/mod.rs`、`package/checkver.rs`…）
  都会连带格式化其子模块文件——本仓 `internal/github.rs`、`internal/network.rs`、`package/checkver_hash.rs`
  带既有 fmt 偏差，曾两次被连带改写后 `git checkout` 回退。**不要传含 mod 声明的文件**；
  非要传就加 `--config skip_children=true`（已实测有效，`--skip-children` 不是 CLI 参数）。
- ⚠️ **`skip_children=true` 的盲区（该 flag 会「看不见」子模块，不只是「不碰」）**：传含 `mod tests { … }`
  的文件时，子模块**根本不被检查**，于是"文件级 fmt 干净"是假绿。本仓判例：`package/operations/script.rs`
  用 `--config skip_children=true --check` 报干净，而整文件 `rustfmt --edition 2024 --check` 报该测试模块未格式化；
  子代理报告里写的正是"file-scoped（含 skip_children）干净"，差点以假绿收尾。
  ⇒ 收尾对**改动文件**要么用**不带 `skip_children`** 的整文件检查（若该文件有既有偏差，只核对你改动的 hunk 是否被报），
  要么显式再查一遍子模块；`skip_children` 只用于"避免连带改写**别人的**文件"，不能当"我改的文件已达标"的证据。
- 其余节奏（先写完整逻辑 → 收尾整轮一次 → 原始日志不回传 LLM、只回传
  OK/改 N 文件/残余 N 诊断）与全局 skill 一致。

## 门禁命令
- 整包可直接跑：`cargo clippy --workspace --all-targets -- -D warnings`（`--no-deps` 可选，仅提速）。
- **历史**：2026-09-19 之前 vendored `crates/innospect` 的既有 lint 会挡住整包 `-D warnings`，
  那时必须 `--exclude innospect --no-deps`。该 fork 已退役（改用 registry `innospect 0.1.4`，
  见 `crates/libscoop/Cargo.toml` 与`.agents/docs/devlog/2026-09-19-innospect-registry-retire-fork.md`），
  旧写法已失效——`.agents/docs/active/audit/s7-s14-investigation-2026-09.md` 里带
  `〔pre-2026-09-19 …〕` 标记的行只作历史记录，别照抄。

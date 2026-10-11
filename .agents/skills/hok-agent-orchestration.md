---
name: hok-agent-orchestration
title: hok 多子代理并行开发（本仓落点）
description: hok 专属：把计划/审计条目分派给多个可续子代理并行做、或作为父会话收口时读——本仓热点文件与易撞符号、必须写进 prompt 的沙箱坑、门禁命令与批次分配、决策与偏离的登记落点。通用编排纪律见全局 agent-orchestration。
metadata:
  type: project
---

**通用编排纪律见全局 `agent-orchestration`**（派活前的文件+语义重叠矩阵、prompt 契约、决策门、并发上限、
证据纪律、收口检查单、门禁分配、交接文档）。**机制层**（worktree/分支/合并脚本/沙箱坑）见
[[hok-worktree-parallel]]。本文件只记 hok 落点，**不复述通用条款**。

## 1. 本仓热点与易撞符号（派活前逐条对）

- **热点文件**：`src/cmd/mod.rs`（命令注册表）、`tests/exit_codes.rs`、i18n 资源（`locales/*.yml`）、
  `README*`；汇总型热点文件的清单与"只在主树改"规则见 [`.agents/AGENTS.md`](../AGENTS.md) → 硬约束 1。
- **易撞符号（只看文件看不出来）**：`internal::string::matches_any_glob` 这类共享谓词、`internal::json` 单入口、
  `Session` 上的共享布局方法——两条会话各改同一符号的不同特性，是白多花一次冲突的典型。
  ⇒ 派活前先 grep 目标符号的引用点，**共享符号只派一条**。
- **父会话自己也在改的文件**要与子代理改动面求交集，重叠的先在自己的 `git status --short` 里认出来，
  再决定让路（`git stash push -- <files>` → ff → `pop`）。

## 2. 必须写进子代理 prompt 的本仓坑

- 沙箱坑（本体见 [[hok-worktree-parallel]] → 本仓事实）：`rebase --continue` 的编辑器走不通 / 提交信息一律
  `-F <文件>` / 有未提交改动时禁 `reset --hard` / `-VerifiedRef` 要求"与被测树逐字节相同"。
- 新 worktree 先 `cargo build -p hok-shim`，否则 `crates/libscoop/build.rs:204` panic；
  且 cargo 要设 `$env:RUSTC_WRAPPER=''`（sccache 在全新 worktree 里编不了 C 依赖）。
- 子代理**不写汇总型热点文件**（写了也白写，并行会话必撞）、**不合主线**（只报"可合并 tip"）、
  **不派生下一级子代理**。

## 3. 门禁命令与批次分配（判据见全局 `agent-orchestration`）

- **子代理自测一次**（它唯一能自我证伪的机会）：`cargo clippy -p libscoop --all-targets -- -D warnings`
  + `cargo test -p libscoop` + 改动文件的 fmt 检查（**检查模式写法见 [[hok-rust-gate-practice]]**）。
- **主干一次完整门禁，在最终树上**：`cargo test --workspace`（含根 `tests/*`）+
  `clippy --workspace --all-targets` + 三个文档门禁（`devlog-index --check` / `doc-links` / `docs-index-check`）。
- **顺序固定**：重放/解冲突 → 重跑门禁 → 以**重放后的 tip** 当 `-VerifiedRef` 再 ff
  （base 落后时先 ff 必然报漂移，那是预期不是错误）。
- 不要为了省而砍掉**新写的、能反驳自己假设的那条用例**；也别对同一棵树连跑两遍门禁。

## 4. 决策与记录的落点

- 用户拍板的决定（含"有意偏离"）逐条登记到计划/审计对应行——否则下次分不清"决定"与"疏忽"。
- 每个落地会话的固定回填清单一律见 [[hok-worktree-parallel]] → 「收尾四件套」。
- 换对话前把交接写进计划文档（起点 tip、余项与建议、可复制的开场 prompt、必读注意事项）；
  口头交接等于没交接。

# Refactor 计划与提示词（专用目录）

> 本目录存放 **Edition 2024 重构**的计划与各阶段/专题的开工提示词。重构已于 2026-09 完成 ⇒
> **本目录整体转为历史**：计划与提示词只作回溯，不再驱动新工作（归档判定见 [`../../AGENTS.md`](../../AGENTS.md) R4）。
> 移动自 `docs/` 根目录（2026-08 整理）；2026-09 随 `docs/` 迁入 `.agents/docs/`。配套行为锚：官方 Scoop 源码在 hok 的兄弟目录 `../Scoop/` 的 `lib|libexec`。

## 计划

| 文件 | 内容 |
|---|---|
| [plan.md](plan.md) | Edition 2024 重构总计划（阶段 0–5 路线 / 风险保障 / 执行日志） |
| [tokio-purity-plan.md](tokio-purity-plan.md) | tokio 迁移现状核查 + 全面纯净化决策 + P1–P4 路线 |

## 提示词

| 文件 | 内容 |
|---|---|
| [prompts/phase-3-prompt.md](prompts/phase-3-prompt.md) | 阶段 3：infra trait 化（network/git/archive + 内存 fake） |
| [prompts/phase-4-prompt.md](prompts/phase-4-prompt.md) | 阶段 4：operations 异步化（tokio，方案 B + reqwest） |
| [prompts/phase-5-prompt.md](prompts/phase-5-prompt.md) | 阶段 5：护栏（unsafe deny / criterion / vs Scoop 基准） |
| [prompts/purity/](prompts/purity/) | 纯净化 P1–P4 开工提示词（见其 README） |

## 约定
- **用法**：每阶段新开对话后，把对应提示词文件整段贴给 agent 开工。
- **顺序纪律**：P4（事件总线 flume→mpsc）最高风险，须在 P1–P3 全绿之后做。
- **行为等价是生命线**：每步 `cargo build + cargo test --workspace` 全绿 checkpoint；只修本次改动 hunk（禁整包 fmt）。
- 提交规范：一个主题一个 commit；`git -c core.autocrlf=false`，eol=lf；gpg 签名前 `$env:HOME=$env:USERPROFILE`。

# Tokio 纯净化阶段提示词（P1–P4）

> 位置：`refactor/edition-2024` 分支 · 配套 `.agents/docs/reports/refactor/tokio-purity-plan.md`（决策：全面纯净化）
> 用法：每阶段新开对话后，把对应 `pN-*.md` 整段贴给 agent 开工。

| 阶段 | 主题 | 提示词文件 | 状态 |
|---|---|---|---|
| P1 | blocking 库包 `spawn_blocking` | `p1-spawn-blocking-prompt.md` | ✅ |
| P2 | `CancellationToken` + Ctrl-C 取消传播 | `p2-cancellation-prompt.md` | ✅ |
| P3 | 入口 `#[tokio::main]` async main | `p3-tokio-main-prompt.md` | ☐ |
| P4 | 事件总线 flume→`tokio::mpsc` + 事件循环线程→task | `p4-eventbus-mpsc-prompt.md` | ☐ |

## 顺序纪律
P4 是最高风险，必须在 P1–P3 全绿、行为被测试钉死之后才做。
每步 `cargo build + cargo test --workspace` 全绿 checkpoint，按分组提交规范逐主题提交。

# 规则复述收敛（7 组）与根 AGENTS.md 两处措辞

> 2026-09-26 ｜ 3 提交 `9eea4a6` `c2fa597` `e5c516f`，worktree 主树（仓库根），分支 `refactor/edition-2024`

**起因**：闸门锚点审计（`e1a2bce`）§3 的 3 项待办——复述收敛、根 `AGENTS.md` 的命令计数措辞、主线名措辞。
本会话照单执行，零重新调研；判据用「复述 vs 引用」与「谁最常在该场景被读到」。

**一、复述收敛（`9eea4a6` + `c2fa597`）**：7 组各收敛到唯一本体，其余降级为指针。

- 热点文件清单 → `.agents/AGENTS.md` 硬约束 1（唯一带理由）；根 Parallel work、`hok-worktree-parallel`、
  `hok-agent-orchestration` 改指它，`hok-worktree-parallel` 只留本域特有的「子代理禁止写」。
- 主线名来源 → `hok-worktree-parallel`「本仓事实」（本机值 + 重设命令）；根不再把主线写成 `main`，
  `pending-tasks.md` 基线不再复述本机值（会漂）。
- 收尾四件套 → `hok-worktree-parallel` §收尾四件套；根 Before landing 与 `hok-agent-orchestration` §7 改指它。
- 语言约定 → `.agents/AGENTS.md`；根只剩一句指向。`hok-report-archive` 删掉复刻的 R1 双轴表。
- 技能 `description`（`hok-worktree-parallel` / `scoop-gap-audit` / `hok-code-organization`）
  只留「谁在什么场景加载」，保留 `P0–P3` 这类发现关键词。

**二、去重门槛归属：推翻审计原建议（`c2fa597`）**：审计原判「保留 `hok-code-organization`」，但那会让
全局技能反向依赖某个仓库的技能（与刚立的技能根边界冲突）。改走：**通用门槛留全局 `proactive-review` §7**，
仓内只留 P2-6 判例数字与「解决什么真实问题」，根 Before implementing 只点名两者、不写参数。

**三、根 `AGENTS.md` 两处措辞（`9eea4a6` + `e5c516f`）**：① 命令计数改指 `src/cmd/mod.rs` 的 `Command` 枚举
（实测 **37** 个变体 = 命令数；`src/cmd/` 目录 39 个条目，含生成物 `__cmd_reg__.rs` 与 `shared_args.rs`），
并补上「加命令还要加变体」这一步；② 主线名不再写死 `main`。

**教训（两条）**：

1. **审计建议也要过边界判据**：原建议在「谁写得全」上成立，却违反了「全局技能不反向依赖仓库技能」——
   复述收敛的取舍标准是**归属边界**，不只是重复度。文档给了建议，现场仍要按边界复核一次。
2. **自包含卡片是刻意例外**：`pending-tasks.md` §8.5 收口检查单 / §8.8 开工卡片保留清单复述（它们要被整段
   复制进 prompt）；其中的 `git rev-parse … $(git config …)` 是**使用**不是**规则陈述**。这条边界写进审计 §3。

**未落地 / 去向**：无。审计 §3 已从待办改为处置记录，§1 两行本体随之更正；三个文档脚本 exit 0。

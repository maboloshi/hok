# NSIS 上游提案全部闭环，记录从「待提交」改为「已合并」

> 2026-10-05 ｜ 提交 `2a8d91b` ｜ worktree `.` ｜ 分支 `refactor/edition-2024`

用户指出 NSIS 上游提案应记为「上游已合并」。按 GitHub API 逐项核对（2026-10-05）后确认：hok 在
`ATRAPSLLC/nsis-rs` 的 4 项提案全部由上游合并，而仓内仍有四处写成「待提交 / OPEN」。

| 项 | 上游结局 |
|---|---|
| issue #2 `dest_path()` 目录跟踪 | 上游 [PR #19](https://github.com/ATRAPSLLC/nsis-rs/pull/19) 修复，0.4.0 发布 |
| issue #3 solid 截断/吞错 | 上游 [PR #8](https://github.com/ATRAPSLLC/nsis-rs/pull/8) 修复（`SolidStatus` + `Error::OutputTooLarge`） |
| issue #4 / PR #5 lzma-rust2 + zlib-rs | [PR #5](https://github.com/ATRAPSLLC/nsis-rs/pull/5) 合并，提交 `201df25`（merge `2e454b7`），0.4.0 发布 |
| innospect PR #3 | 0.1.4（2026-09-18）合并 |

**改动**（只改状态，提案正文一律不动）：`legacy/nsis-issue.md` 文首加「闭环」块 + §PR 3 的 OPEN 改「已合并」
+ 文末「切回 registry」标已完成；`pending-tasks.md` 的「NSIS 上游提案」行改为 ✅ 闭环；
`research/nsis-research.md` 的 §5 P1 与 §9 关联改为「上游已根治 / 只作历史」；
技能 `hok-nsis-extraction` 的 P1 尾注与 How-to-apply 同步（不再暗示有待提交项）。

**顺带记录（未处理）**：上游已是 **0.5.0（2026-09-27）**，含 breaking API（`ParamType::Variable` 带
`Access`、`param_layout()` 取 `NsisVersion`、`OPCODES` 74 项）；hok 仍钉 `nsis = "0.4"`，本次只登记不升级。

**教训**：上游吸收我们的提案后，本仓的「提案草稿」与「待办行」不会自动跟着变——**合并事实要主动回填**，
否则下一次检索会把已闭环项当成待办（这次就是用户先发现的）。

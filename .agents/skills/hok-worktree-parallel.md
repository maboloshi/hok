---
name: hok-worktree-parallel
title: hok 并行 worktree 专属细则
description: 在 hok 里开 worktree 并行干活、做主线 ff-only 落地、或做会话收尾回填时读：本仓专属的 worktree 细则、沙箱坑与收尾四件套（通用纪律与脚本见全局 worktree-parallel）。
metadata:
  type: project
---

**通用纪律见全局 skill `worktree-parallel`**（一会话一 worktree、base 现取、独立构建目录、副本产物验证、显式 `git add`、rebase + `--ff-only`、冲突保双方语义、未跟踪文档只在主树改；§8 还带配套脚本 `scripts/worktree-merge.ps1` 与自测）。本文件只记 hok 落地细节；**多子代理并行编排纪律见 [[hok-agent-orchestration]]**（派活前的文件+语义重叠矩阵、并发上限 2–3、子代理 prompt 契约、决策门与证据纪律、收口落地检查单、换对话前的交接文档模式）。

## 本仓事实
- **全新 worktree 先建 shim 再构建**：`cargo build -p hok-shim`。否则第一次 `cargo test`/`cargo build` 会在 `crates/libscoop/build.rs:204` panic（`hok-shim binary not found … Run \`cargo build -p hok-shim\` first.`）——该 worktree 自己的 `target/` 里还没有 `hok-shim{,-gui}.exe`。
- **全新 worktree 的 cargo 还要绕开 sccache wrapper**：本机 `$CARGO_HOME/config.toml` 用 `rustc-wrapper` 指向 sccache，它在**全新** worktree 里编译 C 依赖（`libsqlite3-sys` / `zstd-sys` / `libz-sys`）时报 `Compiler not supported: Failed to detect showIncludes prefix`（clang-cl）。跑 cargo 前设 `$env:RUSTC_WRAPPER=''`；若看到 `could not parse TOML configuration`，先查那份 config 的值里有没有未转义的反斜杠路径（Windows 路径要正斜杠）。
- 主线是**线性历史**；只接受 `--ff-only` 前进，**禁 merge commit**（后落地者 rebase + 解冲突，保留双方语义）。
  `main` 是最终目标线（当前落后于开发线很多；"合并到 main / push / 版本号"是独立主题）。
- 主线分支名由 `git config worktree-merge.mainbranch` 决定（脚本据此解析主线，无需每次传 `-MainBranch`）。
  **这是机器本地配置、不入库**——本机当前值是开发线 `refactor/edition-2024`；改主线/换分支后必须重设：
  `git config worktree-merge.mainbranch <branch>`。
- **已入库文档**：`.agents/**`、`AGENTS.md`、`CLAUDE.md`、`docs/` 每个 worktree 都有独立副本，正常改、正常提交。
- **汇总型热点文件（例外，仍只在主树改）**：清单与理由见 [`.agents/AGENTS.md`](../AGENTS.md) → 硬约束 1；
  本域特有的一条：**子代理禁止写，回填由父会话做**（它们收尾都会写，并行会话必撞）。
- 仍未跟踪的本机物：`reasonix.toml`（父目录）、`.agents/tmp/`。
- worktree 放 `.worktrees/<会话>-<slug>`；忽略规则在根 `.gitignore`（`/.worktrees`）。各 worktree 用自己的 `target/`。
- **受限沙箱下出站 HTTPS 不可用**：`workspace-write` 模式下 TLS 报 schannel `SEC_E_NO_CREDENTIALS`
  （同一条请求在 `danger-full-access` 下正常；Node 自带 CA 不受影响）⇒ 真实源冒烟只能用本地夹具/回环，
  或先放宽沙箱；**拉新 crate 版本要先在放宽的沙箱里跑一次 `cargo fetch`**，缓存落盘后普通构建回到受限沙箱即可。

## 合并工具（全局技能包）
脚本在**全局技能包**里（主仓库与主线均自动推导），**单一实体，本仓不保留副本**：
```powershell
pwsh "$HOME/.agents/skills/worktree-parallel/scripts/worktree-merge.ps1" -Worktree .worktrees/s5-checkup [-VerifiedRef <实测过的 ref>] [-DryRun]
```
- 用法与退出码 `0/1/2/3` 见全局 `worktree-parallel` 的 `scripts/README.md`；本仓无需传 `-MainRepo`/`-MainBranch`。
- 手工兜底序列见全局 `worktree-parallel` §5。⚠️ **本机沙箱下 `git rebase --continue` 的"确认提交信息"这一步走不通**：msys 的 `true.exe`/`sh.exe` 起不来（`fatal error - couldn't create signal pipe, Win32 error 5`），`GIT_EDITOR=true`、`core.editor=:`、`cmd /c exit 0`、乃至自编的原生 no-op exe 都失败（git 会经 `sh -c` 启动编辑器）。
  **实测可行替代（两条并行分支均已实践过）**：
  ```powershell
  git -C <worktree> branch -f <备份名> <当前 tip>      # 先留退路
  git -C <worktree> rebase --abort                    # 若已卡住
  git -C <worktree> reset --hard <主线当前 tip>
  # 逐提交重放（每个 commit 一次）：
  git -C <worktree> cherry-pick -n <sha>              # 冲突则手工解
  #   → 解冲突 → git add <files>
  git -C <worktree> commit -C <sha>                   # 复用原作者/信息，不开编辑器
  git -C <worktree> cherry-pick --quit                # 清 sequencer 状态后再做下一个
  ```
  注意 `commit -C <sha>` 保留原作者与 message，`commit.gpgsign` 仍会签名；重放完在树上重跑门禁，再用 `worktree-merge.ps1 -VerifiedRef <新 tip>` 做 ff（此时漂移校验要求"与受测树逐字节相同"，正好成立）。
- **`-VerifiedRef` 的语义陷阱**：它要求 rebase 后的树与"你实际测过的那棵树"**逐字节相同**。若分支 base 落后于主线，rebase 后树必然包含主线的进展 ⇒ 校验必然报 `ERR the rebased tree differs`（这是**预期**，不是漂移）。正确顺序是：重放/解冲突 → **在重放后的树上重跑门禁** → 用 `-VerifiedRef <重放后的 tip>` 再跑一次脚本做 ff。
- ⚠️ **提交信息一律走 `-F <文件>`，不要用 `-m "..."` 内联**：PowerShell 会把反引号与 `$` 当转义/变量展开，损坏过的提交信息只能靠 `git fsck --unreachable` 找回悬空 blob 重建。写信息到临时文件再 `git commit -F <file>`。
- ⚠️ **有未提交改动时绝不 `reset --hard`**：它两次吞掉未提交改动。要动引用前先 `git stash` 或先提交。
- ⚠️ **收口脚本失败时仍要核对暂存区**（`.agents/**` 已入库，"必须保持未跟踪"这条不变量已失效，但夹带无关改动仍要拦）：worktree 处于 **detached HEAD**（子代理重放后没把分支引用摆正）时，脚本报 `ERR the worktree is on a detached HEAD`，随后"保存当前工作区状态"的尝试失败，**可能把无关文档整批误暂存进索引**；被它重写的文档 mtime 会变（编辑器报"文件已变，需重读"，**内容其实完好**，别误判为被覆盖）。
  **排查/修复**：`git -C <repo> diff --cached --name-only` 看有无暂存项 → **`git -C <repo> reset`**（mixed，只动索引，工作区不受影响）→ 复查 `git status --short` 无多余暂存项。
  **预防**：落地前先确认 worktree 不在 detached HEAD（`git -C <worktree> rev-parse --abbrev-ref HEAD` 应为分支名）；子代理若用 `cherry-pick -n` 重放，必须让它把分支引用指到新 tip（`git branch -f <branch> <tip>`）再报"可合并 tip"。
- 自测（须在本仓根运行）：`pwsh "$HOME/.agents/skills/worktree-parallel/scripts/test-worktree-merge.ps1"`。

## 收尾四件套（固定动作）
① 活动计划 [`.agents/docs/active/plan/pending-tasks.md`](../docs/active/plan/pending-tasks.md) 自己那行（该表的存续项都在这里）；② `.agents/docs/active/audit/command-deviation-audit-2026-09.md` 对应条/状态行（对标项则改 [`scoop-gap-status.md`](../docs/active/audit/scoop-gap-status.md)）；③ **写一条 devlog**（触发与格式见 [`docs/AGENTS.md` R7](../docs/AGENTS.md)），再跑 `node .agents/scripts/devlog-index.mjs`（索引自动生成，禁手改）——**不再往技能里追加落地记录**（判据见 [`skills/AGENTS.md`](AGENTS.md)）。报告归档见 [[hok-report-archive]]。


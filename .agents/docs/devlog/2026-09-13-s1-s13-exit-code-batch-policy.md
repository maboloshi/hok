# S1+S13 退出码口径 + 安装/卸载批量行为

> 2026-09-13 ｜ 已闭环，4 commit，分支 `fix/exit-code-and-batch-policy`，worktree `.worktrees/s1-s13`

**落地形态**：4 个提交 `4a1fd5a`（clap 层）/ `e7d1e25`（逐命令）/ `a4ea0b7`（批量行为）/ `cfd44ed`（download）——**先 rebase 到 S2 的 `3c90095`，再线性 ff 进 `refactor/edition-2024`（tip `cfd44ed`）**；本仓历史保持线性，**不使用 merge commit**。并行/合并/未跟踪文档同步的通用规则见 [[worktree-parallel]]（hok 细则见 [[hok-worktree-parallel]]）。
**决策**：S1 选 **A 全面对齐官方逐命令退出码**；S13 只对齐**运行中进程**与**未安装应用**两类，`scoop` 自身不加特例，held 跳过/被依赖拒绝**保留**为有意偏离。处置逐条映射见 `.agents/docs/active/audit/command-deviation-audit-2026-09.md` §10。
**实现要点（勿踩）**：
- **clap 层**（`src/cmd/mod.rs::exit_for_parse_error`）：不再用 `get_matches_from`（会自行 exit 2），改 `try_get_matches_from` + 自渲染；`DisplayHelp/DisplayVersion` 与「**无子命令 token** 的 `DisplayHelpOnMissingArgumentOrSubcommand`」→ **0**，其余 → **1**。`hok -v`/`hok --lang en`（只带全局选项）仍为 1（官方无对应场景）。
- **运行中进程**：`check_not_running` 打 WARN 后由调用点决定——默认返回 `RunningCheck::Running` 跳过该包，`ignore_running_processes=true` 时返回 `RunningCheck::Ignored` **继续**（官方 `lib/install.ps1:534-551`：默认分支 `return $true` → 调用者 skip，ignore 分支 `return $false` → 调用者继续）。进程明细走 `Output::Detail`，只在 `-vv` 渲染。**注**：本条最初落成"两支都跳过、该键只决定 warn/error 文案"，系把 `return $false` 误读为"仍在运行"，当日已回正，见 devlog [`2026-09-13-running-process-output.md`](../devlog/2026-09-13-running-process-output.md)。
- **未安装应用**：`remove()`/`install()`(only_upgrade) 报 `cmd.not_installed` 后 `continue`，不再 `Err(PackageNotFound)`；原先为它而设的「是否真有已装目标」抑制 `Everything is ok!` 尾行的守卫，已随该页脚整体删除而**一并移除**（2026-09-13 晚）。
- **shim add**：`add_custom` 现在校验目标存在（官方 `Test-Path`），新增 `Error::ShimTargetMissing` → CLI exit 3；`shim info` 未找到时先翻转 scope 再查，命中另一作用域 → 2 + 提示。
- **virustotal**：按 query 未命中数置位 8（可与 2/4 叠加），`queries.clone()` 传入 query（原先被 move）。
**测试**：新增 `tests/exit_codes.rs`（隔离 `SCOOP`/`USERPROFILE` 空根，锁 `hok`=0、未知命令=1、缺参=1、`which`=2、`bucket list`=2、`shim`=3、`virustotal`=8、未安装=0）；libscoop `check_not_running_skips_by_default_and_continues_when_ignored`（复制 `cmd.exe` 到 `apps/<n>` 后跑 `ping`，断言默认 skip、开键 continue，两者都只 WARN 且明细跟在消息之后）。`tests/cli_help_i18n.rs` 的用法错断言 2→1。
**已知环境噪音**：`cargo test --workspace` 里 `permission::tests::grant_users_write_inherit_*` 失败（临时目录 ACL error 5），**在未改动的主树同样失败**，与本次无关。
**⚠️ 合并注意（已解决）**：与 S2（`3c90095`）在 `src/cmd/mod.rs` 的 parse 错误路径冲突——**已按此解决**（rebase 到 `3c90095`，tip `cfd44ed`，线性 ff 无 merge commit）：保留 S2 的 `InvalidSubcommand`→alias 分发，未命中回落 `exit_for_parse_error(err, &raw)`（**没有**保留 `err.exit()`，否则退出码退回 clap 2）；`normalize_subcommand_case` 形参改为 `&[OsString]` 并复用 S2 的 `command_names`；`src/lib.rs` 的 `translate_error` 同时保留 S2 的 alias 错误臂 + 新增 `ShimTargetMissing`。合并后实测：`hok`=0 / 未知命令=1 / 缺参=1 / `search` 无匹配=1 / 空 `bucket list`=2 / `shim rm`=3 / `virustotal`=8 / 未安装=0，且 S2 别名仍可分发执行（`hok <alias>` 透传退出码）。
**Why:** 退出码是脚本兼容面；「运行中进程中止整批」与「`ignore_running_processes` 同名异义」是两处会误导用户的语义错位。
**How to apply:** 剩余语义绑定的退出码（`reset <未安装>`、`shim`/`alias` 无子命令）随各自会话（S7/S14）一起改（`cleanup` 无参、裸 `cache` 已由 S6 落地，见 devlog [`2026-09-13-s6-cleanup-cache-safety.md`](../devlog/2026-09-13-s6-cleanup-cache-safety.md)）；新增命令若需专用码，用 `crate::ExitCode(n)` 而不是新开关。

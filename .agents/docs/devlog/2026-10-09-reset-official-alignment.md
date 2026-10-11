# reset 对齐官方：位置参数全为 app + 全局自动识别

> 2026-10-09 ｜ 提交 `eddc682` ｜ worktree `.worktrees/ca3-reset-alignment` ｜ 分支 `feat/reset-official-alignment`

## 破坏性改动（用户拍板）

`hok reset` 的**所有位置参数一律是 app**（`app` / `bucket/app` / `app@version`），与官方
`libexec/scoop-reset.ps1:16,31` 一致；hok 原有的 `[version]` 位置参数**删除**。于是旧形态
`hok reset <app> <ver>` 不再表示切版本：两个 token 都会被当 app，第二个按官方语义报 `'…' isn't installed`，
整条仍 exit 0。版本切换改由 `app@version` 表达。解析复用既有的 `package::identity::parse_app`，
**没有**新写第二套 parser；`bucket` 部分按官方只解析不使用（官方也不把它传给 `installed`）。

## 结构：整条官方循环下沉 libscoop

多应用循环、全局自动识别、`(k/N)` 计数一并搬进 `package::sync::reset(session, &queries, all)`，
CLI 侧 `src/cmd/reset.rs` 只剩参数、调度与 `drain_steps`（**−58 行**）。`-a` 与显式 app 两条路径共用同一条管线，
不再各写一遍：

- 逐应用 **error+continue**（官方 `ForEach` + `return`），失败不中断批次；
- 全局自动识别 = 官方 `installed $app $true`（`:33-36`），命中则走 global 作用域，再按 `:60-63` 要求 admin；
- **编号只数会真正尝试的 app** ⇒ 被跳过/不可用的应用不占号，`(k/N)` 保持连续（这是 `-a` 路径原本的设计约束）；
- 新增 `cmd.not_installed_version` 对齐官方 `'$app ($version)' isn't installed`（`:56`）。

`reset_eligible` 被管线自身的资格判定取代、转为私有并从 `sync.rs` 的 re-export 移除。
**保留**：`reset` 多跑 `post_install`（登记在案的有意偏离），本次未动。

## 证据与覆盖缺口

- `tests/exit_codes.rs::reset_positionals_are_all_apps` 用「isn't installed 出现**次数**」钉住新语义：
  `reset app 1.0.0` → **2 次**（旧语义只 1 次）、`reset app@1.0.0` → **1 次**、`reset a b c` → **3 次**，三者都 exit 0。
- libscoop 侧：`app@version` 切版本、`bucket/app`、`a ghost b` 用 `PackageCommitStart` 事件断言编号恰为
  `(alpha,1,2),(beta,2,2)`（ghost 报错且不占号）、全局谓词真/假各一条。
- **本机跑不到**：非管理员分支。`Session::is_admin()` 直接读真实进程令牌、没有测试注入缝，本机即非管理员，
  所以实际走的是 warn+skip 那支；admin 支代码已写但**未被测试覆盖**——这是本条验收里唯一没做到的。
- README / README-zh 实测**无需改**：只有命令清单摘要与 `post_install` 说明，没有用法行、也没有 `<version>`。

## 一次事故（已复原）

过程中有三处改动误落到**主树**（`sync_remove.rs` / `sync.rs` / `reset.rs`）。收了 `git restore --source=HEAD`
精确还原，落地前我复核过主树：HEAD = `505116b`、`git status` 与 `git diff HEAD` 均空、无游离未跟踪文件。
最终改动全部只在 worktree 内。

## 落地

base `8280d8b` 已被同日的提交整理（autostash 四并二）重写，故用 `rebase --onto 505116b 8280d8b` 只重放本提交
（直接 `rebase` 会连带重放被重写的历史而冲突）。重放后在**新树**上重跑门禁：`clippy -p libscoop` 0、
`test -p libscoop` 0（868 passed / 0 failed）、`test -p hok` 0（含新用例）、改动文件 rustfmt 0；`--ff-only` 落地。

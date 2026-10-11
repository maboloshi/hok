# Wave 1：`checkver` 挂起 + 无 home panic

> 2026-09-13 ｜ 2 子代理并行，8 提交线性落地，主线 `4748ae1` → `43795484`

## 背景与编排

两条独立会话并行（`hok-agent-orchestration` 的文件 + 语义重叠矩阵：`package/checkver.rs` vs `internal/fs.rs`/`config.rs` **零交集**），各带 worktree（`.worktrees/r1-checkver-hang`、`.worktrees/r2-env-panic`），先出调研报告、用户拍板决策门后才动代码。父会话负责收口（`worktree-merge.ps1 -VerifiedRef … --ff-only`）、独立复核与文档回填。

## R2（先落地，3 提交 → `9b672207`）

`4748ae1` 上清空 `USERPROFILE`/`HOME` 后**所有建 session 的命令**都 panic（`internal/fs.rs:63:30`，exit `-1073740791`）。子代理独立复现并**用证据更正了父会话的归因**：写链上的空路径来自 `hok_config_path()`（`config.rs:889-891` 经 `config.rs:758`），而非 `scoop_config_path()`（后者只喂 `ConfigBuilder::scoop_path`，只读）。修法三件：`fs.rs:63/115` 的 `parent().unwrap()` → `Error::Io(InvalidInput)`；`init_default` 跳过空路径；**空串 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE` 视为未设置**（对齐官方 `core.ps1:1291-1301`）。按证据删掉因此不可达的守卫。

## R1（后落地，5 提交 → `43795484`）

`hok checkver -d <空目录>` 无限挂起（无输出、无 CPU）。子代理定位到 `checkver.rs:383` 的 `.buffer_unordered(pending.len())`：**容量 0 时 futures 永不 poll 源流**（vendored `futures-util-0.3.34/.../buffer_unordered.rs:62-86`）⇒ `Pending` 且无 waker ⇒ `collect().await` 永久挂起。父会话逐行核验该源码，确认结论。**触发面比题设宽**：空目录 / 全无 checkver / glob 不匹配 / **任何 `--version`**（实测先打印版本行再挂死）。修法 `.max(1)` + 四类回归 + 清陈旧 TODO + 同类站点 debug-only 断言。

## 教训

1. **`buffer_unordered(0)` 是静默死锁**：零容量不会空跑，而是「源流永不被 poll ⇒ 永不 `Ready(None)` ⇒ 无 waker 的 `Pending`」。任何给流式并发设容量的地方，容量必须 ≥1；诊断这类挂起要读 vendored 的 poll 实现，不要猜调度。
2. **静态读码的归因会被实测推翻**：我把写链来源断成 `scoop_config_path()`，子代理用「写入目标实测」（只创建了 `hok_config_path` 那个文件）推翻。`config.rs` 里两个同源空路径极易混，判断"谁进写链"必须看**写入目标**，不能看函数名。
3. **长任务用同一份状态两次核对**：R2 首轮做完后用户追加"方案 A"，第二次收尾才发现首轮加的 `require_global_path`/`ensure_global` 守卫**变不可达**，一并删除——同一主题分两轮做时，第二轮要重新审视第一轮的产物是否仍必要。
4. **沙箱策略会改变门禁口径**：`workspace-write` 下那两条 ACL 测试必失败（`error 5`），切到 `danger-full-access` 后全绿（747/0）。**"已知失败"要标注它依赖的环境条件**，否则下一轮会把环境噪声当回归、或把回归当噪声。
5. **`%TEMP%` 沙箱副本跑签名提交会失败**：`worktree-merge.ps1` 自测在 `workspace-write` 下报 gpg 无法写 `~/.gnupg`（`selftest_exit=1`），放宽权限后 **15/15 通过**。工具自测失败先分清是工具坏了还是权限不够。
6. **决策门用"父会话转达"比让用户切会话更快**：子代理被别的 agent 拥有时 `ask_user_question` 不可用（其会话内被拒），把问题带回父会话集中答复再 `send_message` 转达，实测一个来回就能开工。
7. **子代理收尾前要同步主线快照**：R2 落地后主线前移，R1 的 base 立刻落后；提前通知它"自行重放到新 tip → 在重放后的树上重跑门禁 → 把分支引用指到新 tip"，避免收口时因 `-VerifiedRef` 口径白跑一轮（S1 5/5 零冲突重放）。

---

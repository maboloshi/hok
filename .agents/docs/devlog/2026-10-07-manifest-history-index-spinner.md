# 清单历史索引补进度圈：管道已实测、TTY 交用户自测；外加一次「打真环境」的事故记录

> 2026-10-07 ｜ 提交 `7441be7` ｜ worktree `D:\App\github项目\Scoop\hok`（主树，非 worktree）｜ 分支 `refactor/edition-2024`

## 落地项

- `src/cmd/update.rs` 新增 `refresh_manifest_history`：形状照抄隔壁的 `refresh_manifest_cache`——TTY 走 `cui::RowUI` 一圈圈行，
  管道走 `output::status`/`done` 两行（indicatif 在非 TTY 什么都不画）。3 个 i18n 键（`cmd.history_index{,_done,_skipped}`）。
- 失败**故意不做成缓存那步的红色 `✗`**：索引是派生数据、`hok update` 必须在它坏掉时照样成功，故只打一行暗色 note 收尾。
- 依据：冷启动首次种子实测 extras 92,616 提交 22 s / main 81,453 提交 14 s（16 线程），单线程 extras 97 s；热了以后是毫秒级。

## 证据档位（如实标注）

- **管道路径：实测通过**（隔离副本上跑真 `hok.exe update`）：`正在维护清单历史索引... / ✓ 清单历史索引已更新。`
- **TTY 路径：未取证**。仓库自带的 `.agents/scripts/cleanup-bench/conpty-capture.rs` 在我这儿没把子进程挂上伪控制台——
  用 `cmd /c echo hello-from-pty` 对照诊断：子进程输出直接到了宿主控制台、capture 文件根本没生成（该 exe 已确认是 GUI 子系统、`STARTF_USESTDHANDLES` 也在）。
  故 TTY 分支只能声明"与已验证的缓存那步同形"（同一个 `RowUI`、同样 `set_spin`/`done_with`；`cui.rs:79` 的 `enable_steady_tick` 保证主线程阻塞时圈圈照样转）。用户自行实测。

## ⚠️ 事故：三次 `hok update` 打到了真环境（`D:\App\Scoop`）

- **怎么发生的**：我误以为"把 `config.json` 放在 exe 旁边"就能隔离 CLI。实际上 CLI 固定读 `~/.config/hok/config.json`
  （`config.rs` 开头所谓"3 个可能位置"是**首次迁移**时读 Scoop config 用的，与运行期无关），而那份配置没有 `root_path`，
  默认值来自 `$env:SCOOP = D:\App\Scoop`。于是我写进 `%TEMP%\mhi-cli\config.json` 的东西自始至终没被读。
- **实际影响**：真 `scoop.db` 被写入（53.1 MB→166.3 MB，含 723,960 行索引）、真 config 的 `last_update` 被刷新到 10:11:07、
  6 个桶各被 fetch 一次（**HEAD 未动**：reflog 最后一条 reset 是 10:01 用户自己的更新，我的 fetch 没带来新提交）。
  无数据丢失、无版本前进/回退。用户选 A：**保留现状**，不删索引行。
- **正确姿势**（写在这里，下次别再踩）：CLI 的隔离只能用 `$env:SCOOP`（+`SCOOP_CACHE`）指向副本；且 `last_update` 无论如何会写真配置；
  要零副作用就别走 CLI，用 `Session::new_with` 的 bench 程序（`test_utils::test_session` 同款）。
  另外：测量脚本里的"只读"承诺，必须在**每条命令前后**核对目标文件的 size+mtime，而不是只核对一次。

## 门禁

`rustfmt --edition 2024 --check src\cmd\update.rs` 干净；`cargo clippy --workspace --all-targets -D warnings` 干净（先修掉 `ui.note(&t!(..))` 的 `needless_borrow`）；`cargo test -p hok` 全绿。

## 未落地 / 去向

- TTY 圈圈：用户自测；若有文案或形态要调，是一条小改。
- 前一轮遗留：`pending-tasks.md` / `upstream-0.6.0-impact-2026-10.md` 的回填已在 `7260ab4` 完成；工作树 `.worktrees/mhi-history-index` 与 `stash@{0}`（2026-10-05 旧草稿）仍未清。

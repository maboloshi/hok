# 单步进度统一为 TTY 感知

> 2026-08-23 ｜ spinner / 纯文本，commit `f09d9e0`

**目标**：把 `output::status → done/ok` 的单步进度（Resolving / sizing / hash / commit 等）统一进一个 TTY 感知渲染层——TTY 转圈、完成定格；非 TTY（管道/重定向/CI）保持行内纯文本、**无 ANSI、进度不丢**。TTY 检测用 `std::io::IsTerminal`（stdout），**不加依赖**。`output::` 公开签名不变（TTY 感知收在渲染层）。

## 关键设计决策

1. **`status` 被"真单步进度对"与"报表/数据行"两用**——search/list -V/status/checkver/auto_pr/checkup/update/loading_cache 都用 `status` 打报表行。若直接让 `status` 全局起 spinner，报表命令会在 TTY 上留下一串永不完成的 spinner 帧 → **把报表调用挪到新增的纯文本 `status_line()`**（逐字节复刻历史 `status` 输出），`status/done/ok` 只承载真单步对。与用户确认方案 A。
2. **快步骤无噪音**：spinner 后台线程只在 `MIN_SPIN=300ms` 后才画帧；快于它的步骤保持静态文本，`done/ok` 直接定格，不产生转圈。
3. **非 TTY 真无 ANSI**：非 TTY 分支直接写纯文本（不用 `.green()` 等 Stylize）——crossterm 即使 `force_color_output(false)` 仍会留 `\x1b[m` reset，靠它做不到真无 ANSI。顺带修了历史**双点 bug**（locale 消息已含 `...`，旧 `status` 又加 `...` → `Checking hash......`）。
4. **线程安全**：spinner 线程持 `step.msg` 锁画帧；`step_finish` 先置 `stop` 再等同一把锁写完成 → completion 后线程读到 stop 退出，无残留帧。锁序一致（msg→OUT），无死锁。

## 实现

- `src/output.rs`：`is_tty()`、全局 `STEP`（当前 step）+ `OUT`（输出 sink，测试可换）、`spawn_spinner`（`std::thread` 后台动画，无新依赖）、`step_begin`/`step_finish`、新增 `status_line`。TTY 下重复 `status`（哈希逐文件）原地更新 spinner 消息；非 TTY 逐行输出。
- 调用点：真单步对保留 `status/done/ok`——scoop_handler 的 Resolving/Sizing/Downloading/Checking hash/Commit、`cmd/mod.rs` 的 `Output::Done/Ok` 转发、reinstall 的 release/re-hold；报表调用改 `status_line`（9 个文件）。
- **scope guard（明确不做、保持现状）**：bucket spinner（`BucketUpdateUI`）、下载 bars（`MultiProgressUI`）、手写 `\r\x1b[2K` 解压计数、报表/非进度输出（info/warn/err/header/field/named）、prompt、`output::` 公开签名、libscoop（只发 Event/Output、不写 stdout，未动）。

## 验证

`cargo build` ✅；`cargo clippy -p hok` 0 warnings ✅；`cargo test -p hok` 23 passed（含 6 个新确定性非 TTY 测试，断言纯文本无 ANSI）✅；`cargo test --workspace` 446 passed ✅。测试走非 TTY 文本路径（cargo test 的 stdout 是管道 → `is_terminal()=false`），确定性可断言。改动 hunk `rustfmt --edition 2024 --check` 合规（其余文件 import 排序/let-chain 是既有债，未动）。GPG `Good signature`（maboloshi）。

## 教训

1. 同一函数被"进度对"与"报表/数据行"复用时，直接全局加 spinner 会破坏报表输出——新增纯文本函数承载报表、让进度函数只做真进度，是"命名即文档"的落地（对应 hok-code-organization）。
2. 非 TTY 想真"无 ANSI"，不能靠 `.green()` + `force_color_output(false)`（crossterm 仍留 `\x1b[m` reset），要直接写纯文本。
3. 后台 spinner 线程必须与 `done/ok` 的完成写用同一把锁串行化（置 stop + 持锁写完），否则 completion 后会残留一帧 spinner。

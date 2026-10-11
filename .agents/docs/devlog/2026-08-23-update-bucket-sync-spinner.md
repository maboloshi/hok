# `hok update` 无包名并发 bucket 同步改用 spinner UI

> 2026-08-23 ｜ commit `a80ea33`
>
> 原引用 `2b9f5d0`，2026-08 重构后等价提交为 `a80ea33`。

**背景**：评估 rake 输出层后落地。hok 的 `update_buckets` 一直是**行内** `output::status("Updating 'x'...")` / `output::done("'x' updated.")`，而 `libscoop::bucket::update` 早已**并发**拉取（每 bucket 一个 tokio task，`bucket.rs:499-538`）。并发乱序完成时行内 status/done 配对被打散（`Updating 'main'...` 与 `'extras' updated.` 交错）。rake 的 spinner-per-bucket 视觉正是为并发场景设计，遂采纳。

**实现**（2 文件）：
- `src/cui.rs` 新增 `BucketUpdateUI`（indicatif `MultiProgress`，每 bucket 一根独立 spinner 行）：`start`（`⠋⠙⠹...` 80ms tick）→ `succeed`（原地 `✓ name` 绿）/ `fail`（`✗ name (msg)` 红）/ `finish_all`（清理残留）。✓/✗ 用 crossterm `Stylize`，走 hok 的 `--no-color` 控制。
- `src/cmd/update.rs` `update_buckets`：事件循环里用 `BucketUpdateUI` 替代行内 status/done；`libscoop` 并发逻辑原样未动。

**验证**：`cargo build` ✅；改动文件 `rustfmt --edition 2024 --check`（剩余 diff 均为既有不合规，未动）✅；`cargo clippy -p hok` 0 warnings ✅；`cargo test -p hok` 17 passed ✅；提交 `a80ea33`（仅 2 文件，GPG `Good signature`）。

**复用评估**：`BucketUpdateUI` 结构上通用（key→spinner 行），但 hok 里除 bucket 同步外唯一的"并发逐项+流式"场景是 **`checkver`**（`buffer_unordered` 并发查 manifest + 回调流式逐条渲染）。下载/update 用 bars（`MultiProgressUI`）非 spinner；checkurls/status/cleanup 为顺序报表。**决策**：暂不泛化（命名即文档），如将来 checkver 上 spinner 再抽通用组件。

## 教训
1. hok 的 bucket 更新**早已并发**，行内 status/done 在并发下必然串线——并发逐项操作应配"逐项独立进度行"（spinner）而非行内配对；
2. 评估"参考 X"前先核实 X 是否真做了：rake 的 spinner 值得参考，但 rake 的 bucket 更新其实是**顺序**的，hok 的并发更先进。

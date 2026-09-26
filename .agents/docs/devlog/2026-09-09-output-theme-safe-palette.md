# 输出配色统一为单一「主题安全」调色板

> 2026-09-09

**问题**（两个独立缺陷）
- **不统一**：除 `--no-color` 外各处直接调 crossterm `Stylize`，同一语义在不同命令里颜色不同——包名在 `commit_header`/`closure_*` 是 `white().bold()`、在 `list`/`status` 表格里却是 `dark_cyan()`；`checkurls` 表头 `cyan()` vs `output::header` 的 `dark_cyan()`；`change()` 标签 `dark_blue()` vs `field()` 标签 `dark_cyan()`；`lib.rs` 还手写了一套 `error()`（红 `ERROR `，不粗体），与 `output::err` 并存。
- **明亮主题看不清**：crossterm 的命名是反的——`white`/`grey` = ANSI **97/37**，`red`/`green`/`yellow`/`cyan`/`blue` = **高亮** 91/92/93/96/94（依据 crossterm 0.28 `src/style/types/colored.rs` 的 `Display for Colored` 映射）。这些在白底上几乎不可见（包名、✓、版本、WARN 全中招）；反过来 `dark_blue`(34) 在深色主题下过暗。

**方案**：`src/output.rs` 新增 `pub mod style`，九个语义角色——`accent`/`accent_bold`/`emphasis`/`success`/`warning`/`warning_bold`/`danger`/`danger_bold`/`dim`，只用普通亮度 ANSI（36/32/33/31/90）+ 默认前景加粗。
- `emphasis` = 粗体 + 默认前景：白底黑字、黑底白字都清晰（替代 `white().bold()`）。
- 次级信息 `grey()`(97) → `dim`(90)；箭头 `blue()`(94) 与 `change` 标签 `dark_blue()`(34) → `accent`。
- 所有命令改走角色函数（`cui`、`bucket`、`checkurls`、`list`、`status`、`virustotal`），不再 import `Stylize`。
- 顺带修：crossterm 的 `SetAttribute` 不检查 `NO_COLOR`，`--no-color` 下仍会漏出 `\x1b[1m`；角色函数在关闭时直接返回纯文本。`lib.rs` 手写 `error()` 删除，统一 `output::err`。
- `list` 的 Info 列改为**渲染时**着色（需关注词 danger、续行 dim），不再在构造阶段预染色。

**验证**：`cargo run -p hok -- list` 实测 ANSI 字节——表头 `38;5;6`+`1`、名称列纯 `1`、分隔线 `38;5;8`、需关注词 `38;5;1`；`-N list` 下 SGR 转义为 0（仅剩 eventloop 的 `?25h` 光标恢复）。新增 2 个单测（角色色码集合正确 / 关闭时纯文本），`cargo test -p hok` 19 passed。

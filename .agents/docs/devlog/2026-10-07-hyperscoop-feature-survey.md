# 调研：姊妹项目 hyperscoop（`hp`）值得参考的功能

> 2026-10-07 ｜ 提交 `89b7aca` ｜ worktree `.` （主线 `refactor/edition-2024`）｜ 分支 `refactor/edition-2024`

## 落地项（本次提交内容）

- 新增 `.agents/docs/research/hyperscoop-feature-survey-2026-10.md`：静态读码盘点 `../hyperscoop`（`Cargo.toml` = 4.5.1）的功能候选池，全部带 file:line。
- `.agents/docs/README.md`：登记该调研（R6）。

## 结论要点

- **第一梯队**（与「对标官方 Scoop」不冲突）：`--dry-run` 贯穿 install/uninstall/cleanup/autoremove；`uses` 反查（hok 卸载护栏已有 `sync_remove.rs:94`，缺的是查询面）；`status --json`；`size` 磁盘归因；`checkup` 补 shim/junction 检测（hok 现有 `checkup` 与 `health` 之间存在真空）；跨进程文件锁（hok 全仓无锁）。
- **第二梯队**：`installed_as_dependency` + `autoremove`（动共享 `install.json`，需先核实官方读取面）；`Hpfile` bundle（前置是先对齐官方 Scoopfile，hok `export/import` 的 schema 不兼容是既有 P1）。
- **反例**：`hp service`（硬编码候选 + `sc.exe` + `sleep 800ms`）、`hp merge`（直接删社区 bucket 的 manifest、版本按字符串比、map 键写错）、README 的性能/并行宣称与代码不符（无 memmap、bundle 串行、内置 aria2c.exe 5.39 MB）。

## 处置

**本会话不立项**（用户选择只保留调研结论）；需要时按文档 §4 的顺序逐项立案。

## 门禁状态

纯 `.agents/**` 文档改动，无 Rust 改动 ⇒ 不跑 rustfmt / clippy / cargo test。
`node .agents/scripts/doc-links.mjs`：169 篇文档、255 条相对链接，断链 0 ✅；
`node .agents/scripts/docs-index-check.mjs`：索引缺口 0 ✅。

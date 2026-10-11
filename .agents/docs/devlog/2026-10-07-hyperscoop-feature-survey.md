# 调研：姊妹项目 hyperscoop（`hp`）值得参考的功能

> 2026-10-07 ｜ 提交 `89b7aca` `def7895` `25f0419` ｜ worktree `.` （主线 `refactor/edition-2024`）｜ 分支 `refactor/edition-2024`

## 落地项（本次提交内容）

- 新增 `.agents/docs/research/hyperscoop-feature-survey-2026-10.md`：静态读码盘点 `../hyperscoop`（`Cargo.toml` = 4.5.1）的功能候选池，全部带 file:line。
- `.agents/docs/README.md`：登记该调研（R6）。
- `def7895` 补 §2「命令面差异的四种性质」：把「差异大」拆成保留同名 / 官方名降级为别名 / 真改名（`depends`→`deps`，且 `hp help` 被 `disable_help_subcommand` 关掉）/ 纯新增四类，并定下「只借纯新增命令，不改同名语义、不引别名、不开 `infer_subcommands`」的口径。
- `25f0419` 补 §0「依赖面实测」：用户指出 hp 是照抄 brew 的依赖模型，而 Scoop 的依赖面远没那么重。实测证实——main 55/1669（3.3%）、extras 67/2406（2.8%）、scoop-app 633/17818（3.6%）、versions 16/625（2.6%）声明 `depends`；main+extras 全图**只有 1 条传递边**（`cloud-sql-proxy → gcloud → 7zip19.00-helper`）；本机 66 个已装应用只有 2 个带 `depends`。据此 `uses` 降级、`autoremove` / `leaves` 移入「不建议」，文档 §5 顺序同步调整。

## 结论要点

- **第一梯队**（与「对标官方 Scoop」不冲突）：`--dry-run` 贯穿 install/uninstall/cleanup/autoremove；`status --json`；`size` 磁盘归因；`checkup` 补 shim/junction 检测（hok 现有 `checkup` 与 `health` 之间存在真空）；跨进程文件锁（hok 全仓无锁）。
- **降级 / 排除**：`uses` 降级（§0）；`autoremove` / `leaves` 排除（依赖面 3%、深度 1，且要动官方没有的 `install.json` 标记）。
- **单独立案**：`Hpfile` bundle（价值在声明式环境复制，与依赖面无关；前置是先对齐官方 Scoopfile——hok `export/import` schema 不兼容是既有 P1）。
- **反例**：`hp service`（硬编码候选 + `sc.exe` + `sleep 800ms`）、`hp merge`（直接删社区 bucket 的 manifest、版本按字符串比、map 键写错）、README 的性能/并行宣称与代码不符（无 memmap、bundle 串行、内置 aria2c.exe 5.39 MB）。

## 处置

**本会话不立项**（用户选择只保留调研结论）；需要时按文档 §5 的顺序逐项立案。

## 门禁状态

纯 `.agents/**` 文档改动，无 Rust 改动 ⇒ 不跑 rustfmt / clippy / cargo test。
`node .agents/scripts/doc-links.mjs`：170 篇文档、256 条相对链接，断链 0 ✅；
`node .agents/scripts/docs-index-check.mjs`：索引缺口 0 ✅。

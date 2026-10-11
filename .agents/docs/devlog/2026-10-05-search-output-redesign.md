# search 输出重做：无参数修正 + 表格布局参考 list/hp + 底部汇总

> 2026-10-05 ｜ 提交 `a1f580c` `44a56e0` ｜ worktree `.` （主线 `refactor/edition-2024`）｜ 分支 `refactor/edition-2024`

## 落地项（本次提交内容）

- `src/cmd/search.rs`：无参数时退出 1（`cmd.search_no_query`）；表格布局参考 `list.rs`（`accent_bold` 表头、`-` 规则线、`emphasis` 包名、`pad_visual` 对齐）；常驻列 `Name/Version/Source/Installed`，条件列 `Binary`（`--with-binary`）/ `Description`（`--with-description`）；版本差异在 `Installed` 列内显示 `已安装 → 新版本`；顶部计数行 + 底部汇总行（参考 `hyperscoop` / `list`）。
- `locales/en.yml`、`locales/zh.yml`：新增 `search_no_query`、`search_summary`。
- 设计规格已归档：`.agents/docs/reports/search-output-design.md`（原 `.agents/docs/active/spec/search-output-design.md`，已加 `Archived: 2026-10-05` 并移入 `reports/`）。

## 门禁状态

- `rustfmt --edition 2024 --check src/cmd/search.rs`：干净（无子模块，直接写入不触发递归）。
- `cargo clippy --workspace --all-targets -- -D warnings`：无警告退出。
- `cargo test`：全部通过（含 `missing_checkver_cli`、`root_env_fallback`、doc-tests）。

## 参考关系

表格样式参考：`hok list`（`src/cmd/list.rs:layout`）；计数提示参考 `hyperscoop`（`crates/command/src/search.rs`）；无参数行为参考 `scoop-search.ps1`（`libexec/scoop-search.ps1:195-200`）。设计演示文档内容已直接入本日志正文；归档文件内容冻结（`reports/` 永不编辑），若后续修订需新建 `active/spec/` 文档并在 `Status:` 行注明 `superseded-by` 链接。

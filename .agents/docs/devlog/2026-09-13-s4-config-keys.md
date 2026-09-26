# S4 `config` 键策略与子命令语义

> 2026-09-13 ｜ 已闭环，提交 `38ecfa9`/`f4300aa`/`4cefb81`/`b7c1680`/`7987205`/`98e28a1`/`cf1baec`，分支 `fix/config-key-and-subcommand`，worktree `.worktrees/s4-config`

**结论（用户拍板）**：`config` 行为对齐官方 + 保留 hok 扩展 —— ①键名**只认官方拼写**（下划线；`aria2-*` 连字符），砍掉 hok 别名；②未知键**保持严格**但分流报错（官方独有键 vs 非设置项）；③**删掉 `set`/`unset`/`list`**：写入 = `hok config <key> <value>`、删除 = `hok config rm <key>`、`edit` 保留为 hok 扩展；④输出三档：裸 `config` = 文件里保存的键（`key = value` 行）、`-j/--json` = 同一份 JSON、`-a/--all` = 全键表格；⑤`config -h` **不对标**官方（官方 `-h`=`--help`=153 行手册；hok 保持 clap 短/长，手册走 `--help`）。
**实现**：
- `crates/libscoop/src/config.rs`：`canonical_key` → 唯一 `SETTINGS` 清单（每键一个拼写；serde alias 保留以兼容旧配置文件）；新增 `pub fn value(session, key)`（读**存储值**，不解析默认）；`pub enum OfficialKey` + `map_official_key`（官方 → hok，只认规范拼写）+ `official_name`（hok → 官方，导出用）；`OFFICIAL_ONLY_KEYS` 清单驱动报错分流。
- `crates/libscoop/src/error.rs`：新增 `Error::ConfigKeyUnsupported`（`error.config_key_unsupported`）；`src/lib.rs` 加翻译分支。
- `crates/libscoop/src/package/export.rs`：`build_export(session, ExportSections { config, hok })`，`-c` 段 = `official_name` 过滤后的共享键（不含 hok 独有键与机器相关路径）。
- `src/cmd/config.rs`：`Args { json, all, key, value, command }` + `args_conflicts_with_subcommands`；`Command = Edit | Rm`；裸/`-j`/`-a` 三个视图 + `settings_lines` 对齐输出；配置路径提示改 **stderr**（`output::note`）。
- locales/README 同步（`config_removed` 文案、长帮助写明 hok 扩展与"无别名"、README 用法块与 `config language` 示例修正）。
**顺带修的两个真 bug**：①**S12** `ignore-failures` 默认值显示/帮助/README 的 `true`（运行时 `false`）；②**S3 键名表前提反了**——官方 `set_config` 原样存键、`get_config USE_SQLITE_CACHE` 读下划线名，原 kebab 表让真实 `scoop export -c` 的配置段基本读不进。
**测试**：新增 `tests/exit_codes.rs::config_views_and_usage_errors_exit_cleanly`、`cmd::config` 的 `settings_lines_align_the_value_column`、libscoop `value_reads_back_every_setting`/`boolean_default_column_matches_the_runtime_default`/`config_section_carries_the_shared_settings`；`--test exit_codes|cli_help_i18n|i18n_keys` 全绿；`cargo test --workspace` 仅 2 条 ACL 环境性失败（clean HEAD 同样失败）。
**Why:** 官方 `scoop config` 的四形态是脚本与文档的公共契约（`config <k> <v>` / `rm` / 单键读取），hok 自造 `set`/`unset`/`list` 让按官方文档操作的用户处处踩空；键名别名则让"同一个设置两个名字"，导入导出无法共用一张表。
**How to apply:** 设置键名与拼写统一走 `config::SETTINGS` / `canonical_key`（**勿再加别名**，要加键就加进清单 + `set()` arm + `list_all_rows` + 描述三元）；官方↔hok 键映射只在 `config::{map_official_key, official_name}` 一处，import/export 共用；对官方独有键要给出 `ConfigKeyUnsupported` 而非 `ConfigKeyInvalid`。**遗留**：`aria2-min-split-size` 20M→5M、`proxy` 三态（`none` 存字面值 + 直连、`default`/`currentuser` 不生成非法 URL、hok 无系统代理）尚未做。

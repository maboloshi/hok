# CLI 帮助：全局选项与命令选项分节显示

> 2026-10-05 ｜ 提交 `0b4cd4c3` ｜ worktree 主树（`D:\App\github项目\Scoop\hok`）｜ 分支 `refactor/edition-2024`

## 背景

用户要求把 `hok search --help` 中混合显示的全局选项（`-v`/`-q`、`-L`、`-N`、`--confirm`、`-h`）与命令专属选项（`-e`、`-B`、`-D`）在输出中分开列出。当前 `clap` 通过 `global = true` 将 `Cli` 层选项传播到每个子命令，所有非位置参数都被 `localize_heading_level` 归到同一个 "Options" 标题。

## 改动

- `locales/en.yml` + `zh.yml`：新增 `cli_help.heading.global_options`（"Global Options" / "全局选项"）。
- `src/cmd/mod.rs`：
  - `localize_structural_headings` 在构建前提取顶级 `Cli` 的全局参数 ID（`is_global_set()`）；
  - `localize_heading_level` 按参数来源分配三个标题：位置参数 → `Arguments`、全局参数（含自动生成的 `-h`/`-V`）→ `Global Options`、其余命令专属选项 → `Options`。

结果：每个子命令帮助输出三节清晰分离（见 `search --help`、`install --help` 示例输出，已记录在会话中）。

## 验证

- `cargo check` 无警告；`cargo test`：58 单元测试 + `cli_help_i18n` 3 条（含 `cli_help_is_fully_localized` / `every_command_help_exits_cleanly`）全绿，`exit_codes` / `formatjson_cli` / `missing_checkver_cli` 等均通过。
- 中文/英文 `--lang` 两种语言下帮助渲染均正确分节，无参数错位。

## 记录

- 关于 `-q`/`--quiet`：由 `clap_verbosity_flag::Verbosity`（`#[command(flatten)]` 注入）提供，与 `-v` 配对，属于全局选项，已正确归入新节。
- 提交为独立收尾提交 `docs(agents): log cli-help-global-option-separation`（`0b4cd4c3` 之后），引用本次代码提交 `0b4cd4c3`。

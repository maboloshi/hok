# config 分层：行表与编辑器启动上移 CLI

> 2026-10-11 ｜ 提交 `f1ebad7` ｜ worktree `.worktrees/a1-cli-config-layer` ｜ 分支 `refactor/cli-config-layer`

计划项 A1（来源：`.agents/docs/active/audit/systemic-optimization-audit-2026-09.md` 框架 P1-5）：
`crates/libscoop/src/config.rs` 混入了命令逻辑。**行为零变更**，一个提交。

## 搬了什么 / 留了什么

- **上移到 `src/cmd/config.rs`**：`ConfigCell` / `ConfigRow` / `list_all_rows`（键/值/默认值行表）、
  `fmt_bool` / `fmt_size`、编辑器启动 `edit_config`。判据：`ConfigCell` 只服务这张表的单元格语义
  （本地化占位 vs 原样值），lib 内无第二消费者；启动编辑器不依赖 lib 私有项，本质是 CLI 行为。
- **留在 lib**：`config::list` / `value` / `set`（对配置文件的类型化读写，非呈现），未扩大范围。
- lib 侧只新增 3 个只读路径访问器（`default_root_path` / `default_cache_path` / `default_global_path`，
  转发已有私有 `mod default`）与 `Config::last_update`，`inner` 仍私有。
- 顺带收口：`config::value` 的裸 `serde_json::from_str` 改走 `internal::json::parse_str`（审计 R4 的一个越界点）。

## 官方对照（本机 `../Scoop` v0.6.0）——纠正三处"上游其实没有"

- **没有 `lib/config.ps1`**，官方 config 全在 `libexec/scoop-config.ps1`。
- **不存在 `Get-DefaultConfig`**（全仓 0 命中）：默认值是 `get_config($name,$default)` 的逐调用点形参
  （`lib/core.ps1:118-124`），路径默认是 env→config→fallback 链（`:1375` root / `:1378` global / `:1385` cache）。
  ⇒ hok 表格的默认列改由 CLI 给出，只有 3 个环境解析的路径默认向 lib 要，`aria2-split`=5 /
  `aria2-max-connection-per-server`=5 / `aria2-enabled`=true / `use_git_history`=true 均与官方一致。
- **官方没有列表/表格**（裸 `scoop config` 打印整个 `$scoopConfig`，`:168-169`）⇒ hok 的三列表是已登记扩展，保持。
- **`config edit`**：官方**没有编辑器调用**，`edit` 被当键名读作 `'edit' is not set`（`:178-188`），
  全仓 `*.ps1` 对 `EDITOR` **0 命中**；hok 的 `edit` 是已登记的有意扩展 ⇒ 行为原样保持，
  并删掉了原注释里"mirroring Scoop's `Invoke-Expression "$EDITOR …"`"这一**上游不存在**的引用。
- `aria2-min-split-size` hok 20M vs 官方 5M（`lib/download.ps1:351`）属已登记暂缓项，未动。

## 证据

- **逐字节**：39 个场景（裸 `config` / `--json` / `-j` / `--all` / `-a` / `config list` / `config list --json` /
  各单键 / `set` / `rm` / 非法值与未知键 / 官方独有键 / `--help` / clap conflict / 4 种 `config edit`）
  改动前后 stdout/stderr 原始字节 + exit code **均 30184 字节且逐字节相同**；环境完全隔离（临时 home/root，
  不读真实 `~/scoop`）。注意：**hok 没有 `config list` 子命令**，`hok config list` 是把 `list` 当键。
- `config edit` 的 4 条路径非空跑（直启 / `cmd /c` 回退 / 失败报错 / config `editor` 键压过 `$EDITOR`）。

## 门禁

`clippy -p libscoop --all-targets` 0；`clippy --workspace --all-targets`（重放后树上）0；
`test -p libscoop` 0（866 passed）；`test --workspace` 0；改动文件 `rustfmt --edition 2024 --check` 0。
落地：与 D1 同日落地的 rebase 后重跑门禁、漂移校验 "byte-identical to f1ebad7"。

## 连带修正与登记

- 技能 `.agents/skills/hok-code-organization.md` 的「`config::edit` 保留在库层（供未来子命令复用）」被本次反转——
  那个子命令当时已存在且是唯一调用方，该条已改为按现有调用方判归属。
- 未验证：`config edit` 在 editor 键与 `$EDITOR` 都未设时落到未改动的 `libscoop::os::open_file`（会弹编辑器窗口），
  未做运行时验证；`cmd` 自身 spawn/wait 失败路径在 Windows 不可达。
- 登记（不在本次范围）：`ConfigInner.gh_token` / `virustotal_api_key` 仍是 `pub`、CLI 经 `Deref` 直读；
  审计 R4 的 `export.rs` 两处裸解析未动。

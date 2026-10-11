# S4 调研与落地：`config` 键策略与子命令语义（2026-09-13）

> **来源**：`.agents/docs/active/plan/command-deviation-fix-plan-2026-09.md` S4（+ 计划并轨的 S12、S3 遗留的 `export -c`）。
> **分支**：`fix/config-key-and-subcommand`，worktree `.worktrees/s4-config`，7 个提交线性 ff 进 `refactor/edition-2024`（无 merge commit）。
> **纪律**：[[investigate-first]]（先报告后动手）、[[no-patchwork-fixes]]、[[rust-deterministic-gate]]、[[hok-worktree-parallel]]。

## 1. 实测与证据（动手前）

| 现象 | 证据 |
|---|---|
| `ignore-failures` 默认值自相矛盾 | `53e35f3`（2026-08-13）把 getter 默认改成 `false`（提交说明：*"opposite to upstream Scoop"*），但 `config list --all` 默认列（`config.rs:1122`）、两份 locale 长帮助（`en.yml:564` / `zh.yml:564`）、两份 README 表格仍是 `true` |
| 键白名单只作用于 `set` | `config.rs` 文档注释：读取 Scoop 文件时未建模键**静默丢弃**；`unsupported_keys_rejected` 测试锁定 12 个被拒键 |
| 官方键名拼写 | 官方 `set_config` 只 lowercase、`Add-Member` **原样存键**（`libexec/scoop-config.ps1:112`）；取值点是 `get_config USE_SQLITE_CACHE` / `NO_JUNCTION` / …（`lib/core.ps1`、`lib/install.ps1` 等），规范名是**下划线**，只有 `aria2-*` 是连字符（`scoop-config.ps1:21-153` 用法头即权威清单）。**因此 S3 的 kebab 映射表（`use-sqlite-cache`…）前提反了**，真实 `scoop export -c` 的 `config` 段会被逐条判为 Unsupported |
| `proxy` 消费端 | `internal/network.rs:84-86`（`reqwest::Proxy::all(v)`）、`internal/git.rs:35-44`（补 `http://`）、`checkver.rs:1157-1163`（注入 `HTTPS_PROXY`）⇒ `config set proxy default` / `currentuser` 会生成非法 URL 打挂下载；`proxy none` 删键后恰好等于直连 |
| hok 无系统代理 | `reqwest` 用 `default-features = false`，未开 `system-proxy`（`reqwest-0.12.28/Cargo.toml:101-106`）⇒ 未设 ≠ 官方 `default`（系统代理） |
| 官方四形态 | `libexec/scoop-config.ps1:157-178`：裸 `config` = 文件里的设置；`config <name>` = 值或 `'<name>' is not set`；`config <name> <value>` = 写；`config rm <name>` = 删 |
| 官方 `-h` | `bin/scoop.ps1:43-44` 把子命令后的 `-h`/`--help`/`/?` 统一拦截去 `scoop help <cmd>`，打印的是那段 153 行 usage 手册 —— 即官方 `-h` 与 `--help` **等价** |

## 2. 用户决策（拍板）

| 决策门 | 结论 |
|---|---|
| 未知键策略 | **保持严格白名单**，但区分「官方键但 hok 无实现」与「完全未知键」两类报错 |
| 键名拼写 | **严格按官方拼写**，砍掉 hok 历史别名（`aria2_split`/`no_junctions`/`script_engine`/`ignore_failure`） |
| 子命令 | **不保留 `set` / `unset` / `list`**；写入走官方位置参数，删除走官方 `rm`；`edit` 保留为 hok 扩展 |
| 输出档位（选项 B） | 裸 `config` = 文件实际保存的键，`key = value` 行；`-j/--json` = 同一份的 JSON；`-a/--all` = 全键表格（保留） |
| `config -h` | **不对标**（保持 clap 短/长两档），完整设置手册继续走 `--help` |
| 默认值/语义（R2/R4） | 本轮**不做**：`aria2-min-split-size` 20M→5M、`proxy` 三态语义 —— 登记为暂缓（见 §5 遗留） |
| 范围 | S12（`ignore-failures` 显示）、S3 遗留 `export -c`、上述 config 语义；不含别名/文案翻译 |

## 3. 落地（7 个提交，线性）

| 提交 | 主题 |
|---|---|
| `38ecfa9` | **S12**：`ignore-failures` 默认值显示/帮助/README 改回 `false`；新增守卫测试（新配置下每个 bool 行的默认列必须等于 getter） |
| `f4300aa` | 单键读取 + 裸 `config` + `rm`：`config::value`（读**存储值**而非生效默认）与唯一拼写表；配置路径提示改到 **stderr**，stdout 保持纯 JSON |
| `4cefb81` | **S3 遗留 bug**：官方键名表按规范拼写重建（`config::map_official_key`），修掉 `use_sqlite_cache` 被误判 Unsupported |
| `b7c1680` | 拒绝键分类：新增 `Error::ConfigKeyUnsupported`（官方键无对应）与 `ConfigKeyInvalid`（非设置项）分流 |
| `7987205` | `export -c/--config`：官方键名 `config` 段（与 import 共用同一张表）；`build_export` 改为 `ExportSections` |
| `98e28a1` | **严格键名**：`canonical_key` 收敛为一张 `SETTINGS` 清单（每键唯一拼写），`map_official_key` 同步收紧 |
| `cf1baec` | **CLI 对齐**：裸 `config` = `key = value` 行；`-j` / `-a` 取代 `list`；`set`/`unset`/`list` 移除；长帮助写明扩展与"无别名" |

## 4. 最终 CLI 形态

| 命令 | 说明 |
|---|---|
| `hok config` | 配置文件里实际保存的设置，逐行 `键 = 值`（对齐官方裸 `config` 只看文件） |
| `hok config -j` | 同一份内容的 JSON（hok 扩展形态；stdout 纯 JSON，路径提示走 stderr） |
| `hok config -a` | 全部已知设置 + 当前值 + 生效默认值 + 说明（hok 扩展） |
| `hok config <key>` | 打印该项；未设置 → `'<key>' is not set`（对齐官方） |
| `hok config <key> <value>` | 写入（对齐官方；`-j`/`-a` 与 KEY/VALUE 互斥） |
| `hok config rm <key>` | 删除（对齐官方） |
| `hok config edit` | 打开编辑器（hok 扩展；官方把 `edit` 当键名） |
| `hok config --help` | 完整设置清单（官方 `-h`/`--help` 都打手册，hok 只 `--help` —— 有意偏离） |

**有意偏离（登记）**：① `-h` 短帮助（官方 `-h` = 完整手册）；② `-j`/`-a` 与 `edit` 为 hok 扩展；③ 未建模键读取即丢失（官方原样打印）；④ `~/.config/hok/config.json` 而非 `~/.config/scoop/config.json`。

**暂缓（本轮未做）**：`aria2-min-split-size` 20M→5M；`proxy` 三态（`none` 存字面值 + 直连、`default`/`currentuser` 不再打挂下载、未设≠系统代理）；官方独有 15 键均无实现（`use_external_7zip`/`show_manifest`/`autostash_on_conflict` 为真缺口，其余架构差异或 N/A）。

## 5. 验证（实测）

- 隔离 `USERPROFILE`/`XDG_CONFIG_HOME`/`SCOOP` 下逐条跑：裸 `config`（空配置静默 exit 0）、`-j`（`ConvertFrom-Json` 通过）、`-a`（25 行表格，`ignore-failures` 默认列已为 `false`）、`<key>` 读、`<key> <value>` 写、`rm`、`set`/`unset`/`aria2_split` 报错、`-j -a` 冲突 exit 1。
- `export -c` → `{"config":{…}}` 只含共享键（hok 独有 `jobs`、机器相关 `root_path` 均不出现）；重新 `import` 后键值还原；`hok.config` 扩展段仍走 `--hok`。
- 恢复 `hok import` 官方文件实测：`use_sqlite_cache`/`no_junction`/`PROXY` 写入成功，`root_path` 判机器相关跳过、`scoop_repo` 判无对应跳过。
- `cargo test --workspace`：除 `libscoop::permission::grant_users_write_inherit_*` 两条 **ACL 环境性失败**（主树 clean HEAD 同样失败，`error 5` 需提权）外全绿；clippy 仅 `innospect`（vendored）既有告警。

## 6. 有意后果（需知晓）

- **破坏性**：`hok config set/unset/list`、`list --all`、以及下划线别名（`aria2_split` 等）不再可用；旧脚本需改为 `hok config <key> <value>` / `rm` / `-a`。
- 旧 hok 导出的 Scoopfile 里 `hok.config` 若含下划线别名，`import` 会跳过并报告（`ConfigInner` 的 serde alias 仍保证**配置文件本身**能加载）。
- `private_hosts` 数组值：`export -c` 会导出（官方可读回），但 hok 自己的 `import` 仍报"无法表示"（S3 已登记的既有局限）。

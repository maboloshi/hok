# Hok — Scoop 的 Rust 再实现（社区维护版）

> 本项目是 [chawyehsu/hok](https://github.com/chawyehsu/hok) 的社区维护分支。
> 原作者已暂停开发，本分支独立维护，暂不合并上游。

---

## 关于本分支

原版 hok 是一个优秀的项目，用 Rust 实现了一个高效的 Scoop 替代品。但原作者[已有一段时间未更新](https://github.com/chawyehsu/hok)，而 Scoop 生态仍在演进。

本分支在继承原版所有功能的基础上，做了以下方向的改动：

### 技术选型与工程取舍

依赖策略（轻量化替换清单、已剔除项、当前直接依赖表）、两种 SQLite 构建形状、异步运行时与并发取舍、
native shim 与 i18n 的实现选择、护栏与基准数据，全部收在 [DESIGN-zh.md](./DESIGN-zh.md)（英文版 [DESIGN.md](./DESIGN.md)）。

一句话结论：纯 Rust 优先但有明说的例外（`git2` 优于 `gix`、`.zst` 走 `zstd`），每个**直接依赖**只留一个
major 版本，安全 crate 全面 `unsafe_code = deny`。

### 功能补全

原版 hok 只实现了 Scoop 命令的一部分。本分支逐步补齐：

- **checkver 全套** —— regex / JSONPath / XPath / Script / reverse / replace / GitHub / SourceForge 等版本提取模式 + autoupdate 回写
- **SQLite manifest 缓存** —— `use_sqlite_cache`，兼容 Scoop 格式，并按桶的 git diff 增量更新
- **新命令** —— `depends`、`prefix`、`which`、`checkup`、`alias`、`export`、`import`、`create`、`virustotal`、`shim`
- **`shim -g` 位置自由** —— `-g/--global` 可写在子命令前或后（对齐官方 getopt；`hok shim list -g` 与 `hok shim -g list` 等价，只列全局 shim），`hok shim list *` 列出全部 shim。以 `-` 开头的 shim 参数必须放在 `--` 之后，要传给 shim 的 `-g`/`--global` 同样如此。**破坏性变更**：`hok shim add x cmd.exe -g` 现在创建**全局** shim（旧行为是把 `-g` 当作 shim 参数、创建到 local）
- **native shim** —— 10KB `#![no_std]` 原生 exe shim，嵌入 hok 二进制，零外部依赖
- **i18n 国际化** —— 全部用户消息走自研 `hok-i18n`（0.3.0 起替换 `rust_i18n`），支持中英文切换
- **清单历史索引** —— `hok install app@版本` 可经桶的 git 历史取回历史清单（SQLite 索引 + 增量维护，`use_git_history`）
- **常驻 PowerShell 引擎** —— `script_engine` 复用同一个 `pwsh` 进程执行包脚本，不再每脚本起一个
- **列表表格化** —— `hok list` 表格显示，支持 CJK 对齐、可更新版本提示
- **安装器文件** —— 支持 `installer.file`/`uninstaller.file` manifest 格式
- **`--global/-g`** —— 全局安装支持（install/cleanup/uninstall/hold/unhold）

### 与上游的主要差异

本分支修复了原版 hok 与原版 Scoop 的一批行为问题，明细按版本记录在 [CHANGELOG.md](./CHANGELOG.md)：

- **多包操作不再被单个失败打断**（`ignore_failures`；`cleanup` 本身就失败不中断）；`reset` 现在会执行 `post_install` —— 这是官方 Scoop 的遗留 bug；
- **版本比较重写**：数值/文本混合段与 pre-release 走真实优先级，不再对文本段一律返回 `Equal`；
- **下载可续传**：分片与单流都支持 HTTP Range；native shim 取代 `.cmd` 包装器；
- **快捷方式改用纯 Rust 的 `shortcuts-rs`**，不再依赖 `IShellLinkW` 的 vtable 布局；
- 包 / 仓库 / 候选列表统一为大小写不敏感排序。

### Aria2 配置复用

hok 虽然不使用 aria2c，但**复用了 Scoop 的 aria2 配置项**来控制内置的 HTTP 分片下载行为。二者使用的配置项完全兼容，用户无需额外配置。

| Scoop 配置项 | hok 行为 | 默认值 |
|-------------|---------|--------|
| `aria2-enabled` | 是否启用分片下载 | `true` |
| `aria2-split` | 分片连接数 | `5` |
| `aria2-max-connection-per-server` | 单服务器最大连接数 | `5` |
| `aria2-min-split-size` | 触发分片的最小文件体积 | `20M` |

当 `aria2-enabled` 为 `true`、文件大小超过 `min-split-size`、且分片数 > 1 时，hok 把文件切成多个分片，用 `futures::stream::buffer_unordered` 在 tokio 上并发发起 HTTP `Range` 请求，再按区间顺序合并；失败的分片先重试，重试耗尽后降级为串行重下。这与 aria2c 的 Range 分片逻辑本质上是一致的，只是省去了 aria2c 这个外部进程调用。

```bash
# 配置示例（与原版 Scoop 完全一致）
hok config aria2-enabled true
hok config aria2-split 10
hok config aria2-min-split-size 10M
```

### 国际化（i18n）

hok 0.2.0-beta.1 起支持多语言切换（当时用 `rust_i18n`，0.3.0 起换成自研 `hok-i18n`）：

```bash
# 查看当前语言
hok config language

# 切换为英文
hok config language en
```

目前已内置 `zh`（中文）和 `en`（英文）两种语言。**0.3.0 起改用自研的 `hok-i18n` 运行时**（构建期把 `locales/en.yml` / `locales/zh.yml` 压成查找表），不再依赖 `rust_i18n`；CLI 帮助（`--help`/`-h`）同样本地化，由 `hok-i18n-derive` 自动打补丁生成翻译键。

除 `hok config language` 外，还可以用全局参数 `-L/--lang` 对单次运行强制语言：

```bash
hok -L en install 7zip   # 只让这一次运行说英文
```

### 配置参考（Settings）

hok 的配置文件位于 `~/.config/hok/config.json`（与 Scoop 的 `config.json` 格式兼容）。首次运行时会把受支持的键从 Scoop 配置文件迁移过来，之后只读写 hok 自己的文件。

```bash
hok config                # 配置文件里实际保存的设置（键 = 值）
hok config -j             # 同一份内容的 pretty JSON
hok config -a             # 全部设置 + 当前值 + 生效默认值
hok config <key>          # 打印单个设置项
hok config <key> <value>  # 写入单个设置项
hok config rm <key>       # 删除键（回退到默认值）
hok config edit           # 用编辑器打开配置文件（hok 扩展）
hok config --help         # 完整设置参考（含默认值与取值样式）
```

设置名严格按下面表格的写法，没有别名。`-j` / `-a` 是 hok 扩展（`-a` 是唯一能看到
hok 尚未保存的设置的视图）。

| 键 | 类型 | 默认值 | 说明 |
|----|------|--------|------|
| `aria2-enabled` | bool | `true` | 启用分片下载（hok 内置 HTTP Range 分片下载器） |
| `aria2-split` | int | `5` | 每次分片下载使用的连接数 |
| `aria2-max-connection-per-server` | int | `5` | 每个下载对单个服务器的最大连接数 |
| `aria2-min-split-size` | 大小 | `20M` | 触发分片下载的最小文件大小（如 `5M`、`10M`）；hok 默认 `20M`，官方 Scoop 为 `5M` |
| `cache_path` | 路径 | `$SCOOP_CACHE` 或 `<root>/cache` | 下载缓存目录 |
| `cat_style` | 字符串 | *(空)* | 显示清单时传给 `bat --style` 的组件（需安装 `bat`；`bat` 不接受的值会退回纯 JSON 输出） |
| `default_architecture` | `64bit`\|`32bit`\|`arm64` | 自动检测 | 安装时使用的首选架构 |
| `editor` | 字符串 | *(空)* | `hok config edit` 使用的编辑器（如 `code --wait`）；未设置时回退 `$EDITOR`，再回退系统默认处理程序 |
| `global_path` | 路径 | `$SCOOP_GLOBAL` 或 `%ProgramData%\scoop` | 全局应用安装的根目录 |
| `gh_token` | 字符串 | *(空)* | 用于认证请求的 GitHub API token |
| `ignore_failures` | bool | `false` | 多包操作中单个包失败时继续处理其余包 |
| `ignore_running_processes` | bool | `false` | 应用正在运行时仍继续（仅警告，不中止） |
| `jobs` | int | `4` | 并发下载的包数量 |
| `language` | `auto`\|`en`\|`zh` | `auto` | 消息与帮助文本使用的 CLI 语言 |
| `last_update` | 字符串 | *(空)* | 上次 bucket 更新时间戳（由 hok 写入；可设置，但通常不必手动改） |
| `no_color` | bool | `false` | 禁用彩色输出 |
| `confirm` | bool | `false` | 写盘类操作执行前先询问确认 |
| `no_junction` | bool | `false` | 不使用 `current` 目录联接；shim 直接指向版本目录 |
| `autostash_on_conflict` | bool | `false` | 更新时暂存阻塞仓库 pull 的未提交改动（hok 作用于每次 bucket pull；上游仅 Scoop 自身仓库） |
| `private_hosts` | 数组 | *(空)* | 需要额外认证头的私有主机（仅可编辑文件） |
| `proxy` | 字符串 | `none`（未设置） | 代理：`none` \| `default` \| `currentuser` \| `[username:password@]host:port` |
| `root_path` | 路径 | `$SCOOP` 或 `~/scoop` | Scoop 根目录 |
| `script_engine` | bool | `false` | 复用同一个常驻 `pwsh` 进程执行包脚本 |
| `use_git_history` | bool | `true` | 经桶的 git 历史解析历史清单 |
| `use_isolated_path` | bool\|字符串 | *(空)* | 将应用的 PATH 条目存入专用环境变量（默认为 `SCOOP_PATH`） |
| `use_sqlite_cache` | bool | `false` | 使用 SQLite 数据库缓存清单（兼容 Scoop 的 schema） |
| `virustotal_api_key` | 字符串 | *(空)* | 用于上传/扫描文件的 VirusTotal API key |

本表与 `hok config -a` 对齐（全部可写键 + 生效默认值）。大多数键可通过 `hok config <key> <value>` 修改；`private_hosts` 只能直接编辑配置文件。别名（alias）通过 `hok alias` 命令管理。

---

## 相关文档

- [设计说明（中文）](./DESIGN-zh.md) / [Design（English）](./DESIGN.md) —— 技术选型、依赖策略、构建形状、基准数据
- [架构说明（英文）](./ARCHITECTURE.md) —— 已发布结构：分层、数据流、事件总线、安装管线
- [用 Rust 写的 Scoop 再实现 — Chawye Hsu（原版博文）](./docs/reference/rust-scoop-reimplementation-chawyehsu.zh.md)
- [更新日志](./CHANGELOG.md)
- [命令列表（英文）](./README.md#commands)
- [hok-vs-Scoop 脚本基准](./scripts/benchmark_vs_scoop.ps1)（本机直跑，无需 Windows Sandbox）

## License

**hok** © [Chawye Hsu](https://github.com/chawyehsu) and [contributors](https://github.com/maboloshi/hok/graphs/contributors).
Released under the [Apache-2.0](./LICENSE) license.

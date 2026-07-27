# Hok — Scoop 的 Rust 再实现（社区维护版）

> 本项目是 [chawyehsu/hok](https://github.com/chawyehsu/hok) 的社区维护分支。
> 原作者已暂停开发，本分支独立维护，暂不合并上游。

---

## 关于本分支

原版 hok 是一个优秀的项目，用 Rust 实现了一个高效的 Scoop 替代品。但原作者[已有一段时间未更新](https://github.com/chawyehsu/hok)，而 Scoop 生态仍在演进。

本分支在继承原版所有功能的基础上，做了以下方向的改动：

### 轻量化改造

原版 hok 依赖了不少重型 crate，其中有些是纯 Rust 生态中「太重了但没更好选择」的妥协。本轮改造逐一复核并替换：

| 原依赖 | 替换方案 | 理由 |
|--------|---------|------|
| `chrono` | `jiff`（BurntSushi） | 纯 Rust 轻量时间库，regex 作者出品 |
| `futures` + thread-pool | `std::thread::spawn` | 项目中只用了一次异步，不值得拉整个运行时 |
| `sysinfo` | Win32 FFI（kernel32） | 只用了扫进程一个功能，sysinfo 是整个系统监控 |
| `curl` + static-curl | `ureq`（纯 Rust HTTP） | 去掉了 libcurl 的 C 编译，完全纯 Rust |
| `once_cell` | std（LazyLock/OnceCell） | Rust 1.70+ 已标准化，无需第三方 |
| scoop_hash selfcontained | rustcrypto 后端 | 手写 MD5/SHA1/SHA256/SHA512 ~4700 行是维护债务 |
| `remove_dir_all` | `std::fs::remove_dir_all`（Rust 1.74+） | 原生支持 Windows 长路径，无需第三方 |
| `unarc-rs` | `unrar` + 7z.exe 兜底 | unarc-rs 是万能解压库，拉入旧版 sevenz-rust2 + zip v8，引发整条 crypto 链重复 |
| `thiserror` 1 + 2 并存 | 统一升级到 v2 | 消除依赖树中同一 crate 两个 major 版本的冗余 |
| `md-5`/`sha1`/`sha2` 0.10 + 0.11 并存 | 统一升级到 v0.11 | 同上，消除 RustCrypto digest 双版本 |

### 代码去重

本轮对项目内重复代码做了系统性清理：

| 类型 | 改动 | 效果 |
|------|------|------|
| 跨文件函数提取 | `compute_file_hash`、`encode_wide`、`write_json` 统一到库内 | 删 3 份本地副本，改一处即生效 |
| 宏化模式化方法 | Manifest 9 个访问器、ChecksumBuilder 4 个方法、`is_default_*` 3 个方法 | 47 行重复 → 9 行宏调用 |
| 测试辅助提取 | `tmpdir()` 共享化 | 4 处重复的 tempdir 创建合并 |
| 基准文件合并 | 4 个算法各自的 bench 文件合一 | 229 行 → 109 行 |

### 依赖现状

当前编译依赖约 20 个 crate，全部为功能刚需：

| 分类 | Crate | 说明 |
|------|-------|------|
| **压缩 — 纯 Rust** | `sevenz-rust2`, `zip`, `tar`, `flate2`, `bzip2`, `lzma-rs`, `zstd` | 各对应一种归档/压缩格式 |
| **压缩 — C++ 后端** | `unrar` | RARLab 官方库，RAR 无纯 Rust 替代 |
| **压缩 — 外部进程** | 7z.exe（运行时降级） | 兜底 LZH / ISO 等罕见格式 |
| **Windows 专用** | `junction`, `winreg`, `innospect`, `shortcuts-rs` | Win32 API 的 Rust 封装 |
| **网络** | `ureq` | 比 `reqwest` 轻量，纯 Rust |
| **Git** | `git2` | libgit2 C 绑定（试过 `gix`，依赖树更大） |
| **SQLite 缓存** | `rusqlite` (bundled) | 兼容 Scoop，无纯 Rust SQLite 替代 |
| **序列化** | `serde`, `serde_json`, `json5` | JSON / JSON5 |
| **CLI 框架** | `clap`, `clap_complete`, `clap-verbosity-flag` | — |
| **终端渲染** | `crossterm`, `indicatif` | 终端颜色 + 进度条 |
| **其他** | `anyhow`, `thiserror`, `regex`, `jiff`, `dirs`, `flume`, `rayon`, `tracing` 等 | 标准基础设施 |

**已剔除：** `chrono`, `curl-static`, `futures`, `sysinfo`, `once_cell`, `remove_dir_all`, `unarc-rs`

**重复版本已消除：** `sevenz-rust2`、`zip`、`thiserror`、`md-5`/`sha1`/`sha2` 在依赖树中各只保留一个版本。

### 功能补全

原版 hok 只实现了约一半的 Scoop 命令。本分支逐步补齐：

- **checkver 全套** —— 7 种版本提取模式（regex / JSONPath / XPath / Script / reverse / replace / GitHub / SourceForge）+ autoupdate 回写
- **SQLite manifest 缓存** —— `use_sqlite_cache`，兼容 Scoop 格式
- **新命令** —— `depends`、`prefix`、`which`、`checkup`、`alias`、`export`、`import`、`create`、`virustotal`、`shim`
- **native shim** —— 10KB `#![no_std]` 原生 exe shim，嵌入 hok 二进制，零外部依赖
- **i18n 国际化** —— 全部用户消息迁移到 `rust_i18n`，支持中英文切换
- **可切换输出风格** —— `hok config set output-style pacman`，Scoop/Pacman 两种风格
- **列表表格化** —— `hok list` 表格显示，支持 CJK 对齐、可更新版本提示
- **安装器文件** —— 支持 `installer.file`/`uninstaller.file` manifest 格式
- **`--global/-g`** —— 全局安装支持（install/cleanup/uninstall/hold/unhold）

### 修复的原版 bug

本分支修复了原版 hok（以及原版 Scoop）中的一些问题：

| Bug | 影响 | 修复方式 |
|-----|------|---------|
| **多包操作批量中断** | `install`/`update`/`cleanup` 等操作中，某个包失败会导致整个操作中断 | 实现 `ignore_failures` 配置 + `-f` 参数，失败时打印错误并继续处理剩余包 |
| **reset 不跑 post_install** | `hok reset <app>` 不会执行 manifest 中的 `post_install` 脚本，这其实是 **Scoop 原版的遗留 bug** | reset 命令现在正确执行 `post_install` |
| **版本比较不完整** | `compare_versions()` 对文本段直接返回 `Equal`（如 `1.0.0-beta` vs `1.0.0-alpha`） | 重写比较逻辑，支持数值/文本混合段、pre-release 优先级 |
| **死代码残留** | `get_content_length` 函数未使用，产生 warning | 删除，项目现为 **0 warning** |
| **下载无断点续传** | 分片下载中断后全部重来 | 支持 HTTP Range 续传，已下载的分片跳过，不完整的续传 |
| **缺少 native shim** | 使用 `.cmd` 包装器做 shim，启动慢 | 新增 `hok-shim.exe`：native exe shim，支持 GUI 分离、Job Object、Ctrl+C 传播 |
| **快捷方式依赖于 COM** | 使用 raw IShellLinkW FFI 创建 `.lnk`，vtable 布局不稳定导致部分包失败 | 替换为 `shortcuts-rs` 纯 Rust .lnk 写入器，支持参数和图标 |
| **`update` 无短时保护** | 短期内重复 `hok update` 会重复拉取所有仓库 | 增加 15 分钟 cooldown，支持 `--force` 跳过；仅拉取当前 HEAD 分支 |
| **缓存刷新无反馈** | `update` 最后 SQLite 缓存刷写时终端无响应 | 移到 binary 层并输出 `Refreshing manifest cache...` 提示 |
| **缺少 reinstall** | 需要手动 `uninstall` + `install` | 新增 `reinstall` 命令，自动保持 held 状态 |
| **`SetConsoleCtrlHandler(NULL)` 错误** | NULL 移除所有 handler 导致某些 Windows 版本上 crash | 替换为真实 handler 函数，返回 TRUE 吞掉 Ctrl+C |
| **`ShellExecuteW` 不等待退出码** | elevation 路径不等待子进程、不转发退出码 | 改用 `ShellExecuteExW` + `WaitForSingleObject` |
| **`expand_dp0` 空 args 越界** | 无 args 字段时 `wrapping_sub(5)` 回绕导致 panic | 加 `len < 5` 边界检查 |
| **大小写排序不一致** | 包/仓库/候选列表排序混乱 | 统一 case-insensitive 比较 |

### Aria2 配置复用

hok 虽然不使用 aria2c，但**复用了 Scoop 的 aria2 配置项**来控制内置的 HTTP 分片下载行为。二者使用的配置项完全兼容，用户无需额外配置。

| Scoop 配置项 | hok 行为 | 默认值 |
|-------------|---------|--------|
| `aria2-enabled` | 是否启用分片下载 | `true` |
| `aria2-split` | 分片连接数 | `5` |
| `aria2-max-connection-per-server` | 单服务器最大连接数 | `5` |
| `aria2-min-split-size` | 触发分片的最小文件体积 | `5M` |

当 `aria2-enabled` 为 `true`、文件大小超过 `min-split-size`、且分片数 > 1 时，hok 使用 `std::thread::scope` 启动多个线程，每个线程通过 HTTP `Range` 头并发下载一个分片，最后合并。这与 aria2c 的 Range 分片逻辑本质上是一致的，只是省去了 aria2c 这个外部进程调用。

```bash
# 配置示例（与原版 Scoop 完全一致）
hok config aria2-enabled true
hok config aria2-split 10
hok config aria2-min-split-size 10M
```

### 国际化（i18n）

hok 0.2.0-beta.1 起支持多语言切换：

```bash
# 查看当前语言
hok config LANG

# 切换为英文
hok config set LANG en
```

目前已内置 `zh`（中文）和 `en`（英文）两种语言，全部用户界面消息均已迁移到 `rust_i18n` 框架。CLI 帮助信息（`--help`/`-h`）也支持多语言，通过 `hok-i18n-derive` proc-macro 自动生成翻译键。

### 输出风格切换

```bash
# 切换为 Pacman 风格
hok config set output-style pacman

# 恢复 Scoop 风格
hok config set output-style scoop
```

Scoop 风格（默认）：`  Extracting... done` 逐步进度。
Pacman 风格：`::` 标题前缀、`✓`/`⚠`/`✗` 状态符号，粗体标签 + 普通信息内容。

### Native Shim 基准测试

hok 内嵌了 **10KB** 的 `#![no_std]` 原生 exe shim，完全符合 [Scoop Shim 规范](https://github.com/ScoopInstaller/Shim)。性能对比（whoami.exe, 30 runs）：

| 实现 | 平均耗时 | 开销 | 体积 |
|------|---------|------|------|
| 直接启动 | 29.7 ms | — | — |
| **hok-shim (no_std)** | **51.7 ms** | **+22 ms** | **10 KB** |
| Rust (上游) | 54.9 ms | +25 ms | 121 KB |
| Zig (上游) | 53.9 ms | +24 ms | 71 KB |
| C++ (上游) | 56.9 ms | +27 ms | 158 KB |
| C# (上游) | 107.3 ms | +77 ms | 14 KB |

hok-shim 在所有实现中体积最小（7-16 倍差距），速度与 Rust/Zig/C++ 处于同一量级。关键优化：`AttachConsole` 避免 GUI 子系统创建新控制台（节省约 400ms），`CreateJobObject` 确保子进程树清理。

### 设计原则

- **纯 Rust 优先，但有底线** —— 能不用 C 编译就不用，但 `git2`（libgit2）比 `gix`（20 分钟编译）更务实。「Pure Rust first」有实际边界。
- **不要重复造轮子，但也不当冤大头** —— 标准算法（MD5/SHA）用现成 crate，平台特定 API（COM/Win32）用 raw FFI。后者没有合适的轻量 crate，几十行 FFI 比引入整个 crate 更合理。
- **兼容原版 Scoop** —— SQLite 缓存 schema、config 格式、autoupdate 行为均保持兼容。
- **零 warning 策略** —— 所有代码 0 warning，101 测试全过。

---

## 相关文档

- [用 Rust 写的 Scoop 再实现 — Chawye Hsu（原版博文）](./用%20Rust%20写的%20Scoop%20再实现%20-%20Chawye%20Hsu.md)
- [更新日志](./CHANGELOG.md)
- [命令列表（英文）](./README.md#commands)

## License

**hok** © [Chawye Hsu](https://github.com/chawyehsu) and [contributors](https://github.com/maboloshi/hok/graphs/contributors).
Released under the [Apache-2.0](../LICENSE) license.

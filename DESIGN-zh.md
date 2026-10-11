# 设计说明

> 读者：评估 hok 的使用者，以及想了解「为什么这么做」而非「东西放在哪」的贡献者。

**范围** —— 本文只装 **why**：技术选型、其权衡，以及支撑结论的数字。

以下内容本文**不复述**：

- *东西放在哪*（分层、数据流、管线、构建与发布）—— [`ARCHITECTURE.md`](ARCHITECTURE.md)；
- *每个版本改了什么* —— [`CHANGELOG.md`](CHANGELOG.md)；
- *怎么用* —— [`README.md`](README.md)（[中文](README-zh.md)）。

## 1. 分支与上游

Hok 是 [Scoop](https://scoop.sh/) 的 Rust CLI 实现，fork 自 [chawyehsu/hok](https://github.com/chawyehsu/hok)（原作者已暂停开发）。本分支独立演进：对齐官方 Scoop 0.6.0 的功能补齐、异步运行时、全量本地化，以及一轮工程清理。**不打算合并回上游。**

由此带来两个实际后果：

- **发布是手动的。** `.github/workflows/release.yml`（`workflow_dispatch` + tag 输入）构建 winsqlite3 形状，
  只上传 x64 单个 `hok.exe`（无压缩包、无校验和）；`cd_release_please` 限定在上游仓库、本 fork 永不运行，
  所以版本号与 `CHANGELOG.md` 都手工维护。
- **`CICD` workflow 在本 fork 已被手动禁用**，其徽章不会有运行记录。门禁一律本地跑 —— 见 [`ARCHITECTURE.md`](ARCHITECTURE.md) 的 Building, Testing and Releasing。

## 2. 依赖策略

纯 Rust 优先，但有明确的边界：纯 Rust 方案既存在又性能够用，就用它；不够用时才选 C 后端或系统组件，并且**记录下来**，而不是藏起来。

### 2.1 替换与剔除

| 上游 | 本分支 | 理由 |
|---|---|---|
| `chrono` | `time`（经 `jiff` 过渡） | 纯 Rust 且更轻；本地偏移与格式解析 `time` 都够用 |
| `futures` 执行器 + 线程池 | tokio（`#[tokio::main]`） | checkver、分片下载、bucket 拉取这些扇出负载本来就是异步的，执行器统一到 tokio；`futures` crate 只保留 `StreamExt` 这类流组合 |
| `sysinfo` | Win32 FFI（kernel32） | 只做一次进程扫描，不值得引入整个系统监控 crate |
| `curl` + `static-curl` | `ureq`，最终 **`reqwest`**（native-tls） | 先去掉 libcurl 的 C 编译；异步化重构后客户端改为 `reqwest`。GitHub 与 VirusTotal API 走同一条链路 |
| `once_cell` | std（`LazyLock`/`OnceCell`） | Rust 1.70 起已标准化 |
| `remove_dir_all` | `std::fs::remove_dir_all` | Rust 1.74 起原生支持长路径 |
| `unarc-rs` | `rars`（+ 7z.exe 兜底） | 万能解压库会拉入旧版 `sevenz-rust2` 与 `zip`，把整条 crypto 链复制一份 |
| `thiserror` v1 + v2 | 只用 v2 | 依赖树里同一 crate 只留一个 major 版本 |
| `md-5` / `sha1` / `sha2` 0.10 + 0.11 | 只用 0.11 | RustCrypto digest 只留一代 |
| `rust-i18n` | 自研 `hok-i18n` | 构建期语言表替代运行时解析器（见 §6） |
| `dirs` | std + Win32 FFI | 随 Windows 依赖收敛一并删除 |
| `scoop_hash`（self-contained） | RustCrypto 后端，现为 `libscoop::internal::hash` | 手写约 4700 行哈希是维护债，不是资产 |

### 2.2 已消除的重复版本

`unarc-rs` 被替换后，`sevenz-rust2`（v0.20）与 `zip`（v8）的重复随之消失；`thiserror`、`md-5`、`sha1`、`sha2` 在依赖树中现在各只有一个版本。

### 2.3 当前直接依赖

版本与完整清单以工作区的各个 `Cargo.toml` 为准 —— **下表只说明每组依赖用来做什么，不复制版本号。**

| 分类 | Crate | 说明 |
|---|---|---|
| 压缩 —— 纯 Rust | `sevenz-rust2`, `zip`, `tar`, `flate2`, `bzip2`, `lzma-rust2`, `rars` | 每种格式一个。`flate2` 全工作区固定在纯 Rust 的 `zlib-rs` 后端，`bzip2` 0.6 走 `libbz2-rs-sys` —— 两者都不需要 C 工具链 |
| 压缩 —— C 后端 | `zstd` | 仅用于 `.zst`：纯 Rust 的 `ruzstd` 在该后端定型时实测慢 1.4–3.5 倍（`ruzstd` 不在依赖树中） |
| 压缩 —— 外部进程 | 7z.exe | 罕见格式（LZH、ISO）的运行时兜底 |
| Windows | `junction`, `winreg`, `windows-sys`, `innospect`, `nsis`, `shortcuts-rs` | 目录联接 / 注册表 / Win32 API、Inno Setup 与 NSIS 安装器解析、`.lnk` 写入 |
| 网络（异步） | `reqwest`（native-tls） | 供 `Network` trait、下载与 checkver 使用的异步 HTTP；Windows 上走 schannel，因此沿用系统证书库与代理设置 |
| Git | `git2` | libgit2 绑定；`gix` 评估后未采用（见 §7） |
| SQLite 缓存 | `rusqlite`（`bundled`） | 默认编译 bundled 合并源，另有可选的 `winsqlite3` 形状（见 §4） |
| 序列化 | `serde`, `serde_json`, `json5` | JSON / JSON5 |
| CLI | `clap`, `clap_complete`, `clap-verbosity-flag` | 参数解析与补全 |
| 终端 | `crossterm`, `indicatif` | 颜色与进度 |
| 基础设施 | `anyhow`, `thiserror`, `regex`, `time`, `tokio`, `futures`, `rayon`, `tracing` 等 | `rayon` 仍用于 CPU 密集的桶扫描与清单历史扫描 |

## 3. 解压后端

- **RAR 是纯 Rust，且没有 C++ 兜底。** `.rar` 解压使用 `rars`（覆盖 RAR 1.3/1.4、1.5–4.x、5+，含分卷与 RARVM filter）。仓库里**已没有任何 RAR 相关依赖**（连 dev-dependency 都没有），`cargo test` 不再为 RAR 编译任何 C++。测试比对的是提交进来的 golden 基线（`crates/libscoop/tests/fixtures/rar/expected-trees.txt`）——它历史上由 `unrar` 0.5（UnRAR 6.x 世代）产生，现在由 WinRAR 的 `Rar.exe` 再生（路径走 `HOK_RAR_EXE`），两者在该语料上逐字节一致。覆盖范围与已知差异见 [`crates/libscoop/tests/fixtures/rar/README.md`](crates/libscoop/tests/fixtures/rar/README.md)。
- **仍有一种格式需要 C**：`.zst` 走 `zstd` crate（原因见 §2.3）。
- 7z、zip、tar、gz、bz2、xz 均为纯 Rust；LZH / ISO 在运行时降级到 7z.exe。
- Inno Setup 与 NSIS 安装器由注册表上的 `innospect` 与 `nsis` 解析；上游 PR 落地后，vendored fork 已退役。

## 4. SQLite：两种构建形状

清单缓存使用 `rusqlite`。默认 `cargo build` 编译 bundled 合并源，x86/x64/arm64 通吃；发布形状改为链接 Windows 自带的 `winsqlite3.dll`（约小 1 MB）：

```powershell
pwsh scripts/build-hok.ps1                            # bundled（默认，显式写出）
pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify   # 发布实际使用的形状
```

- 脚本从 Windows SDK 取出 `winsqlite3.h/.lib` staging 到 gitignored 的 `.winsqlite3/`，并**只在该进程内**设置 `LIBSQLITE3_SYS_USE_PKG_CONFIG` / `SQLITE3_LIB_DIR` / `SQLITE3_INCLUDE_DIR` —— 不写用户环境变量、不写 `.cargo/config.toml`，因此之后的 `cargo build` 仍是 bundled。
- **x86 不支持 winsqlite3**：SDK 的 import lib 在 x86 上导出 `__stdcall` 修饰名（`_sqlite3_column_name@8`），与 rusqlite 的 cdecl 声明不匹配；i686 保持 bundled。
- `crates/libscoop/build.rs` 对两种静默失败直接报错，而不是构建出错形状：只设了 `SQLITE3_LIB_DIR` 却忘设 `LIBSQLITE3_SYS_USE_PKG_CONFIG`，以及 x86 + linked 模式。

## 5. 异步运行时与并发

CLI 是全异步的：`#[tokio::main]`，库内每条触碰 I/O、网络或子进程的路径都是 `async fn`。这套运行时的**形状**（哪个模块负责什么、取消与背压如何工作）在 [`ARCHITECTURE.md`](ARCHITECTURE.md) 的 Async Runtime 一节；这里说明理由：

- **为什么异步** —— 有三类负载天然扇出：跨清单的 checkver、分片下载（并发的 HTTP `Range` 分片）、并发 bucket 拉取。线程池要为每个在途请求付出一个线程，tokio 上的 `buffer_unordered` 不需要。
- **为什么是 `reqwest` + `native-tls` 而不是 rustls** —— Windows 上 `native-tls` 就是 schannel：系统证书库与代理配置，也就是官方 Scoop 从 PowerShell 继承到的那一套。它同时把 `ring`/`rustls`/`webpki-roots`（以及它们钉住的 `getrandom 0.2`）挡在依赖树外。hok 只面向 Windows，不需要非 Windows 的 TLS 后端。
- **为什么阻塞活儿不占 async 线程** —— git2、解压、脚本执行与交互提示都在 trait 边界用 `tokio::task::spawn_blocking` 下放，调用点因此保持 `async fn`。
- **为什么 CPU 密集的扫描仍用 rayon** —— 桶扫描与清单历史扫描是 CPU 密集型，异步运行时在那里没有收益，继续用 rayon 线程池。

## 6. Shim 与 i18n 的设计

**native shim。** `hok-shim.exe` 取代生成的 `.cmd` 包装器：`#![no_std]`、固定缓冲区、Win32 FFI，嵌入 hok 二进制，并以两个**只差 PE 子系统**的二进制构建——`hok-shim`（console）与 `hok-shim-gui`（GUI，不弹控制台窗口，shell 不等待）。

| 实现 | 平均耗时 | 开销 | 体积 |
|---|---|---|---|
| 直接启动 | 29.7 ms | — | — |
| **hok-shim (no_std)** | **51.7 ms** | **+22 ms** | **10 KB** |
| Rust (上游) | 54.9 ms | +25 ms | 121 KB |
| Zig (上游) | 53.9 ms | +24 ms | 71 KB |
| C++ (上游) | 56.9 ms | +27 ms | 158 KB |
| C# (上游) | 107.3 ms | +77 ms | 14 KB |

（whoami.exe，30 次。本分支的 shim 体积最小，7–16 倍差距，速度与原生实现同一量级。关键选择：`AttachConsole` 省掉 GUI 子系统新建控制台约 400 ms 的开销，job object 保证子进程树被清理。）

**i18n。** 所有用户可见文案都走自研的 `hok-i18n` 运行时（0.3.0 起替换 `rust-i18n`）。`crates/hok-i18n/build.rs` 在构建期把 `locales/en.yml` / `locales/zh.yml` 压成按 key 排序的查找表，因此发布二进制里不带任何 YAML 解析器；`hok-i18n-derive` 给 clap 的帮助树打补丁，`--help` 同样本地化。用法（`hok config language`、`-L/--lang`）见 [`README-zh.md`](README-zh.md)。

## 7. 有意保留的边界

- **用 `git2` 而不是 `gix`。** 纯 Rust 的 Git 本应是策略首选，但 `gix` 在这里要为 hok 实际用到的四个操作（clone / pull / ls-remote / status）付出约 20 分钟构建时间。务实的一方胜出——边界明说，而不是用开关藏起来。
- **不重复造轮子，也不当冤大头。** 标准算法（MD5/SHA）用现成 crate；没有合适轻量 crate 的平台 API 就保留几十行 raw FFI。
- **兼容官方 Scoop 不可让步。** SQLite 缓存 schema、`config.json` 格式、autoupdate 行为一律对齐官方。
- **`aria2-min-split-size` 默认 `20M`**，官方 Scoop 是 `5M`（`lib/download.ps1`）。对齐它是已登记的暂缓项
  （hok 用自研下载器而非 aria2c），不是未记录的漂移。

## 8. 工程护栏

- **`unsafe_code = deny`**（crate 级）；仅 `no_std` 的 Win32 shim 与少量 FFI 模块（`libscoop::internal::os`、`libscoop::permission`）豁免。每处 `unsafe` 都在 [`.agents/docs/active/audit/unsafe-audit.md`](.agents/docs/active/audit/unsafe-audit.md) 中审计并分类。
- **零 warning 策略** —— 全树编译无警告，`cargo test --workspace` 全绿。
- **criterion 基准护栏** —— `cargo bench` 覆盖关键纯函数（版本比较、glob、哈希、清单解析、URL 工具）的 `pure_functions` 目标，另有事件通道的 `event_emission` 目标；重构造成回归会立刻暴露。

## 9. 性能

Hok（以及作为后端的 libscoop）目标是比官方 Scoop 更快、同时更完整。以下数字在作者机器的 Windows Sandbox 中采集（AMD Ryzen 5 2600、32 GB、Windows 10），版本为 hok 0.1.0-beta.6 —— 属历史数据，重跑脚本即可刷新：

```sh
# Benchmarking scoop bucket list
Benchmark 1: scoop bucket list
  Time (mean ± σ):      5.610 s ±  0.627 s    [User: 6.573 s, System: 3.520 s]
  Range (min … max):    4.784 s …  7.063 s    10 runs

Benchmark 2: hok bucket list
  Time (mean ± σ):     159.4 ms ±  28.3 ms    [User: 86.4 ms, System: 175.2 ms]
  Range (min … max):   140.0 ms … 252.1 ms    18 runs

Summary
  hok bucket list ran
   35.19 ± 7.38 times faster than scoop bucket list
```

复跑方式：

- [`scripts/benchmark/`](scripts/benchmark/README.md) —— 最初的 Windows Sandbox 基准套件。
- [`scripts/benchmark_shims.py`](scripts/benchmark_shims.py) —— §6 那张 shim 表的来源（`whoami.exe`，30 次）。
- [`scripts/benchmark_vs_scoop.ps1`](scripts/benchmark_vs_scoop.ps1) —— 本机直跑的护栏基准：在同一批只读命令上比对退出码与规范化输出，并给出耗时比。Scoop 的网络/有状态命令需加 `-IncludeNetwork` 才跑，其余在离线时跳过。
- `cargo bench` —— §8 的微基准。

# 整体框架分析总结（人话复述）

Status: superseded-by [systemic-optimization-audit-2026-09.md](systemic-optimization-audit-2026-09.md)
Updated: 2026-09-26

> 当前仅分析，无任何代码修改执行（F1–F5 均未动）。本文件为本次会话的框架级信息收集结果的归档总结，不是开发日志（无落地提交 sha，故不写 devlog 条目），也不是持续修订的规格文档（分析完成后可被后续执行取代，故放 audit 而非 spec）。

## 1. 五层架构（现状，无改动）

| 层 | 位置 | 关键文件 | 当前状态 |
|---|---|---|---|
| CLI 层 | `hok/` | `src/bin/hok.rs` → `create_app()` → `cmd::start()` | 薄层：解析参数、初始化 `Session`、分发命令 |
| 门面库 | `crates/libscoop/` | `session.rs`、`event.rs`、`internal/network.rs` | 核心：`Session` 双入口（构建脚本 + `build.rs` 未统一）、`EventBus` 全双工、`Network` trait 注入 |
| 领域服务 | `crates/libscoop/src/package/` | `download.rs`、`checkver.rs`、`sync_install.rs`、`manifest_cache.rs` | `download_fragmented` 与主流重复 `.buffer_unordered()` 模式；任务模型（`PendingItem`/`PackageCache`）无共享抽象 |
| 核心纯类型 | `crates/libscoop/src/package/` 底层 | `Package`、`Manifest`、`Version` | 纯数据：零 I/O、零 async、可确定性单测 |
| 工具层 | `crates/libscoop/src/internal/` | `network`、`archive`、`git`、`hash`、`json`、`os` | `Network` trait（`reqwest` + `native-tls` = schannel）、`Archive` 纯 Rust（无 C++ 工具链）、`Git`（`git2` = `libgit2` C 绑定，无法替代系统 DLL） |

双入口（构建 + 运行）均存在：构建脚本 `scripts/build-hok.ps1` 与 `build.rs` 各自检查 `winsqlite3` 条件（`x86` 限制、环境变量完整性）；`Session` 同时管理 `RwLock` 配置、`OnceLock` 基础设施注入、异步事件总线、取消传播。

## 2. 框架级优化点（F1–F5，全部仅评估未执行）

| 标记 | 主题 | 可做性 | 当前状态 |
|---|---|---|---|
| **F1** | 构建脚本与 `build.rs` 双入口统一（删除脚本重复断言，依赖 `build.rs` `panic!` 作为唯一失败点） | 可做，低风险 | **未改**（`build-hok.ps1` 线 286-304 仍存在，`build.rs` 线 88-117 仍独立） |
| **F2** | 并发框架提取 + `futures` 移除（11 处 `.buffer_unordered()` → `tokio` 原生，删 `Cargo.toml:19`） | 可做，中风险（需行为等价） | **未改**（`futures` 仍在代码与依赖中；审计文档未将其列为 P1 可删项） |
| **F3** | 任务批处理泛化（`PendingItem`/`CheckverFetch` 与 `PackageCache`/`PkgOutcome` 统一为 `TaskBatch`） | 高抽象风险，收益不确定 | **未做**（无新抽象引入） |
| **F4** | 事件/进度框架重构（`ProgressThrottle` 提升为通用速率限制、统一进度闭包签名、优化 `EventBus` 交互） | 可做，中收益 | **未改** |
| **F5** | 构建嵌入框架统一（`assert_sqlite_link_mode`/`embed_hok_shim`/`embed_known_buckets` 提取共享模块） | 低收益，非优先 | **未做** |

## 3. 已固化优化（本次分析未动，仅确认状态）

- SQLite 形状：`bundled`（≈8.12 MB）vs `winsqlite3`（≈7.08 MB），仅 x64/arm64；`x86` 不可（`__stdcall`/`cdecl` 冲突，`build.rs` 断言已封装）；构建脚本处理 `.winsqlite3/` staging、仅进程级环境变量，不污染用户环境。
- 纯 Rust 替代已完成：`unarc-rs` → `rars`（`tests/fixtures/rar/` 无 C++ 编译依赖）；`chrono` → `time`；`curl-static` → `reqwest`（`native-tls`）；`sysinfo` → Win32 FFI；`once_cell` → `std::LazyLock`；`thiserror` 1+2 → v2；`md-5`/`sha1`/`sha2` 0.10+0.11 → v0.11。
- 无系统 DLL 替代：`zstd`（无标准 Windows `zstd.dll`，`ruzstd` 慢 1.4-3.5×）；`git2`（无 `libgit2.dll` 公开接口，`gix` 编译 ≈20 分钟）；压缩格式（`zip`/`tar`/`7z`/`bzip2`/`lzma`）全部纯 Rust，无可链系统库。
- 构建 profile 已最优：`opt-level=z`、`lto=fat`、`panic=abort`、`strip=symbols`、`codegen-units=1`（`Cargo.toml` 线 79-91）；`hok-shim` 同配置（`no_std`，10 KB）。
- 依赖重复已消除（审计 §3.4）：`sevenz-rust2`/`zip`/`flate2` 统一 `zlib-rs` 无 `miniz_oxide`；`thiserror`/`md-5`/`sha1`/`sha2` 各仅保留一个版本。
- `futures` 依赖：`Cargo.toml:19` `futures = "0.3"` 仍存在，代码 11 处使用（`checkver`、`download`、`download_fragmented`、`network`）；审计 P1 未将其列为可直接删除项（需评估替换为 `tokio` 原生并发的行为等价性）。
- `rust-i18n` → 自研 `hok-i18n` 已落地（审计 §9，提交 `c99f07e`/`cb34afb`/`84fe376`/`c0dd4a1`）；发布图 265 → 240（`-e normal`），构建依赖新增 `yaml-rust2`（纯 Rust，无 unsafe）。

## 4. 不可优化（已确认无收益或风险高）

- `build.rs` 三嵌入函数合并：`assert_sqlite_link_mode`（断言）/`embed_hok_shim`（复制二进制）/`embed_known_buckets`（生成 `.rs`）语义完全不同，无共享抽象收益。
- `checkver` 与 `download` 任务模型统一：`PendingItem`/`CheckverFetch`（URL 抓取 + 脚本）与 `PackageCache`/`PkgOutcome`（文件下载 + 验证）任务类型差异大，强制抽象增加间接层（违背“纯 Rust 优先但有底线”设计原则，`README-zh.md` 线 218-220）。
- `ProgressThrottle` 与 `PackageDownloadProgressContext` 分离：频率控制与数据传递职责正确，不应合并。
- `zstd` 保持现状：无系统 `zstd.dll` 替代，`ruzstd` 不可用（慢 1.4-3.5× + 仅解第一帧，静默截断实测）。
- `git2` 无替代：`gix` 编译代价高（≈20 分钟），无 `libgit2.dll` 可直接链。

## 5. 当前未执行任何修改

本报告仅为归档总结。`F1`–`F5` 均未执行：无 `edit`/`write`/`pwsh` 修改操作；`futures` 仍在 `Cargo.toml` 和代码中；`build-hok.ps1` 未简化；`EventBus`/`ProgressThrottle` 未重构；无新抽象引入。后续若执行框架优化，应逐项确认（如先做 `F1` 构建脚本去重，再评估 `F2` `futures` 移除），并按 `.agents/docs/AGENTS.md` R7 写独立 `devlog` 提交引用代码 sha。

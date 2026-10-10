---
name: hok-code-organization
title: Hok 代码组织规范
description: 往 hok 加代码、做重构/去重、或新增目录与共享函数之前读：五层单向依赖、目录布局与 *_dir 语义、跨管道共享函数与坑、去重门槛的本仓判例（P2-6）与验收清单。
metadata:
  type: project
---

Hok 代码组织规范。**核心原则：命名即文档，位置即职责；不为重构而重构、不为去重而去重。**

**Why:** 用户明确：重构/去重前先问「解决什么真实问题」（行为不一致/漂移风险/正确性 bug/维护成本），答不上就不做；行数、重复数量不是目标，**可漂移的重复**才值得合并；度量用「问题是否解决」而非行数；先读官方 Scoop 源码定行为锚（[[local-scoop-source-path]]）；「顺带」改动单独评估独立价值。

**去重/重构量化门槛**：**本体见全局 skill `proactive-review` §7**（本文件不复述，只留本仓判例）。
**判例（P2-6）**是给 3 个从未漂移的 flag（`--offline`/`--no-hash-check`/`--assume-yes`）做 clap `flatten`，
`--help` 逐字节一致，但净 **+54 行**、15 处 flatten + 14 处 `.enabled()`，而重复最重的 `--global`
（9 处/6 种帮助文案）**无法**合并 ⇒ 已回退。
例外：`[workspace.dependencies]` 属依赖声明单点化，收益是版本/feature 防漂移（`flate2` 丢 `zlib-rs` 会把 `miniz_oxide` 拉回依赖树），代价是读者需跳转根 manifest——这类「防漂移」收益成立才做。

**How to apply:** 添加代码前走归属决策；lib.rs/mod.rs 用分区注释（`// ===` 或 `// ───`，全文一致）；mod.rs 只放子模块声明+核心类型。

## 五层架构（单向依赖，禁止反向）
```
CLI (hok crate) → 门面 (libscoop/src/lib.rs) → 领域服务 (libscoop/src/*.rs) → 核心业务 (package/) → 工具 (internal/)
```
- **CLI 层**：参数解析/事件循环/输出渲染/命令调度；禁业务逻辑与文件系统操作（除输出）
- **门面层**：仅 pub mod/pub use，禁函数实现
- **领域服务**：Session/Config/Bucket/Cache/Shim/Shortcut/Persist/Env/Psmodule/Event/Network，pub
- **核心业务**：package/（Manifest/查询/依赖/下载/安装卸载管道），pub(crate) 经 lib.rs 选择性暴露；可依赖 internal+领域服务
- **工具层**：internal/（**18** 个内容文件：arch/archive/**base64**/dag/env/fs/git/github/hash/json/network/os/path/pe/string/time/url/version），pub(crate)；**禁依赖 package/ 或领域服务类型**；glob_to_regex 在 string.rs（无 pattern.rs）；`strip_bom` 在 string.rs，JSON 解析单入口在 json.rs（`parse_str`/`parse_str_or_none`）

## 已接受的架构决策
- **internal/ 已收口 pub(crate)**：lib.rs 仅 `pub use` 转发 `Arch`、`compare_versions` 及薄门面 fs/os/string/url/hash；CLI 不再直用 `libscoop::internal::*`（--arch 走 `Session::set_default_architecture`）
- **archive→event 是工具层唯一例外**：`internal/archive.rs` 依赖 `crate::event::Event` 发提取进度——功能合理（进度上报），保留，勿扩增同类依赖
- **领域服务↔package 双向依赖（无环）已接受**：shim/shortcut/persist/psmodule/env 用 `package::Package` 数据类型；package/operations 调服务入口函数（shim::add 等）。单向表述是理想，实际无环即可
- **config 的呈现逻辑归 CLI，类型化读写归库**：lib 只留 `list`/`value`/`set`（对配置文件的类型化读写）；行表（键/值/默认值）与编辑器启动属命令逻辑，放 `src/cmd/config.rs`。**教训**：曾以"供未来子命令复用"为由把 `edit` 留在库层，但那个子命令当时已存在且是唯一调用方——**没有第二个调用方的"将来复用"不是理由**，按现有调用方判归属
- **checkver 用 `CheckverOptions`（非 clap）**：CLI 持有 clap Args，构造 options 有编译期双向保护，消除镜像漂移


## 当前目录结构
```
src/                     # bin crate（仓库根）：bin/hok.rs lib.rs eventloop.rs output.rs cui.rs format.rs i18n.rs scoop_handler.rs + cmd/（__cmd_reg__.rs 自动生成）
crates/libscoop/src/
├── lib.rs               # 门面：pub mod + pub use + 薄门面（fs/os/string/url/hash）
├── session/config/bucket/cache/network/event/handler/shim/shortcut/persist/env/psmodule/constant/error
├── internal/            # 18 个内容文件（arch/archive/base64/dag/env/fs/git/github/hash/json/network/os/path/pe/string/time/url/version）
└── package/             # mod.rs(核心类型) identity/manifest*/query/resolve/download/sync*/checkver*/checkhashes/checkurls/… + operations/（extract/link/psengine/script）
```

## 目录布局（*_dir 语义）
**官方 Scoop 三族语义**（本地官方源码核对，路径见 [[local-scoop-source-path]]）：
- `appsdir`/`shimdir`/`persistdir` 随 global（core.ps1:349/350/364）→ **effective 族**
- `buckets` 固定用户级（buckets.ps1:1）→ **用户级族**
- `cache`/`scoop.db` 固定用户级（core.ps1:1301/database.ps1:63）

**Session 布局方法**（`session.rs:350-510`）：`effective_root_path/apps_dir/app_dir/app_current_dir/app_current_dir_in/current_dir_name/versioned_dir/shims_dir/modules_dir/persist_dir/persist_root`（effective 族；`current_dir_name` 是运行时入口目录名，见 [[hok-no-junction-implementation]]）+ `buckets_dir/bucket_dir`（用户级族，内部 `config().root_path()`）+ `workspace_dir`。`internal/path.rs` 现有 6 个路径工具：`bucket_dir`（唯一布局函数，根参数化——并行扫描传 root + `config().root_path()`）+ `leaf/leaf_base/is_version_dir/normalize_path/relative_to`（`is_version_dir` 是 `pub(crate)`；另有 `home_dir/config_dir`）。⚠️ **`Session` 自 RwLock/AtomicBool/OnceLock 重构后已是 `Sync + Send`**（编译期测试 `session.rs:706`）；所以并行扫描把 root 传进闭包是数据传递选择，**不是** Sync 约束（`internal/path.rs` 与 `package/query.rs` 的注释均按此表述，别再写成「不是 Sync」）。**曾错过的根**：persist 用错 effective 族（全局安装的 persist 写错位置）、script buckets 用错用户级族——新代码按上面的族归属对一遍。**Working_dir 陷阱**：env.rs 按 `no_junction` 切 `"current"`/version。

**How to apply:** 新路径代码优先 `session.xxx_dir()`；并行扫描（rayon）用 `internal::path::bucket_dir(root, name)` + `config().root_path()`；不要新增裸 `join("apps")`。

## 跨管道共享函数与坑
- `operations::run_hook`（script.rs）：installer/uninstaller file+script 执行块（install/remove 共用）
- `Package::matches_bucket_query`：bucket/name 精确匹配（查询循环/依赖解析共用）
- `manifest_source::resolve_isolated_query`：@version/URL/本地分派（install/download 共用）
- `download::download_and_verify`：**必须先对传入 set 调 calculate_download_size**（否则分片+进度条静默失效）
- `download_verify::verify_downloads`（download_verify.rs:114，仅由 `download_and_verify` 内部调用）：nightly 跳过/空 hash warn/mismatch 删缓存
- **动态分块（package/download_fragmented.rs）**：方案 A 失败区间兜底（chunk 重试耗尽 → `Arc<Mutex<Vec<(u64,u64)>>>` 收集失败 range → `merge_adjacent_ranges` 合并 → 单连接串行补下，复用 `download_range`）；进度事件 `Event::PackageDownloadProgress`（`download_range`/`try_download_range` 加 `on_progress` 回调，32KB read 循环每字节报一次）；part 跟踪 `Vec<(PathBuf, u64, u64)>` 排序校验无空洞；**开放项**：方案 B 跨文件失败率反馈（`PackageSet` 级 effective_chunk_count，文件失败减半下限 2，归因弱优先级低）；`load_cache` 硬编码 remote_size=0 致 cache 命中弱；`calculate_download_size` 双重 HEAD；cookie 格式化三处重复；`update_valid_state` 冗余
- **outdated 文案归 update 命令层**（事件循环 PackageSyncDone 只 break）；should_offline 判 `total==0 && !estimated`；ignore_failures 默认 false；query_installed 已 pub

## 验证清单（重构验收）
- [ ] cargo build + cargo test 全绿；无 util.rs/helpers.rs
- [ ] internal/ 不依赖 package/ 或根目录领域服务
- [ ] run_script/expand_installer_vars/run_hook 在 operations/script.rs
- [ ] download_and_verify 先算 size；outdated 在 update 命令层；should_offline 判 estimated
- [ ] 分区注释一致；零 warning

## 测试基础设施坑
`package/download_fragmented.rs` 的 spawn_range_server（:396）：accept 后必须 `set_nonblocking(false)`（非阻塞继承导致并行偶发失败）；已支持 HEAD（响应 Content-Length、不计入 ranges）。

---
name: hok-code-organization
title: Hok 代码组织规范
description: hok 代码组织规范：五层架构单向依赖、目录布局、共享函数与坑、去重门槛与验收清单。
metadata:
  type: project
---

Hok 代码组织规范（2026-08 重构已落地，合并自 purpose/layering/target-structure/guide-checklist/dir-layout-inventory/dynamic-chunking 六条）。**核心原则：命名即文档，位置即职责；不为重构而重构、不为去重而去重。**

**Why:** 用户 2026-08 明确：重构/去重前先问「解决什么真实问题」（行为不一致/漂移风险/正确性 bug/维护成本），答不上就不做；行数、重复数量不是目标，**可漂移的重复**才值得合并；度量用「问题是否解决」而非行数；先读官方 Scoop 源码定行为锚（[[local-scoop-source-path]]）；「顺带」改动单独评估独立价值。

**去重/重构量化门槛（2026-09-10 补，P2-6 否决案例）：** 去重类改动提交前必须量化四项——①净行数（`git diff --numstat`）②新增间接层（`.enabled()`、`args.x.y` 之类）③是否引入第 N 套写法（与既有模式并存 = 一致性下降）④覆盖范围（只能合并「可合并子集」⇒ 剩下的大头仍重复，反而更不统一）。**净增代码或净增间接层、且真实问题无证据（无漂移、无 bug）→ 直接否决，不提交。** 案例：给 3 个从未漂移的 flag（`--offline`/`--no-hash-check`/`--assume-yes`）做 clap `flatten`，`--help` 逐字节一致，但净 **+54 行**、15 处 flatten + 14 处 `.enabled()`，而重复最重的 `--global`（9 处/6 种帮助文案）**无法**合并 ⇒ 已回退（见 `.agents/docs/active/audit/optimization-audit-2026-09.md` §8）。例外：`[workspace.dependencies]` 属依赖声明单点化，收益是版本/feature 防漂移（`flate2` 丢 `zlib-rs` 会把 `miniz_oxide` 拉回依赖树），代价是读者需跳转根 manifest——这类「防漂移」收益成立才做。

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

## 已接受的架构决策（2026-08 复核落地）
- **internal/ 已收口 pub(crate)**（2026-08 框架修复）：lib.rs 仅 `pub use` 转发 `Arch`、`compare_versions` 及薄门面 fs/os/string；CLI 不再直用 `libscoop::internal::*`（--arch 走 `Session::set_default_architecture`）
- **archive→event 是工具层唯一例外**：`internal/archive.rs` 依赖 `crate::event::Event` 发提取进度——功能合理（进度上报），保留，勿扩增同类依赖
- **领域服务↔package 双向依赖（无环）已接受**：shim/shortcut/persist/psmodule/env 用 `package::Package` 数据类型；package/operations 调服务入口函数（shim::add 等）。单向表述是理想，实际无环即可
- **config::edit 保留在库层**（供未来 `config edit` 子命令复用）；cmd/config.rs 只做调用方
- **checkver 用 `CheckverOptions`（非 clap）**：CLI 持有 clap Args，构造 options 有编译期双向保护，消除镜像漂移


## 当前目录结构（已验证）
```
src/                     # bin crate（仓库根）：bin/hok.rs lib.rs eventloop.rs output.rs cui.rs format.rs i18n.rs scoop_handler.rs + cmd/（__cmd_reg__.rs 自动生成）
crates/libscoop/src/
├── lib.rs               # 门面：pub mod + pub use + 薄门面（fs/os/string）
├── session/config/bucket/cache/network/event/handler/shim/shortcut/persist/env/psmodule/constant/error
├── internal/            # 18 个内容文件（arch/archive/base64/dag/env/fs/git/github/hash/json/network/os/path/pe/string/time/url/version）
└── package/             # mod.rs(核心类型) identity/manifest*/query/resolve/download/sync*/checkver*/checkhashes/checkurls/… + operations/（extract/link/persist/shortcut/script）
```

## 目录布局（*_dir，官方语义已核对）
**官方 Scoop 三族语义**（本地 `../Scoop\lib` 验证）：
- `appsdir`/`shimdir`/`persistdir` 随 global（core.ps1:349/350/364）→ **effective 族**
- `buckets` 固定用户级（buckets.ps1:1）→ **用户级族**
- `cache`/`scoop.db` 固定用户级（core.ps1:1301/database.ps1:63）

**现状（2026-08 归集 + 复核）**：Session 布局方法 12 个：`effective_root_path/apps_dir/app_dir/current_dir/current_dir_name/versioned_dir/shims_dir/modules_dir/persist_dir`（effective 族）+ `buckets_dir/bucket_dir`（用户级族，内部 `config().root_path()`）+ `workspace_dir`。`internal/path.rs` 现有 6 个函数：`bucket_dir`（唯一布局函数，rayon 并行场景用——**Session 含 RefCell/Cell/OnceCell 不是 Sync，不能进并行闭包**，只能传 root 参数）+ 通用小工具 `leaf/leaf_base/is_version_dir/normalize_path/relative_to`。修过的 bug：persist.rs 用户级→effective（全局安装 persist 曾写错根）、script.rs buckets effective→用户级。**Working_dir 陷阱**：env.rs 按 `no_junction` 切 `"current"`/version。

**How to apply:** 新路径代码优先 `session.xxx_dir()`；并行扫描（rayon）用 `internal::path::bucket_dir(root, name)` + `config().root_path()`；不要新增裸 `join("apps")`。

## 跨管道共享函数与坑（2026-08 多轮）
- `operations::run_hook`（script.rs）：installer/uninstaller file+script 执行块（install/remove 共用）
- `Package::matches_bucket_query`：bucket/name 精确匹配（查询循环/依赖解析共用）
- `manifest_source::resolve_isolated_query`：@version/URL/本地分派（install/download 共用）
- `download::download_and_verify`：**必须先对传入 set 调 calculate_download_size**（否则分片+进度条静默失效）
- `download::verify_downloads`：nightly 跳过/空 hash warn/mismatch 删缓存（install/download 共用）
- **动态分块（download.rs，已落地）**：方案 A 失败区间兜底（chunk 重试耗尽 → `Arc<Mutex<Vec<(u64,u64)>>>` 收集失败 range → `merge_adjacent_ranges` 合并 → 单连接串行补下，复用 `download_range`）；进度事件 `Event::PackageDownloadProgress`（`download_range`/`try_download_range` 加 `on_progress` 回调，32KB read 循环每字节报一次）；part 跟踪 `Vec<(PathBuf, u64, u64)>` 排序校验无空洞；**开放项**：方案 B 跨文件失败率反馈（`PackageSet` 级 effective_chunk_count，文件失败减半下限 2，归因弱优先级低）；`load_cache` 硬编码 remote_size=0 致 cache 命中弱；`calculate_download_size` 双重 HEAD；cookie 格式化三处重复；`update_valid_state` 冗余
- **outdated 文案归 update 命令层**（事件循环 PackageSyncDone 只 break）；should_offline 判 `total==0 && !estimated`；ignore_failures 默认 false；query_installed 已 pub

## 验证清单（重构验收）
- [ ] cargo build + cargo test 全绿；无 util.rs/helpers.rs
- [ ] internal/ 不依赖 package/ 或根目录领域服务
- [ ] run_script/expand_installer_vars/run_hook 在 operations/script.rs
- [ ] download_and_verify 先算 size；outdated 在 update 命令层；should_offline 判 estimated
- [ ] 分区注释一致；零 warning

## 测试基础设施坑
`package/download.rs` 的 spawn_range_server：accept 后必须 `set_nonblocking(false)`（非阻塞继承导致并行偶发失败）；已支持 HEAD（响应 Content-Length、不计入 ranges）。

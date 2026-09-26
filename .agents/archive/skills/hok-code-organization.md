---
name: hok-code-organization
title: Hok 代码组织规范
description: Hok 代码组织规范（合并 purpose+layering+structure+checklist）：五层架构单向依赖、目录结构、共享函数与坑、验收清单
metadata:
  type: project
---

Hok 代码组织规范（2026-08 重构已落地，合并自 purpose/layering/target-structure/guide-checklist 四条）。**核心原则：命名即文档，位置即职责；不为重构而重构、不为去重而去重。**

**Why:** 用户 2026-08 明确：重构/去重前先问「解决什么真实问题」（行为不一致/漂移风险/正确性 bug/维护成本），答不上就不做；行数、重复数量不是目标，**可漂移的重复**才值得合并；度量用「问题是否解决」而非行数；先读官方 Scoop 源码定行为锚（[[local-scoop-source-path]]）；「顺带」改动单独评估独立价值。

**How to apply:** 添加代码前走归属决策；lib.rs/mod.rs 用分区注释（`// ===` 或 `// ───`，全文一致）；mod.rs 只放子模块声明+核心类型。

## 五层架构（单向依赖，禁止反向）
```
CLI (hok crate) → 门面 (libscoop/src/lib.rs) → 领域服务 (libscoop/src/*.rs) → 核心业务 (package/) → 工具 (internal/)
```
- **CLI 层**：参数解析/事件循环/输出渲染/命令调度；禁业务逻辑与文件系统操作（除输出）
- **门面层**：仅 pub mod/pub use，禁函数实现
- **领域服务**：Session/Config/Bucket/Cache/Shim/Shortcut/Persist/Env/Psmodule/Event/Network，pub
- **核心业务**：package/（Manifest/查询/依赖/下载/安装卸载管道），pub(crate) 经 lib.rs 选择性暴露；可依赖 internal+领域服务
- **工具层**：internal/（路径/网络/哈希/Git/OS/字符串等 14 文件），pub(crate)；**禁依赖 package/ 或领域服务类型**；glob_to_regex 在 string.rs（无 pattern.rs）

## 当前目录结构（已验证）
```
src/                     # bin crate（仓库根）：bin/hok.rs lib.rs eventloop.rs output.rs cui.rs format.rs i18n.rs scoop_handler.rs + cmd/（__cmd_reg__.rs 自动生成）
crates/libscoop/src/
├── lib.rs               # 门面：pub mod + pub use + 薄门面（fs/os/string）
├── session/config/bucket/cache/network/event/handler/shim/shortcut/persist/env/psmodule/constant/error
├── internal/            # 14 文件（arch/archive/dag/env/fs/git/github/hash/network/os/path/string/url/version）
└── package/             # mod.rs(核心类型) identity/manifest*/query/resolve/download/sync*/checkver*/checkhashes/checkurls/… + operations/（extract/link/persist/shortcut/script）
```

## 跨管道共享函数与坑（2026-08 多轮）
- `operations::run_hook`（script.rs）：installer/uninstaller file+script 执行块（install/remove 共用）
- `Package::matches_bucket_query`：bucket/name 精确匹配（查询循环/依赖解析共用）
- `manifest_source::resolve_isolated_query`：@version/URL/本地分派（install/download 共用）
- `download::download_and_verify`：**必须先对传入 set 调 calculate_download_size**（否则分片+进度条静默失效）
- `download::verify_downloads`：nightly 跳过/空 hash warn/mismatch 删缓存（install/download 共用）
- **outdated 文案归 update 命令层**（事件循环 PackageSyncDone 只 break）；should_offline 判 `total==0 && !estimated`；ignore_failures 默认 false；query_installed 已 pub

## 验证清单（重构验收）
- [ ] cargo build + cargo test 全绿；无 util.rs/helpers.rs
- [ ] internal/ 不依赖 package/ 或根目录领域服务
- [ ] run_script/expand_installer_vars/run_hook 在 operations/script.rs
- [ ] download_and_verify 先算 size；outdated 在 update 命令层；should_offline 判 estimated
- [ ] 分区注释一致；零 warning

## 测试基础设施坑
`package/download.rs` 的 spawn_range_server：accept 后必须 `set_nonblocking(false)`（非阻塞继承导致并行偶发失败）；已支持 HEAD（响应 Content-Length、不计入 ranges）。

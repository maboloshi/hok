---
name: hok-dir-layout-inventory
title: hok *_dir 路径归集完成（官方语义已核对）
description: hok *_dir 归集完成：Session 9 方法（persist/buckets 已对齐官方语义）+ internal/path.rs 仅留 buckets 2 函数（rayon 并行场景，Session 非 Sync）
metadata:
  type: project
---

hok 的 `*_dir` 路径全景（2025 盘点 + 归集完成）。**官方 Scoop 布局语义（本地源码 D:\App\github项目\Scoop\Scoop\lib 验证）**：
- `appsdir`/`shimdir`/`persistdir` 随 global（core.ps1:349/350/364，install.ps1:50 传 `$global`）→ effective 族
- `buckets` 固定用户级（buckets.ps1:1 `$bucketsdir = "$scoopdir\buckets"`）→ 用户级族
- `cache`/`scoop.db` 固定用户级（core.ps1:1301 / database.ps1:63）

**最终归集状态（激进方案，已验证）**：
- `Session` 9 个布局方法全内联：`apps_dir/app_dir/current_dir/version_dir/shims_dir/modules_dir/persist_dir`（effective 族）+ `buckets_dir/bucket_dir`（用户级族，内部 `config().root_path()`）
- `internal/path.rs` 只留 `buckets_dir/bucket_dir` 两个纯函数——唯一保留原因：`package::query.rs` 的 rayon 并行扫描，**Session 含 RefCell/Cell/OnceCell 不是 Sync，不能进并行闭包**，只能传 root 参数（session.rs 顶部文档声称 Sync+Send 是错的）
- 修掉两处与官方不符的 bug：persist.rs:39/128 用户级→effective（原 hok 全局安装 persist 写错根）；script.rs:166 buckets effective→用户级

**Working_dir 陷阱仍有效**：env.rs:55-60 按 `no_junction` 切 `"current"`/version，sync_install.rs 用 version——归集时保留分支。

**Why**: 布局语义必须对齐官方（persist 随 global 曾被误判为固定用户级，导致全局安装 persist 放错根）；Session 非 Sync 是 query.rs 必须用 root 参数的根本原因。
**How to apply**: 新路径代码优先用 `session.xxx_dir()`；并行扫描场景（rayon）用 `internal::path::bucket_dir(root, name)` 并传 `config().root_path()`（用户级）；不要新增裸 join("apps") 等。

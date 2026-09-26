---
name: hok-git-pull-official-alignment
title: hok bucket 更新对标官方 git pull（merge 语义）
description: bucket 更新对标官方 git pull（merge 语义）：libgit2 SAFE checkout、git2 0.19 API、ENOTFOUND 三个坑。
metadata:
  type: project
---

hok 的 bucket 更新已从"仅 fast-forward"改为**对标官方 Scoop `git pull -q`（merge 语义）**（2026-08 落地；ensure_updateable 修复已提交 2d3bbb7）。

**实现（`crates/libscoop/src/internal/git.rs`）**：`pull`：fetch（单分支 refspec 保持）→ `merge_analysis` 分流：
- `is_fast_forward()` → `repo.reset(&obj, ResetType::Hard, None)` 一步更新 ref/index/workdir；
- `is_normal()` → `repo.merge(&[&fetch_annotated], ...)` 3-way merge：无冲突则 `write_tree_to` + `repo.commit`（message `Merge remote-tracking branch 'origin/<branch>' into <branch>`，`repo.signature()` 读 user.name/email）+ `cleanup_state`；有冲突则 `checkout_index(None, Some(&mut co.allow_conflicts(true)))` 留冲突标记并返回 `BucketUpdateMergeConflict`；
- `NONE/UNBORN` → `BucketUpdateNotFastForward`。

**关键前置**：`ensure_updateable(repo, head_id, &fetch_commit)` 在整合前 diff(old_tree→new_tree) + **一次 `repo.statuses(None)` 全量查询**构建 `HashMap<path, Status>`，diff 路径在 map 中且 status 非空才报 `BucketUpdateLocalChanges` —— 对齐 `git pull` 拒绝覆盖本地修改。

**坑 1（libgit2）**：HEAD ref 用 `set_target`+`set_head` 移动后，SAFE `checkout_head`/`checkout_tree` 会**返回 Ok 但静默跳过不更新工作区**（hp 的 `utils/pull.rs` fast_forward 注释 "force is required" 就是此坑）。解法：预检保证无本地修改被覆盖后，ff 直接用 `reset(Hard)`。

**坑 2（git2 0.19 API）**：`MergeAnalysis` 没有 `FASTFORWARD`/`NORMAL` 常量（是 `ANALYSIS_*`），用 `is_fast_forward()`/`is_normal()`；`merge`/`merge_analysis` 参数是 `&[&AnnotatedCommit]`（`find_annotated_commit(oid)`）；`checkout_index(&self, index: Option<&mut Index>, opts)`、`reset(&self, target: &Object, ...)`、`checkout_tree(&self, &Object, ...)` 签名与旧版不同；`Commit` 用 `peel_to_commit()` 而非 `peel`+`.tree_id()`。

**坑 3（git_status_file 语义错配，2d3bbb7 修复）**：`git_status_file(path)` 对「远端新增、本地 HEAD/index/workdir 三处都不存在」的路径返回 `GIT_ENOTFOUND`（klass=3 INVALID，message "attempt to get status of nonexistent file"）——这是 libgit2 status.c 的设计契约（`sfi.count==0` 分支），flags 硬编码无法调整。hok 曾用它逐文件检查本地修改，导致 `hok update` 对含新增 manifest 的 bucket 更新整体失败（apps/main/extras 全中招）。解法：改用 `repo.statuses(None)` 一次全量（`git_status_list_new`），diff 路径不在结果集即视为 clean——与 `git status --porcelain <path>` 语义一致，性能也更好。

**测试**：`crates/libscoop/src/internal/git.rs` 末尾 `mod tests`，5 个用例 `pull_fast_forward` / `pull_fast_forward_dirty`（拦截不覆盖）/ `pull_merge_commit`（双 parent）/ `pull_merge_conflict`（留 `<<<<<<<` 标记）/ `pull_fast_forward_new_files`（远端新增文件，坑 3 回归）。测试用 git2 本地构造 origin+clone（`commit_file` helper，clone 需 `cfg.set_str("user.name/email")` 供 merge commit）。

**错误变体**：`BucketUpdateMergeConflict(String)`、`BucketUpdateLocalChanges(String)`（error.rs error_key + locales en/zh 已补，含原缺失的 `bucket_update_not_fast_forward`）。

**Why:** 用户要求 bucket 更新对标官方；官方 Scoop `scoop-update.ps1` Sync-Bucket 就是 `git pull -q`（merge 语义，非 rebase/autostash；`autostash_on_conflict` 配置只作用于 Scoop 自身更新且 stash 后不 pop）。hyperscoop 的 `run_pull` 同思路但 fast-forward 用 `.force()` 会覆盖本地修改（其注释自认待改进）。

**How to apply:** 改 bucket 更新逻辑看 `internal::git::pull`；碰 libgit2 checkout/merge 先读坑 1/2；用 status API 查「文件是否有本地修改」先读坑 3（勿用 status_file 逐文件）；对比行为读官方 `$env:SCOOP\apps\scoop\current\libexec\scoop-update.ps1` 与 hp `crates\command\src\utils\pull.rs`。

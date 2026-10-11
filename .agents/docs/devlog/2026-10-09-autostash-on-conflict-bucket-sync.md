# autostash_on_conflict：官方键在 hok 落到 bucket 同步

> 2026-10-09 ｜ 提交 `954ff56` ｜ worktree `.worktrees/c1-autostash` ｜ 分支 `feat/autostash-on-conflict`

C1。实现官方键 `autostash_on_conflict`（默认 `false`），并让它在 hok 唯一的同型站点生效。

## 与上游的差异：范围，不是位置

上游只在 `Sync-Scoop` 里认这个键（`libexec/scoop-update.ps1:113-121`：先 `git diff HEAD --name-only` 探测未提交改动，
命中则按开关决定 stash 还是中止）；`Sync-Bucket` 是裸的 `git pull -q`，**不读**它（v0.6.0 与 `master` 同）。
hok 的差异因此是**作用范围**：该键接到**每一次仓库 pull**（`bucket::update` 逐桶透传，`bucket.rs` 是唯一生产调用点），
而不是像上游那样只管自身仓库。理由是用户的 bucket 工作树里可能有**自己改过、尚未 push** 的清单——
此时中止整个 update 对用户没有帮助。

行为：命中 `Error::BucketUpdateLocalChanges` 时 `stash -u`（消息 `WIP at <round-trip>`）后继续，
**不 pop**（同官方），并 `warn` 提示 `git stash pop` 恢复；stash 自身失败即报错、绝不"没保住用户改动还继续"。
这个范围差异写进键的 description（en/zh）与 `scoop-gap-status.md`，让它在 `hok config --all` 里也看得见，
而不是只躺在注释里。

## 结构

- config 单文件闭环：字段 + accessor（默认 false）+ `set()` arm + `SETTINGS` + `list_all_rows` 行，
  移出 `OFFICIAL_ONLY_KEYS`。**另外加进 `SHARED_KEYS`**——该键与官方同名，不加会让 `map_official_key`=Supported
  （import 会写）而 `official_name`=None（export 会丢）自相矛盾。
- `internal::git` 不读配置：`bucket::update` 读一次 → `Git::pull(path, proxy, autostash: bool)`
  → `internal::git::pull`；`FakeGit` 记录该 flag，透传本身也被断言。

## 提交前 review 拦下的一个真缺陷

首版写的是 `if let Err(err) = ensure_updateable(...)`，于是**任何**失败都会在开关打开时被吞掉转去 stash：
工作树其实干净时 `stash_save` 报 "nothing to stash"，真正的失败原因反被一句"stash 失败"覆盖。
`ensure_updateable` 只在末尾返回 `BucketUpdateLocalChanges`，其余是 `find_commit`/`tree`/`diff_tree_to_tree`
的 `?` 错误 ⇒ 改成 `match` + `Err(BucketUpdateLocalChanges(_)) if autostash` + `Err(err) => return Err(err)`。
**教训：`if let Err` 用作"分支判定"时，先问这个 Result 还有没有别的 Err 来源。**

## libgit2 的两个坑

1. `Commit` / `AnnotatedCommit` / `Remote` 都借用 `Repository`，而 `stash_save` 要 `&mut`
   ⇒ `ensure_updateable` 改收 `Oid`（内部自己 `find_commit`）、merge 分支重建 annotated、fetch 后 `drop(origin)`。
2. `-u` 暂存的未跟踪文件在 stash 提交的**第三个 parent**，不在其自身 tree 里
   （`stash_foreach` 只能拿到 OID）；且 libgit2 会把消息写成 `On <branch>: <msg>`，断言用 `contains`。

## 证据

新增 4 条测试：脏 FF 的 stash（含"stash 里内容完整"）、未跟踪文件（`-u`）、config 往返 + `map_official_key`
→ Supported、config→git 透传（默认 `[false]`、开键 `[true]`）。
门禁（rebase 到主线后在自己的树上重跑）：`clippy -p libscoop` 0、`test -p libscoop` 0（856 passed / 0 failed）、
`test -p hok` 0、改动文件 `rustfmt --edition 2024 --check` 0；落地用 `--ff-only`，无 merge commit。

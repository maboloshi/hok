# git 子系统收尾小修

> 2026-09-24 ｜ 3 提交线性落地，`daa1002` → `675b991`

上个会话（评估 gix 能否替代 git2）的结论是**暂不换 gix**，端口停在 `.worktrees/gix-backend`（`feat/gix-backend`，**不合主线**）；但评估过程暴露了三个**与 gix 无关、属现有 git2 实现**的问题，本会话在 `.worktrees/r3-git-cleanup`（base `daa1002`）逐个修掉，一主题一提交：

1. **`diff_manifest_changes` 的重命名文档是错的**（`62d3d77`）：原文写「Added/modified/**renamed** files are reported with their new path」，但 `diff_tree_to_tree(Some(old), Some(new), None)` 从不调 `find_similar()` ⇒ 重命名实际表现为**旧路径 `deleted: true` + 新路径 `deleted: false`**。证据：真实 bucket（`D:\App\Scoop\buckets\scoop-app`）上与 gix 对照的 **10 处差异全部是 `R100`/`R076`**（git2 报旧路径 deleted，gix 默认开重命名追踪故不报）。同一说法还重复在 `UpdateChanges::upsert` 的文档里，一并改掉——**该注释已经误导过一次 gix 端口实现**，所以这次把「没有重命名检测」的事实写进 `internal/git.rs` 的函数文档。
2. **`bucket.rs` 的 diff 失败回退把错误丢了**（`885441c`）：`Err(_) => BucketUpdateOutcome::Full` 的回退本身安全（全量重扫），但错误全丢 ⇒ 将来排查「为什么每次全量重扫」毫无线索。改为 `warn!` 带上 bucket 名 + 基线 commit + 错误，**回退行为一字未改**。
3. **`build.rs` 只认 `<仓库根>/target`**（`675b991`）：用树外 target（`CARGO_TARGET_DIR` / `--target-dir` / `[build] target-dir`）时 `hok-shim.exe` 明明构建成功，build script 仍 panic 成误导性的 "hok-shim binary not found"。改为按序探测 **导出的 `CARGO_TARGET_DIR` → `OUT_DIR` 反推的 target 根 → `<仓库根>/target`**（去重），且**先把 release 在全部候选里找完、再找 debug**，故「release 优先于 debug」仍是全局规则；panic 文案与守卫语义不变。（动 `build.rs` 属行为变更，已先出 A/B/C 三方案 + 推荐 C 给用户拍板。）

## 证据（实测，不是静态读码）
- 探针 crate（`%TEMP%\envprobe`）实测 Cargo 行为：`cargo build --target-dir X` 时 build script 里 **`CARGO_TARGET_DIR=None`**，而 `OUT_DIR=<X>\debug\build\<crate>-<hash>\out` ⇒ **只有 `OUT_DIR` 反推能覆盖 `--target-dir` / `[build] target-dir`**（Cargo 不把它们 re-export 给 build script）；只有用户在 shell 里设置的 `CARGO_TARGET_DIR` 才会被子进程继承。
- **对照实验（关键）**：把 `<worktree>/target/release/hok-shim{,-gui}.exe` **移到别处**后 —— 树内构建 **exit 101** 且 panic 仍是原文案（守卫与报错语义保住）；同一状态下 `CARGO_TARGET_DIR=<树外>` **exit 0**、`--target-dir <树外>` **exit 0**（新探测路径确实生效，不是被树内旧产物兜住）。移回后树内构建 exit 0。
- 门禁（worktree 自建 target）：`rustfmt --edition 2024` 三个改动文件 0 差异；`cargo clippy -p libscoop` exit 0；`cargo test -p libscoop --offline` **769 passed / 0 failed**（另 4 个集成/文档套件全绿）。

## 教训
1. **注释也是行为契约**：一句「renamed files are reported with their new path」直接让一个并行端口的实现假设错了输入形态。凡是描述 libgit2 能力的句子（重命名 / 相似度 / 子模块），都要对着实际 `DiffOptions` 写，别按常识写。
2. **「回退安全」不等于「可以静默」**：全量重扫把错误掩盖成"只是慢一点"，这类静默降级要留日志，否则排查时连入口都没有。
3. **环境变量/路径类修复要做对照反证**：树内旧产物会「兜住」失败用例，让修复看起来有效——必须先把旧产物移开再看。
4. **收尾四件套的口径**：本次不是 `command-deviation` 的 S 项，计划进度表 / 审计条 / `scoop-gap-audit` 均无对应行（已核对），只回填本 devlog；本会话也**没碰** `reqwest` 版本（不换 gix 就没有统一依赖的理由）。

---

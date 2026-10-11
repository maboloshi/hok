# update/upgrade -c：只清理本次真的升级过的包

> 2026-10-09 ｜ 提交 `60e7acd` ｜ worktree `.worktrees/ca2-update-cleanup` ｜ 分支 `feat/update-upgrade-cleanup`

CA2 落地。`hok update -c` / `hok upgrade -c` 在**批次成功之后**清理旧版本。

## 形态

- 升级前对已装版本拍一次快照，批次结束后对照，**只有版本真的变了的包**才进清理名单 —— 这直接满足
  验收的「只清理本次升级过的包」，也天然覆盖了 `-U`、全为最新、仅更新 bucket 三种「什么都没升」的情形：
  名单为空 ⇒ 空操作 + 提示 `hok cleanup -a`。
- 从 `cmd/cleanup.rs` 抽出 `sweep(session, names, cache=false)`，`hok cleanup` 与 `-c` **共用**同一套渲染、
  同一套 fail-soft 选项，不写第二份清理管线。`cache=false` 是硬编码：缓存是 `-k` 的事。
- 明确**不**照抄 hyperscoop 的「先删 current 指向的版本再安装」——那会让安装期间 `current` 悬空，
  且 `current` 已切到最新版时会删掉最新版本身。

## 与计划文档的分歧（已回填文档）

计划把本项写成「官方锚点项」。**官方 v0.6.0 没有这个开关**：`scoop-update.ps1` 的 `getopt 'gfiksqa'`
里没有 `c`，`libexec/scoop-upgrade.ps1` 也已并入 `update`。所以它是**纯新增**，短名跟随 hyperscoop
（`-c/--cleanup`），「不改同名命令既有语义」的前提仍然成立。已在 `cli-additions-plan.md` §4 就地更正。

## 证据

- 4 条 e2e（真二进制 + 隔离根）：升级路径、`current` 保留、旧版本被清、下载缓存未被碰、提示文案；
  外加一条**反证**——不带 `-c` 时残留必须留下。
- 变异测试（把「只清变了的」改成「清全部」并开 `cache=true`）⇒ 4 条里 3 条失败（exit 101），恢复后复绿。
  这条是"测试能反驳自己"的检查，不是形式。
- 门禁：`clippy -p libscoop` 0、`test -p libscoop` 0（852 通过）、`clippy -p hok` 0、`test -p hok` 0、
  改动文件 `rustfmt --edition 2024 --check` 0。落主线用 rebase → 在**重放后的树**上重跑全套门禁 → `--ff-only`。

## 顺带记录的两个环境坑（本机，不在仓库内）

1. `$CARGO_HOME/config.toml` 的 `rustc-wrapper` 值里是未转义的反斜杠路径 ⇒ 机器上**所有** cargo 命令
   报 "could not parse TOML configuration"。已就地改成正斜杠（仓库外，语义等价）。
2. 恢复 wrapper 后，**全新 worktree** 编译 C 依赖（`libsqlite3-sys` / `zstd-sys` / `libz-sys`）时
   sccache 报 `Compiler not supported: Failed to detect showIncludes prefix`（clang-cl）。
   本主题的所有 cargo 命令都以 `RUSTC_WRAPPER=''` 运行。新 worktree 的配方值得写进
   [[hok-worktree-parallel]]，但本次未改技能（见该技能的收尾纪律）。

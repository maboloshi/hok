# S14 `bucket` / `shim` 参数与校验

> 2026-09-13 ｜ 已闭环，4 提交，分支 `fix/s14-bucket-shim` → 已线性 ff 进 `refactor/edition-2024`，tip `dca2b51e`

**结论（用户拍板）**：bucket 全对齐（分 3 组）；URL 校验 = 官方 `Convert-RepositoryUri` **正则 1:1 移植**（**不用 `url` crate**——它不接受官方接受的 `git@host:u/r.git` 等 scp 形式）；shim 三项全做并**接受破坏性变更**；`list -g` 的作用域语义落 **lib**。
**实现**：
- `crates/libscoop/src/internal/git.rs`：`normalize_git_uri`（唯一入口：校验 + 重复比较共用，**大小写不敏感**对齐 PS `-eq`）+ 16 例官方求值对照单测；从 `fetch_options` 抽出 `remote_callbacks()`/`proxy_options()` 供 fetch 与预检共用；`Git` trait 新增 `ls_remote`。
- `crates/libscoop/src/bucket.rs`：`add` 拆 `resolve`（URL 解析/校验 → 重复检测）+ `add_resolved`（预检 + clone），`add` 保薄封装 ⇒ **`src/cmd/import.rs` 不用改且自动获得同样校验**。
- `src/cmd/shim.rs`：`-g` 加 `global = true`；**去掉 `Add.parts` 的 `allow_hyphen_values`**；`list` 剔除字面 `*`；缺 `<command_path>` 走官方文案。
- `crates/libscoop/src/shim.rs`：`list_shim_info(session, global_only)`，`-g` 时**连 local 目录都不扫**。
**⚠️ 两个新踩到的坑（勿再踩）**
1. **git2 0.19 `RemoteConnection::list()` 对「零 refs 仓库」会 abort 进程**：`from_raw_parts(null,0)` 触发 Rust 非空指针前置条件（debug `0xC0000409`，release 为 UB）。⇒ `ls_remote` **只做 `connect_auth` 握手、不调 `list()`**（语义等价：connect 已完成传输握手 + ref 广告）；空 bare repo 有回归单测。代价：对"能连上但 refs 解析异常"的极端情形比 `git ls-remote` 略宽。
2. **`Add.parts` 的 `num_args = 2..` 必须改 `1..`**：`-g` 夹在值中间（`shim add c cmd.exe -g foo`）会让 pending 组少一个值，clap 报 "2 values required"。改 `1..` 后 `<name> <target>` 缺失改由 handler 报官方 `<command_path> must be specified for subcommand 'add'` + exit 1。
**测试**：隔离 `SCOOP`/`SCOOP_GLOBAL` 进程级全绿（重复仓库 warn+**2**、非法 URL 两行 ERROR+**1**、死仓库多行+**1**、`list -g` 只列 global、`list *` 全列、`info -g` 跨作用域=**2**、`rm ghost -g`=**3**、`add` 五种 `-g` 位置与 `-- -g` 转发）。
**破坏性变更**：①`hok shim add x cmd -g` 由「`-g` 当 shim 参数、建 local」→「建 global」，`-` 开头的 shim 参数必须走 `--`；②`shim add <只有名字>` 文案由 clap 通用错 → 官方文案（仍 exit 1）；③`bucket add` 拒绝官方正则不认的 URL（exit 1）与重复仓库（warn + exit 2），输出由 `Adding bucket … ok` → `Checking repo... OK` + `The <name> bucket was added successfully.`。
**有意偏离 / 遗留**：clone 失败仍打 libgit2 原文（官方 `Failed to clone …` 未对齐）；`Checking repo... ` 与 ERROR 分属 stdout/stderr（官方都在 stdout，hok 保持 stderr 约定）；成功行带 hok `✓ ` 前缀；`bucket list -k`/`rm` 多名称、`shim` 无子命令默认 list / 无 `alter` / 合并正则校验保留；无 git 检查不做（内嵌 libgit2）；clone 失败 `Remove-Item` 不补（实测不残留目录）。

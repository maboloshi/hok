# S6 调研：`cleanup` / `cache` 数据安全（2026-09-13）

> 范围：`command-deviation-audit-2026-09.md` §4.1（cache）+ §4.2（cleanup）。
> 对应计划：`command-deviation-fix-plan-2026-09.md` §3 S6。
> worktree：`.worktrees/s6-cleanup-cache`，分支 `fix/cleanup-cache-safety`，base `14b9c29`。
> 状态：**调研报告，等确认后动手**。

---

## 1. 结论摘要

| # | 偏离 | 判定 | 危害 |
|---|---|---|---|
| A1 | `cleanup -k` 清空**整个**下载缓存 | **该做（数据安全，最高优先）** | 删掉其他 app + 本 app 当前版本的缓存，重装即重下 |
| A2 | `cache rm/list` 用**子串**匹配 app 名 | **该做（数据安全）** | `rm zip` 连带删掉 `7zip` 的缓存 |
| A3 | `cache rm <app>` 不删 aria2 sidecar `<app>.txt` | 该做（低成本） | 残留 `.txt` 与官方行为不一致 |
| B1 | `cleanup -g` 无 admin 检查 | 该做 | 非管理员可对全局目录动手（后续删除必失败，但缺官方前置报错） |
| B2 | `cleanup -a -g` 只清全局 | 该做 | 本 app 的本地旧版本清不掉 |
| C1 | `cleanup` 无参/无 `-a` 时**清全部** | **该做（危险默认）** | 本想清理一个 app，实际清了所有已装 app 的旧版本 |
| C2 | 命名 app 未安装时无任何提示 | 该做（文案） | 静默跳过；官方给 `'x' isn't installed.` + 跨作用域提示 |
| C3 | 裸 `cache` / `cache <app>` 报用法错（exit 1） | **需拍板** | 官方默认落到 `show`；hok 需 `cache list` |
| C4 | `cleanup`/`cache` 输出样式（逐文件 `Removing ...`、`Deleted: N files, size`） | 建议**缓办** | 属 S7「低风险批量：文案/列名」批次 |

**全部均非有意偏离**：`git log` 与 `devlog.md` 无相关设计记录（`git log -- src/cmd/cleanup.rs`、`crates/libscoop/src/cache.rs` 未见语义决策提交），技能 `scoop-gap-audit.md:82` 第 ⑤⑥ 条也把它们列为「语义不同，待用户决策」。

---

## 2. 实测复现（worktree 自建 `hok.exe` 副本，隔离 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE`）

夹具：本地 `apps/demoapp/{1.0.0,2.0.0}` + junction `current→2.0.0`；全局 `apps/globapp/` 同构；
缓存 `demoapp#1.0.0#a.zip`、`demoapp#2.0.0#b.zip`、`other#1.0#c.zip`、`demoapp.txt`、`leftover.download`。

| 命令 | 实测结果 | 官方期望 |
|---|---|---|
| `hok cache rm zip` | 删掉 `zip#1.0#a.zip` **和 `7zip#26.0#b.7z`**；`zip.txt` 留下 | 只删 `^zip#`；并删 `zip.txt` |
| `hok cleanup demoapp -k` | 缓存目录**全空**（含 `demoapp#2.0.0`、`other#1.0`、`.txt`、`.download`） | 只删 `demoapp#*` 但保留 `demoapp#2.0.0#*`，再清 `*.download` |
| `hok cleanup`（无参、无 `-a`） | 清理了**全部**本地 app，exit 0 | `ERROR: <app> missing` + usage，exit 1 |
| `hok cleanup -g demoapp`（非管理员） | 无 admin 报错；输出「没有旧版本需要清理」exit 0 | `ERROR: you need admin rights to cleanup global apps`，exit 1 |
| `hok cleanup -a -g` | 只清全局（`globapp`），本地 `demoapp` 仍在 | 本地 + 全局都清 |
| `hok cleanup ghost` | 「没有旧版本需要清理」exit 0 | `ERROR: 'ghost' isn't installed.` exit 0 |
| `hok cache` | 用法错 exit 1 | 等同 `cache show`，exit 0 |
| `hok cache rm` / `cache rm a b` / `cache list zip extra` | 均用法错 exit 1 | `rm` 无参 → `ERROR: <app(s)> missing` exit 1；多参数拼正则 |

---

## 3. 证据（官方 vs hok file:line）

**官方**
- `Scoop/libexec/scoop-cache.ps1:28` `cacheshow`：`Name -Match "^$app#"`（锚定 app 名）
- `:36-46` `cacheremove`：多参数 `'(' + ($app -join '|') + ')'`；`*`/`-a`/`--all` → 全删
- `:37-40` 无参数 → `ERROR: <app(s)> missing` + `my_usage` + `exit 1`
- `:53-55` 删每个文件后顺带 `Remove-Item "$cachedir\$($curr.Name).txt"`（aria2 sidecar）
- `:61-71` `switch($cmd)`：`rm`→remove、`show`→show、`default`→**show**（裸 `cache` / `cache <app>` 均落到 show）
- `Scoop/libexec/scoop-cleanup.ps1:24` 既无 app 也无 `-a` → `ERROR: <app> missing` + usage + `exit 1`
- `:26-28` `-g` 且非 admin → `ERROR: you need admin rights to cleanup global apps` + `exit 1`
- `:32-33` `-k` 逐 app：`Remove-Item "$cachedir\$app#*" -Exclude "$app#$current_version#*"`
- `:67-70` `-a -g`：`$apps = 本地已装`，再 `+= 全局已装`（**本地 + 全局**）
- `:79-81` `-k` 收尾 `Remove-Item "$cachedir\*.download"`
- `Scoop/lib/core.ps1:1084-1121` `Confirm-InstallationStatus`：`'x' isn't installed.` / `isn't installed globally, but it may be installed locally.` + warn `Try again without --global`（exit 0）

**hok**
- `src/cmd/cleanup.rs:29-35` `apps.is_empty()` ⇒ `package::cleanup` 视为「全部」；`:27` 只 `set_global` 无 admin；`:54-56` `cache::remove(session, "*")`
- `crates/libscoop/src/package/cleanup.rs:29-43` 空 names → 扫 `apps_dir` 全部；`:58-65` 已解析 `current_version`（可复用）；`:86-88` 无旧版本即 `continue`
- `crates/libscoop/src/cache.rs:83-93` `package_name().to_lowercase().contains(query)`（子串）；`:134-178` `remove`：`*` 全删、按 query 命中删文件但**不碰** `.txt`
- `src/cmd/cache.rs:19-22` `command: Command` 必填子命令；`:28-41` 单查询 + `arg_required_else_help`
- `tests/exit_codes.rs:65` 锁定裸 `cache` = exit 1

**可复用的现成入口（避免重复实现）**
- admin 门：`src/cmd/shared_args.rs:116 ensure_global(session, global, verb)` + `locales/*.yml:220 admin_rights_required`（`verb = "cleanup"` 与官方逐字一致）
- 当前版本解析：`crates/libscoop/src/package/install_state.rs:122 resolve_current_version`（`package::cleanup` 已调用）
- 已装 app 枚举：`package::query::query_installed`（`src/cmd/reset.rs:35-41` 的 local→global 两趟写法可直接借鉴）
- 「未安装」文案：`locales/*.yml:344 cmd.not_installed`

---

## 4. 候选方案与代价

### A1 `cleanup -k`
- **A（推荐）完全对齐官方**：在 `package::cleanup::cleanup` 逐 app 解析出的 `current_version` 旁，调用新的 `cache::remove_outdated(session, app, current_version)`（删 `app#*` 保留 `app#<current>#*`），循环后统一 `cache::remove_incomplete(session)` 清 `*.download`。
  代价：`cleanup` 需新增一个选项参数（如 `CleanupOptions { ignore_failure, cache }`），`cache.rs` 新增两个函数 + 单测；命令层 `-k` 分支删除。
- **B 保守混合**：保留「所有已装 app 的当前版本」缓存，其余全删。实现更简单，但某 app 的**旧版本**缓存仍被留下（官方会删）——比现状安全，但仍是偏离。
- **C 不改**：现状 = 清空整个缓存。重装任何 app 都要重下，且连累其他 app。

### A2 `cache rm` / `cache list` 匹配
- **A（推荐）锚定 + 多 app**：`CacheFile::package_name()` 与查询参数做**大小写不敏感全等**（等价官方 `^app#`），支持多参数（`cache rm a b`）。`list` 同步改，保证 show/rm 语义一致。
  说明（有意微偏离，需登记）：官方把参数拼成正则，`*`/`.` 等元字符可用；hok 用字面全等，`*` 仍保留「全部」含义。日常用法（app 名）完全一致，且避免正则注入。
- **B 只改 `rm`**：`list` 维持子串。show/rm 不一致，不推荐。
- **C 不改**：`rm zip` 继续误删 `7zip`（数据安全）。

### A3 sidecar
- 附在 A2 的实现里：按 query 删除后，对每个命中 app 顺带删 `<cachedir>/<app>.txt`（`NotFound` 容忍）。官方行为，成本一行循环。

### B1/B2 admin 与 `-a -g`
- **A（推荐）**：`-g` 走 `ensure_global(session, args.global, "cleanup")`（退出 1，文案逐字对齐）；`-a -g` 按 `reset.rs` 的「本地一趟 + 全局一趟」编排，结果合并后打印一次收尾文案。
- **B 只补 admin 检查**，`-a -g` 仍只清全局：留一半偏离，不推荐。
- **C 不改**。

### C1 无参语义
- **A（推荐）对齐**：`args.app` 空且无 `-a`/`*` → `ERROR: <app> missing` + usage，exit 1（新增 locale key，参照 `reset_app_missing`）。
- **B 保留现状**（无参=清全部）但**登记为有意偏离**并在 `--help` 写明。⚠️ 这是 hok 现有可用行为，可能已被用户脚本依赖 ⇒ 属破坏性变更。
- **C 折中**：无参时改为「等价 `-a`」但打印显式警告。仍与官方不同。

### C2 未安装文案
- **A（推荐）**：命名 app 未安装时逐条 `cmd.not_installed`（可复用 `query_installed` 的既有报错路径），exit 保持 0。
- **B 完整对齐 `Confirm-InstallationStatus`**：再加跨作用域提示（`isn't installed globally, but ...`）与 `isn't installed correctly`。文案最齐，但需新增 2 组 locale + 双作用域探测。
- **C 不改**。

### C3 裸 `cache`
- **A（推荐）对齐**：`cache` 无子命令 → 视为 `show`（列全部/按参数过滤），退出 0。需改 `tests/exit_codes.rs:65`。
- **B 保留**（必须带子命令，exit 1），登记有意偏离 + `--help` 说明。
  - 注：`scoop-gap-audit.md:129` 把「裸 `cache`」退出码归 S6；S1 只是把它降级为「语义会话处理」。

### C4 输出样式
- 建议**缓办**：`Removing <file>...` + `Deleted: N files, <size>`、逐 app `Removing app: v1 v2`、`app is already clean` 等属 S7 文案批次；本会话只保证数据安全与语义正确。
- 若你要一并做，追加 1 个 commit，但需改现有 `cache` 表格单测/快照。

---

## 5. 推荐落地（若全部选 A）

**提交拆分（3 个主题 commit + 收尾）**

1. `fix(cache): anchor cache matching to app names`（A2+A3）
   - `crates/libscoop/src/cache.rs`：`list`/`remove` 改多参数 + 全等匹配；新增 `remove_outdated`、`remove_incomplete`；sidecar 清理；单测（`rm zip` 不命中 `7zip`、sidecar 删除）。
   - `src/cmd/cache.rs`：`List`/`Remove` 接受 `Vec<String>`。
2. `fix(cleanup): keep current version cache with -k`（A1）
   - `crates/libscoop/src/package/cleanup.rs`：新增 `CleanupOptions`；逐 app 清过期缓存 + 收尾 `*.download`；单测。
   - `src/cmd/cleanup.rs`：调用新签名。
3. `fix(cleanup): align global scope, admin gate and missing-app semantics`（B1+B2+C1+C2）
   - `src/cmd/cleanup.rs`：`ensure_global`；`-a -g` 本地+全局两趟；无参报错 exit 1；未安装文案。
   - `locales/{en,zh}.yml`：新增 `cleanup_app_missing`（+ 视 C2 选项的跨作用域文案）。
   - `tests/exit_codes.rs`：视 C3/C1 决定，补 `cleanup` 无参 = 1 等断言。
4. （可选，仅 C3 选 A）`fix(cache): default to listing without a subcommand` + 改 `tests/exit_codes.rs:65`。

**影响面**：`src/cmd/{cleanup,cache}.rs`、`crates/libscoop/src/{cache.rs,package/cleanup.rs}`、`locales/{en,zh}.yml`、`tests/exit_codes.rs`。无新增依赖。
**破坏性变更**：C1（无参不再清全部）、C3（裸 `cache` 不再报错）。其余为行为修正。
**回归**：`cargo test -p libscoop`、`cargo test --test exit_codes`；隔离环境 e2e 复演本报告 §2 全部夹具。

**收尾四件套**：计划 §6 S6 行、审计报告 §4.1/§4.2 状态列、`scoop-gap-audit.md:82` 第 ⑤⑥ 条、必要时 `devlog.md`。

---

## 6. 待确认点（一次性拍板）

1. **范围**：全做（A1–A3 + B1/B2 + C1/C2，C3 另议）还是只做数据安全（A1–A3）？
2. **A1**：完全对齐官方（逐 app 保留当前版本）还是保守版（保留所有已装 app 当前版本）？
3. **A2**：`cache list` 是否同步锚定？官方 regex 元字符语义是否保留（推荐不保留，登记微偏离）？
4. **C1**：`hok cleanup` 无参改为官方报错 exit 1（破坏性）还是保留现状并登记偏离？
5. **C3**：裸 `cache` / `cache <app>` 是否对齐官方「默认 show」（破坏性，需改 `tests/exit_codes.rs:65`）？
6. **C2**：只报 `isn't installed`（A）还是完整 `Confirm-InstallationStatus` 跨作用域提示（B）？
7. **C4**：输出样式（逐文件 `Removing` / `Deleted:` 行）本会话做还是留给 S7？

---

## 7. 落地结果（2026-09-13 闭环）

**用户拍板**：范围全选（A1–A3 + B1/B2 + C1/C2，含 C3 裸 `cache`）；A1 完全对齐官方；A2 `list` 同步锚定且不保留官方 regex 元字符；C1/C3 两处破坏性变更**都对齐官方**；C2 完整对齐 `Confirm-InstallationStatus`；C4 输出样式留 S7。

**提交（4 个，rebase 到 S5 的 `5ae994d` 后线性 ff 进 `refactor/edition-2024`，无 merge commit）**

| 提交 | 主题 |
|---|---|
| `1e6b929` | `fix(cache): anchor cache selection to app names`（A2+A3：锚定全等匹配、多 app、sidecar、`<app(s)> missing` exit 1） |
| `33ba858` | `fix(cleanup): keep each app's current version cache with -k`（A1 + 目标列表始终显式 + `installed_app_names`） |
| `1c1311b` | `fix(cleanup): require admin for -g and sweep local+global with -a -g`（B1+B2） |
| `cc2c072` | `fix(cleanup,cache): align no-arg and missing-app semantics`（C1+C2+C3 + `tests/exit_codes.rs` + locales） |

**落地后的实测（隔离 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE`）**

| 命令 | 结果 |
|---|---|
| `hok cache rm zip` | 只删 `zip#1.0#a.zip` + `zip.txt`；`7zip` 保留 |
| `hok cache rm zip 7zip` | 两个 app 的缓存都删（多参数） |
| `hok cache` / `hok cache zip` | 列全部 / 只列 zip；exit 0 |
| `hok cache rm`（无参） | `<app(s)> missing`，exit 1 |
| `hok cleanup demoapp -k` | 删 `demoapp#1.0.0` 与 `*.download`；保留 `demoapp#2.0.0`、`other#1.0`、`zip*` |
| `hok cleanup`（无参） | 报错，exit 1（不再清全部） |
| `hok cleanup ghost` / `hok cleanup globapp`（跨作用域） | 只报错 + 提示，**不动任何 app**，exit 0 |
| `hok cleanup -g demoapp`（非管理员） | 管理员报错，exit 1 |
| `hok cleanup -a` | 只清本地；全局 `globapp` 不动 |
| `hok cleanup demoapp`（有 manifest 无 `install.json`） | 正常清理（官方 `Select-CurrentVersion` 只读 manifest） |
| `hok cleanup brokenapp`（`current` 丢失） | 报 `'brokenapp' 未正确安装`，跳过，exit 0 |

**门禁**：`cargo test -p hok` 全绿（43+3+4+3）；`--test exit_codes` 4/4；`--test i18n_keys` 3/3；libscoop 新增单测 6 条全过（`cargo test --workspace` 仅既有环境性失败：`permission::tests::grant_users_write_inherit_*` ACL error 5、`manifest_cache::serving_a_stale_cache_warns_once` 全局 `AtomicBool` 并行 flake，两者文件本会话均未触碰）；clippy 本仓 0 新增、`rustfmt --edition 2024 --check` 干净。

**实现期间发现的额外缺陷（本报告原始清单未列）**：`package::cleanup::cleanup` 把"空名单"当"扫描全部已装 app"，而按 `Confirm-InstallationStatus` 过滤后的命名路径正会传空名单 ⇒ 首版实测 `hok cleanup ghost` **清光了所有 app 的旧版本**。已把 API 语义改为"目标列表始终显式"（空名单 = no-op）并加单测钉死。

**有意/微偏离（已登记）**：`cache` 用字面全等而非官方 regex 元字符；缺参报错用提示文案替代官方 usage 块；broken app（`current` 缺失）报错后跳过（官方会删光其版本目录，hok 保留更安全行为）；`cache`/`cleanup` 输出样式归 S7。

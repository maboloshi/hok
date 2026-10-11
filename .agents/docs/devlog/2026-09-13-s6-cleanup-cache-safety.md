# S6：`cleanup`/`cache` 数据安全

> 2026-09-13 ｜ 4 commit：`1e6b929` `33ba858` `1c1311b` `cc2c072`

**背景**：两条"想清一点、清了全部"的数据安全缺陷 + 一组未对齐的官方语义。

- **`cache` 选择**（`1e6b929`）：`list`/`remove` 原用大小写不敏感**子串**匹配 app 名，`hok cache rm zip` 连 `7zip`/`unzip` 一起删；改为按解析出的 `package_name()` 全等（等价官方 `^<app>#` 锚定），支持多 app，并在按 app 删除后顺带清 aria2 `<app>.txt` sidecar（官方 `scoop-cache.ps1:53-55`）。
- **`cleanup -k`**（`33ba858`）：原实现 `cache::remove("*")` 清空整个缓存目录——连本 app 的当前版本和其它 app 的缓存一起丢。改为在逐 app 循环里（`current_version` 已解析处）调 `cache::remove_outdated(app, current)`，循环后 `cache::remove_incomplete()` 清 `*.download`（官方 `scoop-cleanup.ps1:32-33,79-81`）。
- **作用域/权限**（`1c1311b`）：`-g` 走共享 `ensure_global` 管理员门；`-a -g` 由"只清全局"改为本地一趟 + 全局一趟（官方 `:67-70`）。
- **无参与未安装语义**（`cc2c072`）：裸 `cleanup` 由"清全部"改为 `<app> missing` + exit 1（官方 `:24`）；命名 app 走完整 `Confirm-InstallationStatus`（跨作用域 `--global` 双向提示、`isn't installed correctly`，exit 0）；裸 `cache`/`cache <app>` 默认列缓存（官方 `scoop-cache.ps1:61-71`）。

## 验证

隔离 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE` 环境进程级实测 10 条（`rm zip` 不再命中 `7zip`、`-k` 保留当前版本且不碰其它 app、裸 `cache`=0、裸 `cleanup`=1、`-g` 非管理员=1、`cleanup ghost`/跨作用域不再误清全盘）；新增 libscoop 单测 6 条；`cargo test -p hok` 全绿、`--test exit_codes` 4/4、`--test i18n_keys` 3/3；`rustfmt --edition 2024` 合规 · clippy 本仓 0 新增。落地按 `hok-worktree-parallel`：脚本 `worktree-merge.ps1` 完成 rebase 到 S5 的 `5ae994d` → 漂移校验（4 commit / 主题 / 7 文件集守恒）→ 线性 `--ff-only`（无 merge commit），随后在落地树上重跑全部门禁。

## 教训

1. **"空集合 = 全部"是脚枪**：`package::cleanup::cleanup` 原把空名单当"扫描所有已装 app"，而命名路径按 `Confirm-InstallationStatus` 过滤后正好会传空名单 ⇒ 实测 `hok cleanup ghost` **清光了所有 app 的旧版本**。修法不是加个 `if`，而是把语义拆开：目标列表**始终显式**，`-a` 自己用 `installed_app_names` 枚举；并补一条"空名单是 no-op"的单测钉死。
2. **别用更严的邻居谓词代替官方谓词**：首版用 `query_installed` 的 `Installed` 判"装好了没"，它额外要求 `install.json` 可解析，比官方 `failed`（只看 `current` junction + 版本可解析）严——会把官方认为可清理的 app 误报"未正确安装"并跳过。改成按官方 `failed` 逐字复刻后，`install.json` 缺失的 app 正常清理、`current` 丢失的 app 才报错。
3. **数据安全类改动要同步核对"谁还会读到同一份数据"**：`-k` 从"清全缓存"收窄成"清本 app 过期项"后，`*.download` 这类不属于任何 app 的残留必须显式补清（官方也是单独一行），否则会留下垃圾。
4. **破坏性语义改动要一次问清**：裸 `cleanup`/裸 `cache` 的默认行为改动会进 `tests/exit_codes.rs` 与 `--help` 语义面，开工前用 [[investigate-first]] 把"是否对齐 + 是否保留"集中拍板，比改完再回退便宜。

---

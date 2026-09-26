# hok ↔ Scoop 子命令偏离审计（2026-09-12）

> **主题**：`checkup` 之外，hok 还有哪些**同名子命令**存在行为偏离。
> **对照源**：本地官方源码 `../Scoop`（`libexec\scoop-*.ps1`、`lib\*.ps1`、`bin\*.ps1`，v0.5.3）；hok 侧为当前工作树（2026-09-12，HEAD `3d4969f`）。
> **方式**：4 组并行子代理**静态读码**逐命令比对；除 §3「已人工复核项」外，行号可信，但结论**未逐条实测**。
> **状态**：**当前有效**；S1（退出码口径）+ S13（安装/卸载批量行为）已于 2026-09-12 处置，逐条对应见 **§10**；其余条目仍待各自会话决策。

---

## 0. 结论摘要

除 `checkup`（hok=已装包健康检查，官方=环境检查）之外，**还有一批同名命令偏离**，按严重度分四类：

| 类别 | 含义 | 代表命令 |
|---|---|---|
| **A 语义不同** | 同一个命令名，做的事情不同（等同 checkup 级别） | `alias`、`export`/`import`、`config`、`which`、`cleanup`、`cache`、`download`(aria2)、`virustotal`、`missing-checkver`、`reset`、`uninstall`/`update` 批量失败策略、`install`/`update` 的 `-i` 冲突 |
| **B 行为缺失 / 默认值 / 边界** | 命令主功能一致，但缺分支、缺参数、默认值不同 | `search`、`list`、`status`、`bucket`、`create`、`shim`、`hold`/`unhold`、`prefix`、`cat`、`home`、`depends`、`checkver`(变量替换/hash 兜底) |
| **C 有意偏离（已登记）** | 代码注释或文档已写明理由，**不建议改** | `info` ×4、`checkver` Digest 解码、`formatjson` JSON5、`checkhashes` 对象回写、`parse_json` 严格报错、`reset` 多跑 `post_install`、单二进制无自更新 |
| **D hok 独有命令** | 官方无同名命令，只能与官方 `bin/*.ps1` 辅助脚本比 | `checkver`、`checkhashes`、`checkurls`、`formatjson`、`missing-checkver`、`completions`、`reinstall`、`upgrade` |

**最接近「下一个 checkup」的三处**（功能性缺陷，非风格差异）：

1. **`alias`** —— hok 只把别名写进 config，**从不生成 `shims\scoop-<name>.ps1`，别名不可作为命令调用**；
2. **`export` / `import`** —— 与官方 Scoopfile **schema 互不兼容**，官方导出的 Scoopfile 无法被 hok 导入；
3. **`config`** —— 键白名单化（未建模键直接报错）+ `unset`/无单键读取/`proxy none` 语义与官方不同。

---

## 1. 口径与分类

- **只收录"同名命令、可观察行为不同"**：语义、参数含义、默认值、输出、退出码、边界处理。
- **不收录**：Rust vs PowerShell、异步/并发、性能、纯代码风格；「hok 多一个功能」除非改变了共有命令的行为。
- **证据要求**：每条都有 hok `file:line` 与 Scoop `file:line` 双侧证据。
- **类别取值**：`语义不同` / `行为缺失` / `默认值不同` / `输出或文案不同` / `退出码不同` / `边界处理不同` / `hok 独有命令(无官方对标)` / `有意偏离(已登记)`。
- **退出码背景（2026-09-12 已更新）**：hok 现为 **clap 用法错/缺参 exit 1、帮助/版本/裸命令 exit 0**，并逐命令补了官方专用码（`which` 2、`bucket list` 空 2、`shim` 2/3、`virustotal` 位掩码、`search` 无匹配/`list` 空装 1）。下方各表"缺参退出码不同"的行**均已修复**，原文保留只为溯源；逐条对应见 §10。

---

## 2. 分级索引（按命令）

| 命令 | 主要类别 | 关键偏离 | 明细 |
|---|---|---|---|
| `install` | A/B | ~~`-i/--independent` 被 hok 占用为 `-I`~~ **✅ update-align**；无 scoop 自更新；多出官方会拒绝的 6 个开关（`-S/--escape-hold` 已删）；~~成功多打印 `Everything is ok!`~~ **2026-09-13 已删页脚** | §4 |
| `uninstall` | A | 拒卸被依赖包；~~静默跳过 held 包~~ **✅ update-align**；运行中进程中止整批（官方跳过继续，exit 0）；`scoop` 自身不可卸；未安装 exit 1 vs 0 | §4 |
| `update` | A/B | ~~bucket 同步被 15 分钟 cooldown 门控~~ **✅ update-align（冷却整体删除）**；无 scoop 自更新；`-f` 不留 `_<v>.old` 备份；无 `-q`；未安装 exit 1 vs 0 | §4 |
| `upgrade` | A/D | 官方无此命令（只是建议别名 `upgrade = scoop update *`）；~~`-f` 在 hok=ignore-failure~~ **✅ S7 起 = force**；~~flag 集比 `update` 少 4 个~~ **✅ update-align 补 `-i`/`-k`** | §4 |
| `reinstall` | D | 官方无此命令 | §4 |
| `reset` | A | 不自动识别全局安装；单应用无 admin 检查；不接受 `app@version`/`bucket/app`；多跑 `post_install` | §4 |
| `cleanup` | A | 无参=清全部（官方报错）；`-k` 清空整个下载缓存；`-g` 无 admin 检查；`-a -g` 只清全局 | §4 |
| `cache` | A | 必须带子命令；`rm` 单查询 + 子串匹配（`zip` 命中 `7zip`）；不删 aria2 sidecar | §4 |
| `list` | B | 不列全局安装；无 `Updated` 列/`Global install`；无匹配退出码 0 vs 1；多查询 OR（官方只看第一个） | §5 |
| `search` | B | 不搜 known-but-unadded bucket 远程结果；无参报错 vs 列全部；默认不匹配 bin 名 | §5 |
| `status` | B | 无 `-l/--local`；无 scoop/bucket 过期检查；多 `Source` 列 | §5 |
| `info` | C | 4 条已登记有意偏离 + 2 条语义不同（deprecated 版本显示、多 bucket 未安装时的呈现） | §5 |
| `depends` | A/B | 输出为缩进树（官方平铺依赖序）；重复依赖重列；环形依赖不报错；helper 判定与 `lessmsi` 默认不同 | §5 |
| `which` | A | 打印 shim 文件本身而非其指向的真实目标；不查 PATH、不查全局 shim；列出全部变体 | §5 |
| `prefix` | B | 不回退全局目录；错误文案不同 | §5 |
| `cat` | B | 不解析 URL/本地 manifest；多 bucket 交互提示（官方取第一个）；JSON 美化缩进/行尾不同 | §5 |
| `home` | B | 不解析 URL/本地 manifest；多 bucket 交互提示；缺 `homepage` 无守卫 | §5 |
| `bucket` | B | 空列表退出码 / 排序 / 列顺序 / `known` 输出 / `-k` 语义 / 不校验重复仓库与 git URL | §6 |
| `config` | A | 键白名单化；`unset` vs `rm`；无单键读取；`proxy none` 语义相反；`aria2-min-split-size` 默认 20M vs 5M | §6 |
| `export` | A | 输出 schema 与官方 Scoopfile 完全不兼容；无 `-c/--config`；默认丢非 bucket 安装 | §6 |
| `import` | A | 忽略 `config` 段；不添加 bucket；忽略 app 的 Global/arch/Held；**官方 Scoopfile 导不进来** | §6 |
| `alias` | A | **别名不可调用**（不生成 shim 脚本）；重名静默覆盖；`rm` 不存在也 exit 0 | §6 |
| `hold`/`unhold` | B | 无 `scoop` 自身特例（官方写 `hold_update_until`）；`unhold` 写 `hold:false` vs `null` | §6 |
| `shim` | A/B | `-g` 必须前置；`list -g` 仍合并 local+global；`list *` 报正则错；`alter` 不做（决策） | §6 |
| `create` | A | 会真实下载 URL 并算 SHA-256；模板与默认值不同；默认打到 stdout；无交互式版本询问 | §6 |
| `download` | A | `aria2-enabled` 在 hok 是内置分片开关（从不调用 aria2）；失败退出码相反；缺 special-URL 重写/Referer | §7 |
| `checkver` | D | hok 独有命令；github/sourceforge 快捷分支、jsonpath/xpath、autoupdate 变量替换、hash 兜底链差异约 50 处 | §7 |
| `checkhashes` | D | hok 独有命令；缓存策略（默认复用 vs 官方重下）、空 hash 处理、`all_urls()` 合并等约 22 处 | §7 |
| `checkurls` | D | hok 独有命令；HEAD vs GET、200–399 vs 仅 200、无 ftp、UA 固定等约 14 处 | §7 |
| `missing-checkver` | D | hok 独有命令；`-s/--supported` 与官方 `-SkipSupported` **语义相反**；输出格式不同 | §7 |
| `virustotal` | A | 默认行为相反：官方查 `/files/{hash}`，hok 总是提交扫描；缺 `-a/-s/-n/-p`；退出位 8 缺失 | §7 |
| `formatjson` | D | hok 独有命令；子命令被 kebab 化为 `format-json`；解析失败行为、告警、写盘条件不同 | §7 |
| `ci-auto-pr` | D | hok 独有命令；`-u` 可选、无 `-App`、无 `-ThrowError`、变更判定用字节快照 vs `hub diff` | §7 |
| `completions` | D | 官方全树无补全生成器 | §7 |
| `checkup` | A | **已修（S5，`5ae994d`）**：hok `checkup` 已对齐官方环境检查；原已装包健康检查迁至新命令 `health`（官方无同名命令）。调研见 `.agents/docs/reports/s5-checkup-investigation-2026-09.md` | — |

---

## 3. 已人工复核项（本次会话核过代码）

| 项 | 复核结果 | 证据 |
|---|---|---|
| `alias` 别名不可调用 | ✅ 确认：`src/cmd/alias.rs` 全文仅读/写 config 映射；全仓 `alias*` 消费点只有 `config.rs:1300/1309` 与 `mod.rs:179`（clap 子命令别名，与此无关） | `src/cmd/alias.rs:43-83` |
| `export` schema 与官方不兼容 | ✅ 确认：输出为 `{buckets:{name:url}, apps:{bucket:{name:version}}}` | `crates/libscoop/src/package/export.rs:28-61` |
| `aria2-enabled` 不调用 aria2 | ✅ 确认：`aria2_enabled/split/max_connection/min_split_size` 只喂给内置下载器的分片参数，全仓无 aria2 进程调用 | `crates/libscoop/src/package/download.rs:520-524` |
| `config` 键白名单 | ✅ 与技能记忆一致：未建模键 → `ConfigKeyInvalid`；`unsupported_keys_rejected` 测试锁定 12 个被拒键 | `crates/libscoop/src/config.rs:743`、`:1401-1424` |

---

## 4. 明细：安装 / 卸载生命周期

> 覆盖 `install` `uninstall` `update` `upgrade` `reinstall` `reset` `cleanup` `cache`。

### 4.1 cache

| 偏离 | hok 证据 | Scoop 证据 | 类别 | 处置 |
|---|---|---|---|---|
| `hok cache` 需要子命令（`list`/`rm`），裸 `cache`/`cache <app>` 是 clap 用法错误（exit 2）；官方 `scoop cache` / `cache <app>` 落到 default 分支直接展示缓存 | `src/cmd/cache.rs:19-22`、`src/cmd/mod.rs:92,101` | `libexec/scoop-cache.ps1:61-71` | 语义不同 | **已修（S6）**：`command` 改 `Option` + 顶层 `app` 位置参数，裸 `cache` / `cache <app>` 默认 list（exit 0），`show`/`list`/`rm`/`ls` 保留 |
| `cache rm` 只接受一个可选查询，`hok cache rm a b` 报错；官方把所有位置参数拼成一个正则逐个删 | `src/cmd/cache.rs:33-41` | `libexec/scoop-cache.ps1:36-46` | 语义不同 | **已修（S6）**：`Vec<String>` 多参数（`list` 同步） |
| hok 按大小写不敏感**子串**匹配，官方锚定文件名开头（`^$app#`）→ `hok cache rm zip` 会命中 `7zip`/`unzip` | `crates/libscoop/src/cache.rs:83-93` | `libexec/scoop-cache.ps1:28,45` | 语义不同 | **已修（S6）**：按 `package_name()` 大小写不敏感**全等**（等价官方锚定）；官方 regex 元字符语义不保留，登记为微偏离 |
| `cache rm` 无参：hok 打帮助并 exit 2；官方 `ERROR: <app(s)> missing` + usage，exit 1 | `src/cmd/cache.rs:34` | `libexec/scoop-cache.ps1:37-40` | 退出码不同 | **已修（S6）**：运行时 `<app(s)> missing` + exit 1（退出码层 S1 已改） |
| 按查询删除时 hok 不删 aria2 sidecar `$cachedir\<app>.txt`，官方一并删除 | `crates/libscoop/src/cache.rs:134-178` | `libexec/scoop-cache.ps1:53-55` | 行为缺失 | **已修（S6）**：按 app 删除后顺带删 `<app>.txt` |
| 输出不同：hok 一行汇总；官方逐文件 `Removing <file>...` + `Deleted: N files, <size>` | `src/cmd/cache.rs:96-117` | `libexec/scoop-cache.ps1:49-58` | 输出或文案不同 | **暂缓（S7）**：样式批次统一处理 |

### 4.2 cleanup

| 偏离 | hok 证据 | Scoop 证据 | 类别 | 处置 |
|---|---|---|---|---|
| 既无 app 也无 `-a/--all` 时 hok 清理**全部**已装应用；官方报 `ERROR: <app> missing` + usage，exit 1 | `src/cmd/cleanup.rs:29-35` | `libexec/scoop-cleanup.ps1:24` | 语义不同 | **已修（S6）**：裸 `cleanup` 报 `<app> missing` + exit 1（破坏性变更，已登记） |
| `cleanup -g` 无管理员检查；官方报 `you need admin rights to cleanup global apps`，exit 1 | `src/cmd/cleanup.rs:26-27` | `libexec/scoop-cleanup.ps1:26-28` | 行为缺失 | **已修（S6）**：走共享 `ensure_global` 门，文案逐字对齐、exit 1 |
| `cleanup -k` 清空**整个**下载缓存（`cache::remove("*")`），含当前版本缓存；官方只删 `$app#*` 且排除 `$app#$current_version#*` | `src/cmd/cleanup.rs:54-56` | `libexec/scoop-cleanup.ps1:32-33` | 语义不同 | **已修（S6）**：逐 app 删 `app#*` 保留当前版本，收尾清 `*.download`（`cache::remove_outdated` / `remove_incomplete`）。**2026-09-25 追加（`ff05b80`/`cb75ba4`）**：`app#*` 的匹配改用官方 glob 语义（原字符类漏掉含 `+`/`_` 的版本号）；坏安装（current 解析不出）的缓存也全删（官方 `-Exclude "$app#$null#*"` 匹配不到任何东西 ⇒ 该 app 缓存全清）；缓存删除量与大小由 CLI 打一行汇总（**hok 扩展，官方无此行**）——详见文末 2026-09-25 节 |
| `cleanup -a -g` 只清全局；官方 `-a -g` 清本地并追加全局 | `src/cmd/cleanup.rs:27-35` | `libexec/scoop-cleanup.ps1:67-70` | 边界处理不同 | **已修（S6）**：本地一趟 + 全局一趟，结果合并后打印一次收尾 |
| 命名应用无可清理时 hok 打印全局 "No old versions to clean up." 并静默跳过未安装应用；官方逐应用 `"$app is already clean"` / `"'$App' isn't installed."` | `src/cmd/cleanup.rs:46-47`、`crates/libscoop/src/package/cleanup.rs:47-51` | `libexec/scoop-cleanup.ps1:38-41`、`lib/core.ps1:1104,1113` | 输出或文案不同 | **已修（S6）**：命名 app 走完整 `Confirm-InstallationStatus`（`'x' isn't installed.` / 跨作用域 `--global` 双向提示 / `isn't installed correctly`，exit 0）；`Some(0)` 时不再多打"无旧版本"；逐应用 "is already clean" 文案归 S7 |

### 4.3 install

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| hok 从不自更新 Scoop 核心，也无 `-u/--no-update-scoop`；官方在 `is_scoop_outdated`（≥3h）时跑 `scoop-update.ps1`（或 `-u` 时告警） | `src/cmd/install.rs:21-74,94-103` | `libexec/scoop-install.ps1:27,66-72` | 有意偏离 |
| `--independent` 短选项：hok=`-I`，官方=`-i`（`hok install -i x` 被拒） | `src/cmd/install.rs:51` | `libexec/scoop-install.ps1:24,46` | 语义不同（**已修 update-align `c25e39c`**：改 `-i` + 官方描述） |
| hok 多出官方 getopt 会拒绝的开关：`-f/--ignore-failure`、`-d/--download-only`、`-o/--offline`、`-y/--assume-yes`、`-R/--no-replace`、`-U/--no-upgrade`（`-S/--escape-hold` 已删，见 §4.4/§4.5） | `src/cmd/install.rs:29-68` | `libexec/scoop-install.ps1:46-47` | 语义不同 |
| 已安装文案：hok `'<name>' is already installed. Skipping.`（无版本、无恢复提示）；官方带版本并提示 `Use 'scoop update $app'` | `src/cmd/install.rs:108-111`、`locales/en.yml:332` | `libexec/scoop-install.ps1:81-85` | 输出或文案不同 |
| ~~成功时 hok 额外打印 `Everything is ok!`~~（官方到 per-app 成功行结束）→ **2026-09-13 晚已删除该页脚，对齐官方** | ~~`src/cmd/install.rs:143`~~ | `lib/install.ps1:79`、`libexec/scoop-install.ps1:139` | ✅ 已修 |
| 缺 `<app>`：hok clap exit 2；官方 `'<app> missing'` + usage，exit 1 | `src/cmd/install.rs:20-24` | `libexec/scoop-install.ps1:60` | 退出码不同 |

### 4.4 uninstall

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| `scoop` 自身被 hok 的已装查询过滤，`hok uninstall scoop` 报 PackageNotFound；官方特判并跑 `bin/uninstall.ps1` | `crates/libscoop/src/package/query.rs:66-77`、`sync_remove.rs:49-58` | `libexec/scoop-uninstall.ps1:39-42` | 行为缺失 |
| held 包被 hok 静默跳过（除 `-S`）；官方 uninstall 无 hold 检查，照删 | `crates/libscoop/src/package/sync_remove.rs:61-63` | `libexec/scoop-uninstall.ps1`（无 hold 分支） | 语义不同（**已修 update-align `20575a5`**：跳过删除，held 包可直接卸载；换版本走 卸载 → `hok install <app>@版本` → `hok hold`） |
| hok 拒绝卸载被其他已装包依赖的应用（`PackageDependentFound`，exit 1）；官方无反向依赖检查 | `crates/libscoop/src/package/sync_remove.rs:67-104` | `libexec/scoop-uninstall.ps1:9-15`（未 source depends.ps1） | 语义不同 |
| 运行中进程：hok 中止整批（exit 1），官方 `continue` 跳过该应用继续，exit 0 | `crates/libscoop/src/package/sync_remove.rs:156-174,253` | `libexec/scoop-uninstall.ps1:65-67` | 语义不同 |
| 未安装应用：hok exit 1；官方 `'$App' isn't installed.` + 全局提示，exit 0 | `crates/libscoop/src/package/sync_remove.rs:49-58` | `libexec/scoop-uninstall.ps1:44-45`、`lib/core.ps1:1104-1114` | 退出码不同 |
| ~~成功时 hok 额外打印 `Everything is ok!`~~ → **2026-09-13 晚已删除该页脚，对齐官方** | ~~`src/cmd/uninstall.rs:71`~~ | `libexec/scoop-uninstall.ps1:147-150` | ✅ 已修 |
| 缺 `<app>`：hok clap exit 2；官方 exit 1 | `src/cmd/uninstall.rs:13` | `libexec/scoop-uninstall.ps1:28-32` | 退出码不同 |

### 4.5 update / upgrade / reinstall

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| 裸 `hok update` 只同步 bucket、不更新 Scoop 自身；`hok update scoop`/`update <app>` 也从不触发核心同步；官方 `scoop update` 跑 `Sync-Scoop`+`Sync-Bucket`，`update <app>` 在过期时同样先同步，`update scoop` 更新核心 | `src/cmd/update.rs:98-109`、`query.rs:66-77` | `libexec/scoop-update.ps1:388-400,407-416` | 行为缺失 |
| hok 在上次更新后 15 分钟内静默跳过 bucket 同步（除非 `--force`）；官方 `scoop update` 每次都同步（其 3h 只门控自更新检查） | `src/cmd/update.rs:113-117`、`crates/libscoop/src/config.rs:360-366` | `libexec/scoop-update.ps1:397-399`、`lib/core.ps1:1168-1178` | 有意偏离（**已修 update-align `c2581d4`**：冷却整体删除，`hok update` 每次同步；`-f` 收敛为官方「即使没有新版本也强制更新」） |
| hok `update` 无 `-q/--quiet`；官方用它隐藏"已是最新"等消息与版本箭头 | `src/cmd/update.rs:17-71` | `libexec/scoop-update.ps1:15,38,283-284` | 行为缺失 |
| `hok update -k/--no-cache` 无 app 时被接受并忽略；官方报 `--no-cache is invalid when <app> is not specified.`，exit 1 | `src/cmd/update.rs:102-108` | `libexec/scoop-update.ps1:393-396` | 退出码不同 |
| `--independent` 短选项同 install（hok=`-I`） | `src/cmd/update.rs:49` | `libexec/scoop-update.ps1:11,32` | 语义不同（**已修 update-align `c25e39c`**：改 `-i` + 官方描述） |
| `update <not-installed>`：hok exit 1；官方文案 + exit 0 | `src/cmd/update.rs:279-295`、`sync_install.rs:105-134` | `libexec/scoop-update.ps1:424-427`、`lib/core.ps1:1104` | 退出码不同 |
| 已是最新：hok 只打印 `All apps are up to date.`；官方逐应用 `$app: $version (latest version)` + 结尾提示 | `src/cmd/update.rs:292-294`、`locales/en.yml:176` | `libexec/scoop-update.ps1:439-446,456-457` | 输出或文案不同 |
| 运行中进程：hok 中止整批 exit 1；官方跳过该应用继续，exit 0 | `sync_install.rs:442-467` | `libexec/scoop-update.ps1:296-301` | 语义不同 |
| `update -f` 同版本重装：hok 直接装进现有版本目录，不移动旧目录；官方先把旧目录改名 `_$version.old`（`_$version.old(i)`），因此 hok 会保留陈旧文件、无备份目录 | `sync_install.rs:564-567` | `libexec/scoop-update.ps1:355-365` | 行为缺失 |
| `-g -a`/`-g *`：hok 只升全局；官方 `update * -g` 升本地并追加全局 | `src/cmd/update.rs:94-95,259-264` | `libexec/scoop-update.ps1:418-422` | 边界处理不同 |
| ~~成功时 hok 打印 `Everything is ok!`~~（官方打印 `Updating N outdated apps:` + 逐应用行）→ **2026-09-13 晚已删除该页脚** | ~~`src/cmd/update.rs:310`~~ | `libexec/scoop-update.ps1:454-465` | ✅ 已修 |
| 升级引擎 `execute_upgrade` 住在 `update.rs`，`upgrade.rs` 只是薄壳转调 | `src/cmd/update.rs:259-338`（改动前） | 官方无 `upgrade` 命令，`update` 是唯一入口 | 结构（**update-align `ab7ea6e`**：引擎搬入 `upgrade.rs`，`update` 反向引用，方向 update → upgrade） |
| `upgrade` 的 flag 集比 `update` 少 `-i/--independent`、`-k/--no-cache`（在 `sync_args` 里硬编码 `false`） | `src/cmd/upgrade.rs:51-55`（改动前） | `libexec/scoop-update.ps1:378-385,393`（`$independent`/`$use_cache` 确实进升级路径） | 语义不同（**已修 update-align `ba6f923`**：补 `-i`/`-k`；`-U`/`-R` 有意不给——`-U` 与引擎强制的 `OnlyUpgrade` 相抵） |
| install 路径对已装且 held 的包**静默跳过**（`continue`，返回前端空集）；官方 install 无 hold 检查（会照装/照升） | `crates/libscoop/src/package/sync_install.rs:232-238`（改动前） | `libexec/scoop-install.ps1:46-47`（无 hold 分支） | 有意偏离（**update-align `20575a5`**：改为可见警告 `cmd.held_install_skip`，点名被锁版本并提示 `hok unhold`；门本身保留——`hok install <已装包>` 实质是升级，属 hold 语义范围） |
| `--escape-hold`（install/update/upgrade/uninstall 四处 `-S`）可绕过 hold；官方全仓库无此开关，绕过只能靠 `hok unhold` | `src/cmd/{install,update,upgrade,uninstall}.rs`（改动前） | `libexec/scoop-update.ps1:432-438`（唯一闸门）、`scoop-alias.ps1:14` | 有意偏离（**update-align `20575a5` 删除**：CLI flag + `SyncArgs` 字段 + `SyncOption::EscapeHold` + 4 个 i18n 键全删；`resolve_cascade` 的 `escape_hold` 形参一并去掉，held 依赖恒报 `PackageCascadeRemoveHold`） |
| 官方无 `upgrade` 命令（`scoop upgrade` → `'upgrade' isn't a scoop command`，exit 1）；hok 实装 `upgrade`（官方仅在 alias 示例里出现） | `src/cmd/upgrade.rs:56-69` | `lib/commands.ps1:5-16`、`bin/scoop.ps1:49-52`、`libexec/scoop-alias.ps1:14` | 有意偏离 |
| `hok upgrade -f` = ignore-failure 且无 force 选项；等价官方路径 `scoop update -f` 的 `-f` = force | `src/cmd/upgrade.rs:11-36` | `libexec/scoop-update.ps1:9,35` | 语义不同 |
| 官方无 `reinstall` 命令（`scoop reinstall x` → `'reinstall' isn't a scoop command`）；hok 实装为 uninstall+install 并处理 hold | `src/cmd/reinstall.rs:57-161` | `lib/commands.ps1:5-16`、`bin/scoop.ps1:49-52` | 有意偏离 |

### 4.6 reset

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| `reset <app>` 额外跑 manifest 的 `post_install`；官方只重建 current/shim/快捷方式/env/persist | `sync_remove.rs:622-640` | `libexec/scoop-reset.ps1:80-91` | 有意偏离 |
| hok 不自动识别全局安装，`hok reset <global-app>` 按用户目录解析后失败；官方 `installed $app $true` 时置 `$global` | `src/cmd/reset.rs:82-84`、`sync_remove.rs:684-697` | `libexec/scoop-reset.ps1:33-36` | 行为缺失 |
| 单应用路径无 admin 检查；官方告警 `'$app' ($version) is a global app. You need admin rights to reset it. Skipping.`（hok 仅 `--all` 告警） | `src/cmd/reset.rs:82-84` | `libexec/scoop-reset.ps1:60-63` | 行为缺失 |
| hok 用第二个位置参数表示版本，不解析 `app@version` / `bucket/app`；官方经 `parse_app` 拆分 | `src/cmd/reset.rs:11-14`、`sync_remove.rs:687-701` | `libexec/scoop-reset.ps1:31`、`lib/core.ps1:1150-1156` | 语义不同 |
| `reset <未安装>`：hok 返回 PackageNotFound（exit 1）；官方文案 + exit 0 | `sync_remove.rs:527-539` | `libexec/scoop-reset.ps1:43-46` | 退出码不同 |
| 官方无本地/全局自动判定（同上），hok 亦无 → 组合影响：`hok reset --all` 之外无法重置全局应用 | `src/cmd/reset.rs:82-84` | `libexec/scoop-reset.ps1:33-36,60-70` | 行为缺失 |

## 5. 明细：查询 / 报表

> 覆盖 `list` `search` `status` `info` `depends` `which` `prefix` `cat` `home`。
> 注：`info`/`status`/`list` 已于 2026-09 多轮对齐，以下为对齐后仍在的偏离。

### 5.1 list

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| hok 只列本地作用域且**无作用域开关**，全局安装的应用不会出现；官方合并本地+全局 | `src/cmd/list.rs:22-38`、`query.rs:47` | `libexec/scoop-list.ps1:15-18` | 行为缺失 |
| hok 只渲染 Name/Version/Source/Info；官方另有 `Updated` 列与 `Global install`/架构字样 | `src/cmd/list.rs:41-50`、`locales/en.yml:85-88` | `libexec/scoop-list.ps1:48-57`、`supporting/formats/ScoopTypes.Format.ps1xml:22-25` | 行为缺失 |
| URL/本地路径安装的 Source：hok 显示 `__isolated__`；官方显示记录的 url，或在等于用户 manifest 时显示 `<auto-generated>` | `query.rs:127`、`status.rs:106-110` | `libexec/scoop-list.ps1:42-47` | 输出或文案不同 |
| hok 复用 `status` 的措辞（`Deprecated`/`Manifest removed`/`Held package`）；官方 list 用 `Deprecated package`/`Global install`/`Install failed`/`Held package` | `src/cmd/mod.rs:62-81`、`locales/en.yml:167-172` | `libexec/scoop-list.ps1:51-57` | 输出或文案不同 |
| 空结果：hok `No installed app matched.` exit 0；官方 `There aren't any apps installed.` exit 1 | `src/cmd/list.rs:153-156`、`locales/en.yml:92` | `libexec/scoop-list.ps1:19-22` | 退出码不同 |
| hok 接受多个查询并 OR（含 `bucket/` 前缀解析）；官方只把第一个位置参数当名字正则，多余词静默忽略 | `src/cmd/list.rs:24-25`、`query.rs:510-516` | `libexec/scoop-list.ps1:4,26`、`lib/commands.ps1:38` | 语义不同 |
| hok 在 Info 里追加过期的 `→ <new>` 标记并打印计数页脚；官方 list 无任何更新指示或汇总 | `src/cmd/list.rs:114-117,160-176` | `libexec/scoop-list.ps1:50-58` | 输出或文案不同 |
| 标题：hok 固定 `Installed apps`；官方回显查询 `Installed apps matching '<query>':` | `src/cmd/list.rs:151`、`locales/en.yml:84` | `libexec/scoop-list.ps1:25` | 输出或文案不同 |
| hok 有官方不存在的 `-V/--versions` 扩展 | `src/cmd/list.rs`（Args） | `libexec/scoop-list.ps1`（无此参数） | 语义不同 |

### 5.2 search

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| hok 只有加 `-B/--with-binary` 才匹配二进制名；官方默认就会回退匹配 bin 名/别名，默认结果集不同 | `src/cmd/search.rs:24`、`query_matcher.rs:159-167` | `libexec/scoop-search.ps1:70-87,17-29` | 语义不同 |
| 无查询：hok 用法错误；官方构造空正则列出全部可用应用 | `src/cmd/search.rs:15,18` | `libexec/scoop-search.ps1:8-9,174-177` | 边界处理不同 |
| hok 对多词 OR；官方只用第一个词（其余落入 `$args` 被忽略） | `src/cmd/search.rs:18-19,42`、`query.rs:510-516` | `libexec/scoop-search.ps1:9`、`lib/commands.ps1:38` | 语义不同 |
| 输出：hok `name/bucket version [installed]`；官方 `Results from local buckets...` 表头 + Name/Version/Source/Binaries 表且无已装标记 | `src/cmd/search.rs:44-76` | `libexec/scoop-search.ps1:190-193` | 输出或文案不同 |
| 无匹配：hok 静默 exit 0；官方 warn `No matches found.` exit 1 | `src/cmd/search.rs:44-77` | `libexec/scoop-search.ps1:195-200` | 退出码不同 |
| hok 不查询 known-but-unadded bucket 的远程结果；官方走 GitHub tree API 并输出 `Results from other known buckets...` | `src/cmd/search.rs:42-76` | `libexec/scoop-search.ps1:120-158,195-202` | 行为缺失 |

### 5.3 status

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| hok 无 `-l/--local`（改提供 `-a/--all`），`hok status -l` 报参数错误；官方用它关闭远程检查 | `src/cmd/status.rs:13-17` | `libexec/scoop-status.ps1:4-5,16-17` | 行为缺失 |
| hok 不做 Scoop/bucket 新鲜度检查，缺 `Scoop out of date`/`Scoop bucket(s) out of date`/`Scoop is up to date.` | `src/cmd/status.rs:150-151` | `libexec/scoop-status.ps1:23-52` | 行为缺失 |
| hok 多一列 `Source`（六列）；官方表为 Name/Installed Version/Latest Version/Missing Dependencies/Info（五列） | `src/cmd/status.rs:37-45`、`locales/en.yml:160-165` | `supporting/formats/ScoopTypes.Format.ps1xml:72-86` | 输出或文案不同 |
| 缺失依赖：hok 用 ` · ` 连接、列名 `Missing deps`；官方用竖线 ` \| ` 连接、列名 `Missing Dependencies` | `src/cmd/status.rs:180`、`locales/en.yml:164` | `libexec/scoop-status.ps1:68` | 输出或文案不同 |
| 一切正常：hok `All apps are up to date.`；官方 `Everything is ok!` | `src/cmd/status.rs:161`、`locales/en.yml:158` | `libexec/scoop-status.ps1:79-81` | 输出或文案不同 |
| hok 额外打印 attention 汇总页脚；官方无页脚 | `src/cmd/status.rs:186-213` | `libexec/scoop-status.ps1:79-83` | 输出或文案不同 |
| hok 额外信息词 `Source bucket missing`/`Source unknown`，并把清单被删复用为 `Manifest removed`；官方只发 Install failed/Held package/Deprecated/Manifest removed | `src/cmd/mod.rs:71-79`、`locales/en.yml:169-172` | `libexec/scoop-status.ps1:70-74` | 输出或文案不同 |

### 5.4 info

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| 单元素 `bin` 数组：hok 渲染为文件路径；官方渲染 `<null-alias>.<ext>`（如 `.exe`） | `crates/libscoop/src/package/info.rs:15-19,828-837` | `libexec/scoop-info.ps1:233-237` | 有意偏离 |
| `shortcuts` 裸字符串项：hok 整串渲染；官方取 `$_[1]` 输出该串第二个字符 | `info.rs:17-19,841-847` | `libexec/scoop-info.ps1:244-247` | 有意偏离 |
| `Download size` 探测架构：hok 用配置/默认架构；官方用 `Get-DefaultArchitecture` 宿主架构 | `info.rs:20-22` | `libexec/scoop-info.ps1:201` | 有意偏离 |
| 已安装的 deprecated 应用且已装版本 ≠ 清单版本时：hok 打 `<installed> (Update to <latest> available)`；官方 if/elseif 链落到纯清单版本 | `info.rs:390-404` | `libexec/scoop-info.ps1:66-75` | 语义不同 |
| 多 bucket 命中且均未安装：hok 只列候选（不显示字段）；官方描述第一个命中 bucket 的清单并只告警歧义 | `src/cmd/info.rs:314-325` | `libexec/manifest.ps1:81-92,107-109` | 语义不同 |
| `-v` 是 hok 的全局 verbosity 计数，`hok -v info x` 也会开启详细视图；官方 `-v` 是 info 局部参数 | `src/cmd/info.rs:25-29`、`src/cmd/mod.rs:105-106` | `libexec/scoop-info.ps1:11` | 有意偏离 |

### 5.5 depends

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| 输出形态：hok 缩进前序树且首行为被查询应用；官方为平铺列表（依赖解析序：最深层在前、应用本身在末）带 Source/Name 列 | `src/cmd/depends.rs:65-86`、`depends.rs:52-77` | `libexec/scoop-depends.ps1:22-35`、`lib/depends.ps1:56-65` | 语义不同 |
| 重复可达的依赖：hok 重列并标 `(already listed)`；官方 `$Resolved` 检查只列一次 | `crates/libscoop/src/package/depends.rs:52-61`、`locales/en.yml:262` | `lib/depends.ps1:47-54` | 输出或文案不同 |
| 环形依赖：官方 `Circular dependency detected: ...` 直接中止；hok 在重复节点静默停止 | `depends.rs:52-61` | `lib/depends.ps1:49-51` | 边界处理不同 |
| 依赖清单找不到：hok 静默丢弃（依赖列为空）；官方告警未添加的 bucket 并中止 | `depends.rs:70` | `lib/depends.ps1:38-43` | 边界处理不同 |
| hok 总是附上安装 helper（7zip/lessmsi/innounp/dark）；官方 `Get-InstallationHelper` 会剔除已安装的 helper | `manifest.rs:1246-1279` | `lib/depends.ps1:121-127` | 语义不同 |
| 见到 `.msi` URL：hok 无条件加 `lessmsi`；官方仅在 `USE_LESSMSI` 配置开启时加（默认关） | `manifest.rs:1252-1257,1274-1280` | `lib/depends.ps1:112` | 默认值不同 |
| 无 bucket 前缀的依赖：hok 继承父应用 bucket；官方经 `Get-Manifest` 解析各自真实 bucket（或 url） | `depends.rs:73-76` | `libexec/scoop-depends.ps1:26-31` | 语义不同 |
| hok 依赖集合用 `HashSet`，不保留声明顺序；官方 `Select-Object -Unique` 保序 | `manifest.rs:1240` | `lib/depends.ps1:45` | 语义不同 |

### 5.6 which

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| hok 打印 shim 文件自身（`shims\<cmd>.exe`）；官方解析出 shim 指向的**真实目标** | `src/cmd/which.rs:21-25`、`shim.rs:43-52` | `libexec/scoop-which.ps1:12-19`、`lib/core.ps1:518-530,854-866` | 语义不同 |
| hok 打印全部匹配变体（无扩展名/`.cmd`/`.ps1`/`.exe`）；官方只打印一个解析结果 | `shim.rs:46-51` | `lib/core.ps1:518-530` | 输出或文案不同 |
| hok 只查本地 shims 目录，PATH 上的非 Scoop 程序或 Scoop 别名一律 exit 2；官方可解析 PATH 程序与别名 | `src/cmd/which.rs:21-37`、`session.rs:391` | `lib/core.ps1:523-527` | 行为缺失 |
| hok 不查全局 shim 目录；官方同时查用户与全局 shim 目录 | `src/cmd/which.rs:21`、`session.rs:391` | `lib/core.ps1:508-521` | 行为缺失 |

### 5.7 prefix / cat / home

| 命令 | 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|---|
| prefix | hok 只解析本地安装，否则报错；官方先回退全局应用目录（`currentdir $app $true`） | `src/cmd/prefix.rs:20-31` | `libexec/scoop-prefix.ps1:12-21` | 行为缺失 |
| prefix | hok 有两条不同文案（无 `current` 符号链接 / 未安装）；官方统一 `Could not find app path for '<app>'.` | `src/cmd/prefix.rs:25-30`、`locales/en.yml:177-178` | `libexec/scoop-prefix.ps1:20` | 输出或文案不同 |
| cat | hok 只按精确名解析 bucket 清单；官方 `Get-Manifest` 还接受 manifest URL/UNC/本地文件，并回退已安装/deprecated 清单 | `src/cmd/cat.rs:60-65` | `libexec/scoop-cat.ps1:16`、`lib/manifest.ps1:38-41,52-75,95-99` | 行为缺失 |
| cat | 多 bucket 命中：hok 交互式询问；官方取第一个匹配并只告警 | `src/cmd/cat.rs:74-87` | `lib/manifest.ps1:81-92,107-109` | 语义不同 |
| cat | 美化：hok 用 serde（2 空格缩进、LF、保留单元素数组）；官方 `ConvertToPrettyJson` 用 4 空格、CRLF 并折叠单元素数组/多行字符串 | `src/cmd/cat.rs:35-36` | `lib/json.ps1:27,29,184-194` | 输出或文案不同 |
| cat | `bat` 缺失时 hok 静默回退纯 JSON；官方无条件调用 `bat`，缺失即失败 | `src/cmd/cat.rs:96-101` | `libexec/scoop-cat.ps1:19-24` | 边界处理不同 |
| home | hok 只按精确名解析 bucket 清单；官方 `Get-Manifest` 还接受 URL/UNC/本地文件 | `src/cmd/home.rs:19-21` | `libexec/scoop-home.ps1:10`、`lib/manifest.ps1:38-41` | 行为缺失 |
| home | 多 bucket 命中：hok 交互式让用户选序号；官方按 bucket 顺序静默取第一个 | `src/cmd/home.rs:35-52` | `lib/manifest.ps1:81-92` | 语义不同 |
| home | 清单无 `homepage`：hok 把空串交给 ShellExecuteW 得到底层 IO 错误；官方 `Could not find homepage in manifest for '<app>'.` | `src/cmd/home.rs:32-33`、`manifest.rs:919-921`、`internal/os.rs:417-438` | `libexec/scoop-home.ps1:12-16` | 边界处理不同 |
| home | 无参数：hok clap 帮助 exit 2；官方 `my_usage` 后 exit 1 | `src/cmd/home.rs:10` | `libexec/scoop-home.ps1:20-23` | 退出码不同 |

---

## 6. 明细：bucket / config / 元数据

> 覆盖 `bucket` `config` `export` `import` `alias` `hold` `unhold` `shim` `create`。

### 6.1 alias

> **状态（2026-09-13，S2）**：全部 7 条已处置，提交 `3c90095`（`fix/alias-availability`）。
> 方案 = 官方式脚本别名：`add` 写 `<shims>\scoop-<name>.ps1`（`# Summary:` + 正文，UTF-8 无 BOM / LF / 末尾 CRLF，与官方 `Out-UTF8File` 逐字节一致）并在 config 记 `名字 -> "scoop-<名字>"`；
> 分发按官方语义**只查文件**（`<shims>\scoop-<token>.ps1` 存在即执行，pwsh 跑脚本、参数进 `$args`、退出码透传）。
> 第 7 行有更正：改动前 `alias rm nope` 不是"打印错误"，而是**静默成功并误报"已删除"**（实测）。
> 新逻辑落在 `crates/libscoop/src/alias.rs`（唯一入口），`config::alias_add`/`alias_remove` 已删除。
> 详见 `s2-alias-investigation-2026-09.md` §7。

| 偏离 | hok 证据 | Scoop 证据 | 类别 | 状态 |
|---|---|---|---|---|
| **hok 只把别名写进 config 的 alias 映射、从不生成可执行脚本，别名无法作为命令调用**；官方写 `shims\scoop-<name>.ps1` | `crates/libscoop/src/config.rs:1300-1306`、`src/cmd/alias.rs:43-83` | `lib/commands.ps1:57-74` | 语义不同 | **已修**：脚本落盘 + `hok <别名>` 可调用 |
| 无子命令时 hok 默认 list；官方报 `<subcommand> missing` exit 1 | `src/cmd/alias.rs:18,46` | `libexec/scoop-alias.ps1:32-40` | 语义不同 | **有意偏离**（保留便利语义，已登记） |
| `alias add` 无可选 description（官方写入脚本首行 `# Summary:`） | `src/cmd/alias.rs:28-33` | `libexec/scoop-alias.ps1:9,45`、`lib/commands.ps1:43-50,62-65` | 行为缺失 | **已修** |
| `alias list` 无 `-v/--verbose`（官方 `-v` 加显示 Summary 列） | `src/cmd/alias.rs:24` | `libexec/scoop-alias.ps1:42-46`、`lib/commands.ps1:124-127` | 行为缺失 | **已修**（复用全局 verbosity，同 `info -v` 约定） |
| 列表把命令截断到 60 字符且不识别脚本缺失；官方输出完整命令/Summary，脚本丢失显示 `<BROKEN>` | `src/cmd/alias.rs:55-60` | `lib/commands.ps1:96-127` | 输出或文案不同 | **已修**（完整命令 + `<BROKEN>` + 表格） |
| 别名已存在：hok 静默覆盖；官方 `abort "Alias 'x' already exists."` | `config.rs:429-431` | `lib/commands.ps1:52-55` | 边界处理不同 | **已修**（报错 exit 1；另加脚本占用与内建重名两条守卫） |
| `alias rm` 删除不存在的别名：hok 打印错误但返回 Ok → exit 0；官方 `abort` exit 1 | `src/cmd/alias.rs:75-80`、`src/lib.rs:139-148` | `lib/commands.ps1:84-86`、`lib/core.ps1:293` | 退出码不同 | **已修**（报错 exit 1；原文"打印错误"应更正为"静默成功+误报已删除"） |

### 6.2 bucket

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| 无已添加 bucket 时 hok 打印空表 exit 0；官方 warn `No bucket found...` 并 `exit 2` | `src/cmd/bucket.rs:70-88` | `libexec/scoop-bucket.ps1:61-70` | 退出码不同 |
| 排序：hok 纯字母序；官方把已知 bucket 按 `buckets.json` 顺序提前，其余按目录序 | `crates/libscoop/src/bucket.rs:462-467` | `lib/buckets.ps1:56-75` | 输出或文案不同 |
| 列顺序：hok Name/Manifests/Updated/Source；官方对象序 Name/Source/Updated/Manifests | `src/cmd/bucket.rs:75-83` | `lib/buckets.ps1:107-118` | 输出或文案不同 |
| `bucket known`：hok 打表头+两列表格；官方输出裸名称列表 | `src/cmd/bucket.rs:93-96,228-236` | `libexec/scoop-bucket.ps1:71-74`、`lib/buckets.ps1:48-50` | 输出或文案不同 |
| hok `list -k` 改为列出已知 bucket；官方忽略该多余参数始终列已添加 bucket | `src/cmd/bucket.rs:36-41,65-69` | `libexec/scoop-bucket.ps1:61-70` | 语义不同 |
| `bucket rm` 接受多个名字逐个删；官方形参只有 `$name`，多余参数被忽略 | `src/cmd/bucket.rs:45-51,97-106` | `libexec/scoop-bucket.ps1:20,52-59` | 语义不同 |
| 已知 bucket 名匹配：hok 大小写敏感（`add Extras` 报缺 repo）；官方属性查找大小写不敏感 | `bucket.rs:439-443` | `lib/buckets.ps1:43-46` | 边界处理不同 |
| 重复仓库检测：hok 只查目标目录是否存在；官方归一化比较所有已添加 bucket 的 remote URL，重复则 warning + exit 2 | `bucket.rs:432-435` | `lib/buckets.ps1:139-147` | 行为缺失 |
| bucket 已存在：hok 报错 exit 1；官方 warn 后 `return 2`（exit 2） | `bucket.rs:433-435`、`src/lib.rs:147-148` | `lib/buckets.ps1:130-133,49-50` | 退出码不同 |
| 添加流程输出：hok 直接 clone 并打印 `Adding bucket`/ok；官方先 `Checking repo... ` + `git ls-remote` 预检再打 OK | `src/cmd/bucket.rs:56-63`、`bucket.rs:446-449` | `lib/buckets.ps1:149-168` | 输出或文案不同 |
| git URL 校验：hok 不校验/归一化，原样交给 libgit2；官方 `Convert-RepositoryUri` 校验并报 `is not a valid Git URL!` | `bucket.rs:437-449` | `lib/buckets.ps1:83-102,135-138` | 边界处理不同 |

### 6.3 config

> 2026-09-13 落地（会话 S4，`fix/config-key-and-subcommand` → 已线性进 `refactor/edition-2024`）：
> 键名严格化（只认官方拼写、砍掉 hok 别名）、报错分流（官方独有键 → 新 `ConfigKeyUnsupported`，
> 非设置项 → `ConfigKeyInvalid`）、子命令对齐（保留 `edit` 为 hok 扩展、`rm` 用官方名；
> `set`/`unset`/`list` 删除，写入改 `hok config <key> <value>`）、输出三档（裸 `config` = 文件
> 里保存的键 `key = value`；`-j` = 同一份 JSON 且路径提示走 stderr；`-a` = 全键表格）。
> S12 的 `ignore-failures` 默认值显示随之修正。详见 `s4-config-investigation-2026-09.md`。
>
> 逐条处置：键白名单 → **保持严格**（改分流报错）；官方键缺失 → 登记有意偏离（15 键）；
> `aria2-min-split-size` 默认值 → **暂缓**；`unset` vs 官方 `rm` → **已修**（只留 `rm`）；
> 无单键读取 → **已修**（`hok config <name>`）；裸 `config` 用法错 → **已修**（`key = value` 行）；
> `config list` 输出 → **已修**（`-j` 纯 JSON，路径转 stderr）；配置文件路径 → 有意偏离；
> `config edit` → **保留为 hok 扩展**（有意偏离）；`proxy none` 语义 → **暂缓**。

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| **键白名单化**：任何未建模键 → `ConfigKeyInvalid`；官方 `set_config` 无校验、任意键都写 | `crates/libscoop/src/config.rs:743` | `lib/core.ps1:105-138` | 语义不同 |
| hok 不支持官方这些键：`use_lessmsi`/`use_external_7zip`/`show_manifest`/`show_update_log`/`update_nightly`/`hold_update_until`/`autostash_on_conflict`/`debug`/`force_update`/`scoop_repo`/`scoop_branch`/`shim`/`aria2-retry-wait`/`aria2-warning-enabled`/`aria2-options` | `config.rs:154-301` | `libexec/scoop-config.ps1:24-153` | 行为缺失 |
| `aria2-min-split-size` 默认值 hok 20M，官方 5M | `config.rs:554-555,1079` | `lib/download.ps1:350` | 默认值不同 |
| 删除子命令：hok `unset`，官方 `scoop config rm <name>` | `src/cmd/config.rs:37-42` | `libexec/scoop-config.ps1:161-163` | 语义不同 |
| hok 无单键读取；官方 `scoop config <name>` 输出值或 `'<name>' is not set` | `src/cmd/config.rs:18-43` | `libexec/scoop-config.ps1:167-177` | 行为缺失 |
| 裸 `config`：hok clap 错误 exit 2；官方打印整个配置 exit 0 | `src/cmd/config.rs:13-16` | `libexec/scoop-config.ps1:157-158` | 语义不同 |
| `config list`：hok 先打配置文件路径再打仅含受支持键的 JSON；官方直接输出 `$scoopConfig`（全部键） | `src/cmd/config.rs:73-77` | `libexec/scoop-config.ps1:158` | 输出或文案不同 |
| hok 读写 `~/.config/hok/config.json`，官方 `~/.config/scoop/config.json`（已登记） | `config.rs:915-922` | `lib/core.ps1:1263-1264` | 有意偏离 |
| `config edit`：hok 打开编辑器；官方把 `edit` 当键名输出 `'edit' is not set` | `src/cmd/config.rs:20-21,47-54` | `libexec/scoop-config.ps1:167-169` | 语义不同 |
| `config set proxy none`：hok 直接删键（视为未设置）；官方写入字符串 `none`（绕过系统代理直连） | `config.rs:664-667` | `libexec/scoop-config.ps1:50-53`、`lib/core.ps1:119-130` | 语义不同 |

### 6.4 export / import

> 2026-09-13 落地（会话 S3，`fix/export-import-interop` → 已线性进 `refactor/edition-2024`）：
> 只保留**一种** schema（官方形状），hok 自有内容放官方会忽略的两处——顶层 `hok` 命名空间
> （hok 自身配置 + 别名命令）与 app 项内的 `Hok` 对象（Hold/Global/Architecture/Failed）；
> `hok export` 默认输出纯官方 Scoopfile，`--hok` 追加扩展段。`import` 读官方数组与 hok 旧 map
> 两种形状，只导入兼容子集并逐条报告原因。另修掉一个阻断项：退出时无条件写 `ESC[?25h`
> 让 `hok export > f.json` 连自己都解析不了（现仅在终端恢复光标）。
> 详见 `s3-export-import-investigation-2026-09.md` §9–§11。

| 命令 | 偏离 | hok 证据 | Scoop 证据 | 类别 | 状态 |
|---|---|---|---|---|---|
| export | **schema 完全不同**：hok `{buckets:{名:url}, apps:{bucket:{名:版本}}}`；官方 buckets 是含 `Name/Source/Updated/Manifests` 的数组、apps 是含 `Name/Version/Source/Updated/Info` 的数组 | `crates/libscoop/src/package/export.rs:28-61` | `libexec/scoop-export.ps1:8-21`、`libexec/scoop-list.ps1:29-63` | 语义不同 | **已修**：输出官方数组形状（时间戳用官方 `.NET` round-trip 形状） |
| export | hok 无 `-c/--config`（官方导出配置并剔除 last_update/root_path/global_path/cache_path/alias） | `src/cmd/export.rs:10-14` | `libexec/scoop-export.ps1:10-16` | 行为缺失 | **已修（S4）**：`-c/--config` 输出官方键名的 `config` 段（两配置共享的键，机器相关键剔除），与 `import` 共用 `config::{map_official_key, official_name}` 单入口 |
| export | hok 默认跳过非 bucket（URL/本地）安装，需 `-a`；官方无条件导出全部已安装应用 | `export.rs:46-49` | `libexec/scoop-export.ps1:19` | 默认值不同 | **已修**：`-a` 删除，isolated 无条件导出（`Source`=清单 URL，用户清单记 `<auto-generated>`） |
| export | hok 的 buckets 映射只收录有 remote 的 bucket，本地非 git bucket 被丢弃；官方用本地路径作为 Source 列出 | `export.rs:34-38` | `lib/buckets.ps1:112-115` | 行为缺失 | **已修**：`Bucket::source()`（无 remote 用本地路径），`Updated` 取 HEAD 日期/目录时间 |
| import | hok 只接受本地文件路径；官方还接受 http(s) URL 与 UNC | `src/cmd/import.rs:17-26` | `libexec/scoop-import.ps1:18-22` | 行为缺失 | **已修**：本地路径 / http(s) URL / UNC |
| import | hok 完全忽略 `config` 段；官方逐键 `set_config` 并打印 `'k' has been set to 'v'` | `import.rs:32-51` | `libexec/scoop-import.ps1:26-29` | 行为缺失 | **已修**：`hok.config` 原样还原（别名走 alias API）；官方键映射到 hok 键名，不支持/机器相关键逐条报告 |
| import | hok 不添加 bucket，且把 `buckets` 键误当 bucket→apps 映射解析 | `import.rs:36-49` | `libexec/scoop-import.ps1:31-34` | 语义不同 | **已修**：逐条 `bucket add`（git 远程；空 source 回落内建表），已存在/非 git 本地路径报告跳过 |
| import | hok 忽略每个 app 的 Info（Global install/架构/`Held package`）；官方据此传 `--global`/`--arch` 并在装后重新 hold | `src/cmd/import.rs:42-47` | `libexec/scoop-import.ps1:36-64` | 行为缺失 | **已修**：global/arch 按 (global,arch) 分组安装，held 装后 `hold`；扩展段 `Hok` 优先于 `Info` 解析 |
| import | **官方 Scoopfile 导不进来**：hok 期望 `apps` 是对象，官方是对象数组 → `as_object()` 得 None | `import.rs:37-44` | `libexec/scoop-list.ps1:29-63` | 语义不同 | **已修**：官方数组 + hok 旧 map（含最旧的 `buckets` 嵌套变体）双形状解析 |
| import | hok 固定以 `AssumeYes`+`IgnoreFailure` 恢复；官方调用普通 install | `src/cmd/import.rs:45` | `libexec/scoop-import.ps1:60` | 语义不同 | **保持**（有意偏离，已登记）：非交互恢复；同时改为按"实际已安装"统计，结尾汇总 `已导入/未安装/跳过`，退出码 0（同 S1 批量口径） |

### 6.5 hold / unhold / shim / create

| 命令 | 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|---|
| hold | hok 无 `scoop` 自身特例，`hok hold scoop` 走包路径报"未安装"；官方把 `hold_update_until` 设为 1 天后并成功提示 | `crates/libscoop/src/package/hold.rs:35-39`、`src/cmd/hold.rs:78-81` | `libexec/scoop-hold.ps1:34-39` | 行为缺失 |
| unhold | hok 无 `scoop` 自身特例，`hok unhold scoop` 报"未安装"；官方清除 `hold_update_until` | `src/cmd/unhold.rs:25-28`、`hold.rs:35-39` | `libexec/scoop-unhold.ps1:35-39` | 行为缺失 |
| unhold | 取消 hold 后 hok 写 `"hold": false`；官方写 `"hold": null` | `crates/libscoop/src/package/manifest.rs:1533-1535` | `libexec/scoop-unhold.ps1:66` | 输出或文案不同 |
| shim | 无子命令时 hok 默认 list；官方报 `<subcommand> missing` exit 1 | `src/cmd/shim.rs:15,58` | `libexec/scoop-shim.ps1:40-48` | 语义不同 |
| shim | hok 无 `alter` 子命令（用户决策不做，已登记） | `src/cmd/shim.rs:19-49` | `libexec/scoop-shim.ps1:40,185-237` | 有意偏离 |
| shim | `-g` 位置相反：hok 的 `-g` 属顶层 `shim` 且必须在子命令前（`shim list -g` 报错）；官方 getopt 允许 `-g` 出现在 `--` 前任意位置 | `src/cmd/shim.rs:10-16,24-29` | `bin/scoop.ps1:9,41-47`、`lib/getopt.ps1:27-58` | 语义不同 |
| shim | `shim add` 不检查目标是否存在照样生成；官方目标不存在报 `Command path does not exist` exit 3 | `crates/libscoop/src/shim.rs:397-424` | `libexec/scoop-shim.ps1:114-123` | 边界处理不同 |
| shim | `shim rm` 有名字未找到 hok exit 1；官方 `ERROR: Local/Global shim not found:` exit 3 | `src/cmd/shim.rs:79-87` | `libexec/scoop-shim.ps1:125-141` | 退出码不同 |
| shim | `shim info` 未找到 hok exit 1；官方 exit 3，另一作用域存在时 exit 2 并提示 `--global` | `src/cmd/shim.rs:118-123` | `libexec/scoop-shim.ps1:169-184` | 退出码不同 |
| shim | `shim list -g`：hok 仍合并列出 local+global；官方只列 global | `src/cmd/shim.rs:54-56,140-152` | `libexec/scoop-shim.ps1:155-166` | 语义不同 |
| shim | `shim list` 只打缩进裸名字；官方输出含 Name/Source/Path/Type/Alternatives/IsGlobal/IsHidden 的对象 | `src/cmd/shim.rs:111-116` | `libexec/scoop-shim.ps1:68-81,167` | 输出或文案不同 |
| shim | `shim info` 只输出名字+扩展/类型/路径；官方还输出 Source/Alternatives/IsGlobal/IsHidden | `src/cmd/shim.rs:124-131` | `libexec/scoop-shim.ps1:68-81` | 输出或文案不同 |
| shim | `hok shim list *` 把 `*` 当正则编译报 invalid pattern；官方先剔除 `*`，`scoop shim list *` 列全部 | `src/cmd/shim.rs:96-109` | `libexec/scoop-shim.ps1:143` | 边界处理不同 |
| create | hok 会真实下载 URL 文件并算 SHA-256 生成清单；官方不下载，只生成静态空模板 | `crates/libscoop/src/package/create.rs:70-107` | `libexec/scoop-create.ps1:6-30` | 语义不同 |
| create | 模板内容不同：hok 填 version `0.0.0`/description/homepage `https://example.com`/license `Unknown`；官方各字段为空串 | `create.rs:85-99` | `libexec/scoop-create.ps1:32-36` | 输出或文案不同 |
| create | 输出目标：hok 默认打到 stdout（`-o` 才写文件）；官方始终写当前目录 `<name>.json` 并打 `Created '<path>'.` | `src/cmd/create.rs:42-53` | `libexec/scoop-create.ps1:27-29` | 语义不同 |
| create | hok 从 URL 推导名字、从不询问版本；官方交互式提示 App name 与 Version | `create.rs:72-74` | `libexec/scoop-create.ps1:18-25,47-59` | 行为缺失 |
| create | 无参数：hok clap 帮助 exit 2；官方调 `scoop-help create` exit 0 | `src/cmd/create.rs:13` | `libexec/scoop-create.ps1:61-65` | 退出码不同 |
| create | URL 非法时 hok 仍尝试下载；官方 `parse_url` 阶段 abort `Error: <url> is not a valid URL` | `src/cmd/create.rs:24-37` | `libexec/scoop-create.ps1:12-16` | 边界处理不同 |

## 7. 明细：检查 / 网络类

> 覆盖 `checkver` `checkhashes` `checkurls` `missing-checkver` `virustotal` `download` `formatjson` `ci-auto-pr` `completions`。
> 前六个中除 `virustotal`/`download` 外，官方**没有同名命令**，对标对象是官方 `bin/*.ps1` 辅助脚本，故此处偏离数量最多、也最需要按用途取舍。

### 7.1 checkver（对标官方 `bin/checkver.ps1` + `lib/autoupdate.ps1`）

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| `-d/--dir` 默认 `.` 直接扫当前目录；官方无默认，`-App` 非文件路径时直接 throw | `src/cmd/checkver.rs:15` | `bin/checkver.ps1:62,85` | 默认值不同 |
| hok 把第二个位置参数也当 app 模式；官方的 `checkver.ps1 APP DIR` 第二位是 `Dir` | `src/cmd/checkver.rs:18-20` | `bin/checkver.ps1:54-62` | 语义不同 |
| hok 不支持 `-App` 为 manifest 文件路径（官方据此推导 `$Dir` 并只匹配叶子名） | `src/cmd/checkver.rs:18-20`、`manifest_walker.rs:71-78` | `bin/checkver.ps1:78-80` | 行为缺失 |
| app 模式匹配：hok 大小写敏感；官方 `Get-ChildItem -Filter` 在 Windows 上大小写不敏感 | `internal/string.rs:60-72` | `bin/checkver.ps1:83` | 边界处理不同 |
| hok 扫描时排除 `package.json`；官方 `*.json` 递归会把它当应用处理 | `manifest_walker.rs:44-46` | `bin/checkver.ps1:83` | 边界处理不同 |
| hok 独有 `-t/--timeout`（默认 30s，同时限制脚本执行）；官方对抓取不设超时 | `src/cmd/checkver.rs:38-40`、`checkver.rs:175,391` | `bin/checkver.ps1:116-121` | hok 独有命令(无官方对标) |
| hok 未实现 `-ThrowError`（代码 TODO 自述），autoupdate 异常只打印，退出码仍 0 | `checkver.rs:45-47` | `bin/checkver.ps1:67,412-418` | 行为缺失 |
| `-V/--version` 时 hok 把版本当第一个捕获组传给 autoupdate（生成 `$match1`）；官方此时 `$matchesHashtable` 未赋值 | `checkver.rs:220-229` | `bin/checkver.ps1:411` | 边界处理不同 |
| 版本不同但清单无 `autoupdate` 键时 hok 仍重写 version 落盘；官方仅在 `$Update -and $json.autoupdate` 时更新 | `checkver.rs:508-518,840-846` | `bin/checkver.ps1:404,406` | 语义不同 |
| hok 仅在 `ver > current` 时打印 "autoupdate available"；官方 `Compare-Version != 0`（降级也算） | `checkver.rs:484-486` | `bin/checkver.ps1:395-397` | 语义不同 |
| `"checkver":"sourceforge"` 字符串 hok 当字面正则；官方走 SourceForge RSS+CDATA 快捷分支 | `manifest_parse.rs:319-322` | `bin/checkver.ps1:171-183` | 语义不同 |
| SourceForge 无自定义 re 时 hok 用 `/([\d.]+)/` 扫整页；官方用锚定文件路径的 CDATA 正则 | `checkver.rs:261-263` | `bin/checkver.ps1:169,182,196` | 语义不同 |
| SourceForge project 提取 hok 只认 `sourceforge.net/projects/<p>`，漏 `sf.net`/`sourceforge.io`/`<project>.sourceforge.net`，也无 `strip_ext $name` 兜底 | `internal/url.rs:204-206`、`checkver.rs:257-258` | `bin/checkver.ps1:172-177` | 边界处理不同 |
| `checkver.url` 优先于 `sourceforge`/`github` 快捷方式（hok）；官方快捷方式块无条件覆盖 `$url` | `checkver.rs:251-281` | `bin/checkver.ps1:159-163,171-197` | 语义不同 |
| `checkver:"github"` 但 homepage 非 github：hok 报错跳过整条；官方只打印 error 后仍请求 | `checkver.rs:283-291` | `bin/checkver.ps1:150-157` | 边界处理不同 |
| hok 无 token 也把 github releases URL 改写为 api.github.com 并自动补 `$.tag_name`；官方仅在有 gh_token 时改写且不注入 jsonpath | `checkver.rs:307-322` | `bin/checkver.ps1:219-224` | 语义不同 |
| `checkver.github` + 自定义 re：hok 仍走 API 并把正则链到 tag_name；官方保持 HTML 抓取并跑用户正则 | `checkver.rs:319-322,571-576` | `bin/checkver.ps1:159-163,342-345` | 语义不同 |
| tag 提取：hok 取整个 `$.tag_name` 仅剥前导 v/V；官方用 `/releases/tag/(?:v\|V)?([\d.]+)/` 只抽数字段，`release-1.2.3` 类 tag 结果不同 | `checkver.rs:479-481` | `bin/checkver.ps1:148` | 语义不同 |
| hok 解析了 `checkver.useragent` 却从不读取，一律用会话 UA | `checkver.rs:171-174,392`、`manifest.rs:306` | `bin/checkver.ps1:117-121` | 行为缺失 |
| hok 不对 `checkver.url` 做 `$version/$dotVersion/...` 变量替换；官方请求前会 substitute | `checkver.rs:251-256,324` | `bin/checkver.ps1:114,226` | 行为缺失 |
| hok 把 PRIVATE_HOSTS 头也注入哈希页/JSON/RDF 辅助抓取；官方 `find_hash_*` 只设 Referer+UA | `network.rs:104-131`、`checkver_hash.rs:60,72` | `bin/checkver.ps1:240-244`、`lib/autoupdate.ps1:24-27` | 语义不同 |
| hok 把 gh_token 以 `Authorization: Bearer` 发给每个 checkver 请求；官方只在 api.github.com 场景加 `Authorization: token` | `checkver.rs:354,395`、`internal/network.rs:111-113` | `bin/checkver.ps1:221-224` | 语义不同 |
| hok 只读配置键 `gh_token`，不读 `SCOOP_GH_TOKEN`；官方以该环境变量优先 | `checkver.rs:164`、`config.rs:232` | `lib/download.ps1:580-582`、`bin/checkver.ps1:88` | 行为缺失 |
| hok 强制 `String::from_utf8`（非 UTF-8 即报下载错）；官方嗅探 `1F 8B` 并按 Content-Type charset 解码 | `checkver.rs:399-406`、`network.rs:137` | `bin/checkver.ps1:288-295`、`lib/download.ps1:539-545` | 边界处理不同 |
| `checkver.script`：hok 另起 powershell 子进程且只注入 `url`；官方用 `Invoke-Command` 进程内执行，脚本可读 `$json/$app/$expected_ver` | `checkver.rs:1151-1166` | `bin/checkver.ps1:271,298-301` | 语义不同 |
| 有 `checkver.script` 时 hok 完全跳过 HTTP 下载；官方仍先下载，下载失败则跳过该包 | `checkver.rs:361-377` | `bin/checkver.ps1:282-301` | 语义不同 |
| `replace` 无 `re`：官方报 `'replace' requires 're' or 'regex'` 并跳过；hok 无校验且会应用到 jsonpath/xpath/整页结果 | `checkver.rs:557-580,598-608` | `bin/checkver.ps1:278-280,364-366` | 行为缺失 |
| jsonpath 非 `$` 开头或 jsonpath-rust 失败时 hok 直接判无版本；官方回落 `json_path_legacy` | `checkver.rs:557-563` | `bin/checkver.ps1:311-313`、`lib/json.ps1:129-158` | 行为缺失 |
| jsonpath 结果：hok 只取第一个且必须 `as_str()`；官方单结果 `ToString()`、多结果整体序列化 | `checkver.rs:563-576` | `lib/json.ps1:113-120` | 语义不同 |
| `reverse` 在 hok 只对正则生效（jsonpath 分支忽略）；官方会传给 `json_path` | `checkver.rs:557-580,623-627` | `bin/checkver.ps1:310` | 语义不同 |
| xpath：hok 用 sxd_xpath 直接求值、不注册 XML 命名空间；带命名空间的 xpath 失败 | `checkver.rs:1107-1119` | `bin/checkver.ps1:320-340` | 行为缺失 |
| 清单解析失败：hok `Err(_) => continue` 静默跳过；官方 `parse_json` warn `Error parsing JSON at '<path>'.` | `checkver.rs:201-204` | `lib/manifest.ps1:9-11`、`bin/checkver.ps1:100` | 输出或文案不同 |
| 无 `autoupdate.hash` 时 hok 一律下载算 SHA256；官方仍按 URL 自动识别 fosshub/sourceforge/github 并抓取哈希 | `checkver_hash.rs:26-41,203-211` | `lib/autoupdate.ps1:232-234,259-269` | 行为缺失 |
| `autoupdate.hash` 的原始 JSON 直接进哈希流水线，`url/regex/jsonpath/find` 变量从不替换（`sub_url` 是恒等函数） | `checkver.rs:868-903`、`checkver_url.rs:7-10` | `lib/autoupdate.ps1:211-223,75-76` | 行为缺失 |
| autoupdate 变量表缺 `$url/$baseurl/$urlNoExt/$basenameNoExt`；官方全部注入 | `checkver.rs:777-812` | `lib/autoupdate.ps1:215-219` | 行为缺失 |
| `$basename`：hok 用去扩展名且不 URL 解码的 `url::basename`；官方用 `url_remote_filename`+`UrlDecode`（含扩展名） | `checkver.rs:855`、`internal/url.rs:43-49` | `lib/autoupdate.ps1:212` | 语义不同 |
| `hash.find`：hok 当字面子串+十六进制 token 匹配；官方当正则并取第 1 组 | `checkver_hash.rs:496-508` | `lib/autoupdate.ps1:246-251,77-80` | 语义不同 |
| hok 缺官方默认正则 `^\s*([a-fA-F0-9]+)\s*$` 与"文件名旁哈希"/metalink 两条兜底正则 | `checkver_hash.rs:484-512` | `lib/autoupdate.ps1:71-73,94-107` | 行为缺失 |
| hok 缺 `$md5/$sha1/$sha256/$sha512/$checksum/$base64` 正则模板替换 | `checkver_hash.rs:484-493` | `lib/autoupdate.ps1:44-51,75-76` | 行为缺失 |
| hok 直接用捕获组原文（含空白）；官方 `-replace '\s',''` 清洗 | `checkver_hash.rs:488-492` | `lib/autoupdate.ps1:79` | 边界处理不同 |
| hash 长度非 32/40/64/128 时 hok 仍写原始值；官方 `format_hash` 返回 `$null` 并抛 "hash failed!" | `checkver_hash.rs:214-219,439-454` | `lib/autoupdate.ps1:15,624-626` | 边界处理不同 |
| 哈希模板数少于 URL 数：hok 回落下载+算 SHA256；官方复用最后一个模板 | `checkver_hash.rs:24,203-211` | `lib/autoupdate.ps1:621` | 语义不同 |
| RDF 提取忽略 basename（参数未用）直接取第一个 Content 的 sha256；官方用 `about -eq $basename` 过滤 | `checkver_hash.rs:317-327` | `lib/autoupdate.ps1:36-38` | 语义不同 |
| base64→hex 只在 hok 的 `extract` 模式生效，fosshub/sourceforge/metalink 路径不转；官方在 `find_hash_in_textfile` 内对所有调用者生效 | `checkver_hash.rs:67` | `lib/autoupdate.ps1:82-92,287,291,296` | 边界处理不同 |
| fosshub 文件名：hok 用仍带 `?dwl=` 的 `remote_filename` 并放宽为 `([a-fA-F0-9]+)`；官方用 `(?:\/\|\?dwl=)(?<filename>.*)$` | `checkver_hash.rs:97,103,227-229` | `lib/autoupdate.ps1:259,291` | 边界处理不同 |
| Digest 头 base64 解码失败：hok 视为"无 Digest"继续走 `.meta4`；官方让 FormatException 逸出中断整轮（hok 注释登记为有意偏离） | `checkver_hash.rs:343-347` | `lib/autoupdate.ps1:193-194` | 有意偏离 |
| 清单原本无顶层 `hash` 时 hok 仍写入新 `hash`；官方只在 `$Manifest.hash` 或 architecture 存在时更新 | `checkver.rs:905-910` | `lib/autoupdate.ps1:374-391` | 语义不同 |
| 属性替换范围：hok 只替换硬编码的 bin/env_add_path/env_set/installer/license/notes/persist/psmodule/shortcuts/extract_dir/url/hash；官方遍历 autoupdate 除 architecture 外的全部属性（如 `depends`） | `checkver.rs:966-1022,1036-1043` | `lib/autoupdate.ps1:483-490` | 行为缺失 |
| 哈希回退下载：hok 落在 `%TEMP%\hok-autoupdate` 且结束即删；官方 `Invoke-CachedDownload` 落到 Scoop 缓存可复用 | `checkver.rs:848-850,1087` | `lib/autoupdate.ps1:318-326` | 语义不同 |
| 输出行：hok `  app (1.0.0 -> 2.0.0)`；官方 `app: 2.0.0 (scoop version is 1.0.0) autoupdate available` | `src/cmd/checkver.rs:71-83` | `bin/checkver.ps1:385-401` | 输出或文案不同 |
| 错误文案：hok 自定义 i18n；官方 `couldn't match '<re>' in <source>` / `couldn't find new version in <source>` | `checkver.rs:440-475`、`locales/en.yml:126-133` | `bin/checkver.ps1:279,284,315,337,371,377` | 输出或文案不同 |
| autoupdate 过程输出：hok 只有一行 "updated to <ver>"；官方另有 `Forcing autoupdate!`/`Autoupdating`/`Searching hash for …`/`Writing updated … manifest` 与 notes 提示 | `src/cmd/checkver.rs:73-83` | `bin/checkver.ps1:408`、`lib/autoupdate.ps1:479,496,503-518` | 输出或文案不同 |

### 7.2 checkhashes（对标官方 `bin/checkhashes.ps1`）

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| `-d/--dir` 默认 `.`；官方 `$Dir` 是 Mandatory | `src/cmd/checkhashes.rs:15` | `bin/checkhashes.ps1:27-35` | 默认值不同 |
| app 是可变位置参数（多个 glob，大小写敏感）；官方单个 `-App` 交 `-Filter "$App.json"`（大小写不敏感） | `src/cmd/checkhashes.rs:19`、`internal/string.rs:62-71` | `bin/checkhashes.ps1:26,62` | 语义不同 |
| 给了 `-u` 但 hash 全对时 hok 不写回；官方只要 `$Update` 为真就无条件重写 manifest | `checkhashes.rs:167-175` | `bin/checkhashes.ps1:159-189` | 语义不同 |
| `-f/--force`：hok 每条都记为 mismatch 并打印 "mismatch! updated"；官方仍按真实比较打印 `OK`（只是随后写回） | `checkhashes.rs:162,175` | `bin/checkhashes.ps1:52,140-159` | 输出或文案不同 |
| 遇 `""` 空 hash：hok 立即判失败并 `break`（后续 URL 不再校验、`-u` 不写回）；官方把空 hash 当 sha256 期望值比较并可写回真实 hash | `checkhashes.rs:103-110` | `bin/checkhashes.ps1:115`、`lib/download.ps1:754-766` | 语义不同 |
| `"hash":"TODO"` 之类非法 hash 让整个清单解析失败并被 hok 静默 `continue`；官方能解析并报 hash mismatch | `checkhashes.rs:68-71`、`manifest.rs:1337-1340` | `lib/manifest.ps1:5-12`、`bin/checkhashes.ps1:115,125-136` | 行为缺失 |
| URL 集合：hok 用 `all_urls()/all_hashes()` 合并顶层与 64/32/arm64；官方二选一（有顶层 url 就只用顶层） | `checkhashes.rs:83,240-259`、`manifest.rs:1120-1172` | `bin/checkhashes.ps1:73-83` | 语义不同 |
| ~~缓存文件已存在时 hok 默认跳过下载（仅 `-f` 或不存在才下）~~ **❌ 结论有误（2026-09-13 实测更正）**：hok 默认（不带 `-k`）**每次重下**、跑完删临时目录，**与官方默认一致**；`-k` ≡ 官方 `-UseCache`（复用缓存）。**真缺陷**是 `-k` 的缓存键**不含版本**（官方 `<app>#<version>#…`），跨版本同 URL 会复用旧文件报**假 OK**——已在 S10 修复为 `<cache>/<version>/…` | `checkhashes.rs:122-123`（旧）→ S10 `4e990a0` | `bin/checkhashes.ps1:122`、`lib/download.ps1:56` | **已更正 / 已修** |
| 缓存位置：hok `%TEMP%\hok-checkhashes`，名 `<name>-<arch前缀>-<远端文件名>`；官方 `<cachedir>\<app>#HASH_CHECK#<sha7><ext>` | `checkhashes.rs:58-61,114-120` | `bin/checkhashes.ps1:119-124`、`lib/core.ps1:367-379` | 语义不同 |
| 缓存位置与清理时机不同：hok 用 `%TEMP%\hok-checkhashes` 且跑完删（`-k` 保留）；官方用用户缓存目录、跑前清 `$cachedir\*HASH_CHECK*`、跑完文件留盘 | `checkhashes.rs:234-236` | `bin/checkhashes.ps1:54` | **有意偏离（S10 登记保留）**：不污染用户真实 Scoop 缓存 |
| `-k/--keep-cache` 只保留 hok 临时目录；官方 `-UseCache` 还把缓存 key 换成 manifest 版本（复用真实下载缓存） | `checkhashes.rs:58-61,234` | `bin/checkhashes.ps1:116-122`、`lib/download.ps1:56-60` | 语义不同 |
| hok 额外提供 `-c/--cache` 指定缓存目录；官方无对应参数 | `src/cmd/checkhashes.rs:39-40` | `bin/checkhashes.ps1:25-41` | 语义不同 |
| 清单既无 url 也无 architecture 时 hok 静默跳过；官方打印 `Manifest does not contain URL property.` | `checkhashes.rs:83-94` | `bin/checkhashes.ps1:84-87` | 行为缺失 |
| URL/hash 数量不匹配：hok 记 Failed 并计汇总；官方只打印 `URLS and hashes count mismatch.` 后 continue、不计统计 | `checkhashes.rs:86-93`、`src/cmd/checkhashes.rs:82-88` | `bin/checkhashes.ps1:90-93` | 语义不同 |
| 单个 manifest 下载失败：hok 记错后继续；官方 `Invoke-CachedDownload` 抛异常直接中断脚本 | `checkhashes.rs:123-130` | `bin/checkhashes.ps1:122`、`lib/download.ps1:71-86` | 边界处理不同 |
| hok 的校验下载完全不发 UA/Referer/PRIVATE_HOSTS 头（`RequestOptions::default()`） | `network.rs:64-73`、`internal/network.rs:99-104` | `lib/download.ps1:93-110` | 行为缺失 |
| hok 下载不做 `handle_special_urls` 重写（fosshub/sourceforge/github 私有 release） | `network.rs:64-83` | `lib/download.ps1:76,595-624` | 行为缺失 |
| 写回 `{ "url":…, "hash":… }` 对象形式时 hok 会刷新对象内 hash 成员（注释登记为有意偏离） | `manifest.rs:612-618,648-679` | `bin/checkhashes.ps1:160-161`、`lib/autoupdate.ps1:566-575` | 有意偏离 |
| nightly 判定：hok 大小写敏感 `== "nightly"`；官方 `-eq` 不区分大小写 | `checkhashes.rs:72` | `bin/checkhashes.ps1:68` | 边界处理不同 |
| hok 末尾额外打印 `Scanned N: … ok, … failed, … updated.`；官方无汇总 | `src/cmd/checkhashes.rs:82-88` | `bin/checkhashes.ps1:107-189` | 输出或文案不同 |
| mismatch 明细：hok 只打印被截断的 expected/actual；官方还打印 First bytes(magic) 与完整 Expected/Actual | `checkhashes.rs:207-226` | `bin/checkhashes.ps1:148-156` | 输出或文案不同 |
| 写回成功：hok `<app>: hash mismatch! updated`；官方 `Writing updated <app> manifest` | `src/cmd/checkhashes.rs:64-72` | `bin/checkhashes.ps1:184` | 输出或文案不同 |

> **状态（2026-09-13，S10 `fix/s10-checkhashes` → 已线性 ff 进 `refactor/edition-2024`，tip `e97744a`）**：§7.2 全表 22 条已逐条处置，提交 `e859192` / `6e142ef` / `4e990a0` / `ad5d253`。
> - **已更正的两行**：上表「hok 默认跳过下载」「hok 默认复用」经**本地实测**（本地夹具服务器 + 换服务端内容 + 数请求）**不成立**；真缺陷是 `-k` 的缓存键不含版本（已修）。
> - **已修**：空 hash（`""` 按 sha256 期望比较、`-u` 可写回）、数组逐条独立（去 `break`）、`TODO`/未知算法/对象形式不再静默 skip、顶层 url 优先（`checkhashes_accessors()`，不动 `all_urls()`）、app 过滤大小写不敏感、`nightly` 大小写不敏感、无 URL 打印官方文案、mismatch 明细补 `First bytes` + 完整 Expected/Actual、数量不匹配不再误记 failed、`-k` 缓存键含版本。
> - **有意偏离（登记保留）**：`handle_special_urls` 不做；`-d` 默认 `.` 与多 glob；末尾汇总行；缓存位置与清理时机；`-c/--cache` 扩展；下载失败继续（官方抛异常中断）；`-u` 只在有 mismatch 时写回（官方无条件重写**全部** manifest，hok 更安全）；写回只动被改文件并保留原行尾。
> - **划出**：`download_file` 缺 UA/Referer/cookies/PRIVATE_HOSTS（本表 `network.rs:64-73` 那行）→ 另开会话 `fix/d4-download-headers`。

### 7.3 checkurls（对标官方 `bin/checkurls.ps1`）

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| `-d/--dir` 默认 `.`；官方 `$Dir` Mandatory | `src/cmd/checkurls.rs:15` | `bin/checkurls.ps1:16-24` | 默认值不同 |
| app 为可变位置参数（多个 glob、大小写敏感）；官方单个 `-App` + `-Filter`（大小写不敏感） | `src/cmd/checkurls.rs:19`、`internal/string.rs:62-71` | `bin/checkurls.ps1:15,36` | 语义不同 |
| hok 用 HTTP HEAD 探测；官方用 `GetResponse()`（默认 GET） | `checkurls.rs:78`、`internal/network.rs:189` | `bin/checkurls.ps1:56,74` | 语义不同 |
| hok 把 200–399 全算成功（含 204/206/3xx）；官方只认 `OK`(200)/`OpeningData` | `internal/network.rs:202-206` | `bin/checkurls.ps1:107` | 语义不同 |
| URL 集合：hok 取当前平台 `url()`（x64 只查 64bit）；官方无顶层 url 时固定查 64bit+32bit+arm64 | `checkurls.rs:49-57`、`manifest.rs:1104-1111` | `bin/checkurls.ps1:96-102` | 语义不同 |
| 片段裁剪：hok 砍掉第一个 `#` 之后全部；官方只裁 `#/`（`#frag` 原样请求） | `checkurls.rs:59-62`、`internal/url.rs:115-117` | `bin/checkurls.ps1:54` | 边界处理不同 |
| manifest 无任何 URL 时 hok 直接 `continue` 不输出该包；官方仍打印 `[0][0][0] <name>` | `checkurls.rs:64-66` | `bin/checkurls.ps1:110-136` | 边界处理不同 |
| 解析失败的 manifest hok 静默跳过；官方 warn 并输出 0 URL 行 | `checkurls.rs:44-47` | `lib/manifest.ps1:5-11`、`bin/checkurls.ps1:112-136` | 行为缺失 |
| hok 走 reqwest 只支持 http(s)；官方经 `FtpWebRequest` 支持 ftp:// | `internal/network.rs:189` | `bin/checkurls.ps1:58,83,107` | 行为缺失 |
| UA：hok 固定 `Scoop/1.0 (+http://scoop.sh/)`；官方 `Get-UserAgent` 含 PowerShell/OS 版本与架构 | `network.rs:53` | `lib/download.ps1:547-549`、`bin/checkurls.ps1:59` | 行为缺失 |
| Cookie 头：hok 用 `"; "` 连接；官方 `cookie_header` 用 `';'` 无空格 | `internal/network.rs:106-108` | `lib/download.ps1:529-537` | 边界处理不同 |
| 表头：hok 单行 `[U]RLs   [O] [F]`；官方多行 `[U]RLs`/`\| [O]kay`/`\|  \| [F]ailed` | `src/cmd/checkurls.rs:38-43` | `bin/checkurls.ps1:41-50` | 输出或文案不同 |
| 错误明细：hok 合成 `HTTP <code>`、缩进 8 空格；官方 .NET 异常文案、缩进 7 空格 | `src/cmd/checkurls.rs:75-77`、`internal/network.rs:205` | `bin/checkurls.ps1:76-81,138-139` | 输出或文案不同 |
| hok 末尾额外打印 `Checked N URLs: … valid, … invalid.`（或 `No manifests to check.`）；官方无汇总 | `src/cmd/checkurls.rs:82-91` | `bin/checkurls.ps1:52-141` | 输出或文案不同 |

### 7.4 missing-checkver（对标官方 `bin/missing-checkver.ps1`）

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| **`-s/--supported` 语义相反**：hok=只列有 checkver 或 autoupdate 的；官方 `-SkipSupported`=跳过两者都有的 | `src/cmd/missing_checkver.rs:17-19`、`missing_checkver.rs:41-44` | `bin/missing-checkver.ps1:23,42` | 语义不同 |
| 默认输出：hok 只输出缺字段的 manifest；官方输出全部 manifest 的 `[C][A]` 矩阵 | `missing_checkver.rs:55-57`、`src/cmd/missing_checkver.rs:32-35` | `bin/missing-checkver.ps1:44-51` | 语义不同 |
| hok 无 `-App` 过滤，永远扫整个目录 | `src/cmd/missing_checkver.rs:12-20` | `bin/missing-checkver.ps1:13,39` | 行为缺失 |
| `-d/--dir` 默认 `.`；官方 `$Dir` Mandatory | `src/cmd/missing_checkver.rs:14` | `bin/missing-checkver.ps1:14-22` | 默认值不同 |
| hok 用 `Option::is_some()` 判"有"，`"checkver": ""` 也算有；官方真值判断视其为缺失 | `missing_checkver.rs:37-38`、`manifest_parse.rs:315-335` | `bin/missing-checkver.ps1:42,45,49` | 语义不同 |
| 解析失败的 manifest hok 静默跳过；官方输出 `[ ][ ] name` | `missing_checkver.rs:27-30` | `bin/missing-checkver.ps1:40,44-51` | 边界处理不同 |
| hok 排除 `package.json`；官方 `*.json` 会把它当 manifest 报告 | `manifest_walker.rs:44-46` | `bin/missing-checkver.ps1:39` | 边界处理不同 |
| 输出格式：hok `name (issues)`+汇总行；官方固定 `[C]heckver \| [A]utoupdate` 表头+逐行 `[C][A] name` | `src/cmd/missing_checkver.rs:28-46` | `bin/missing-checkver.ps1:31-51` | 输出或文案不同 |

### 7.5 virustotal

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| 缺 `-a/--all`、`-s/--scan`、`-n/--no-depends`、`-u/--no-update-scoop`、`-p/--passthru` 全部选项，也无过期自动更新 | `src/cmd/virustotal.rs:15-20` | `libexec/scoop-virustotal.ps1:23-29,38,44-50` | 行为缺失 |
| `*`/`-a`：官方枚举本机（本地+全局）已装应用；hok 无 `--all` 且以 `installed=false` 只查 bucket 清单 | `src/cmd/virustotal.rs:24-25` | `libexec/scoop-virustotal.ps1:40,52-54` | 语义不同 |
| 官方默认经 `Get-Dependency` 展开依赖再检查（`-n` 跳过）；hok 从不展开依赖 | `src/cmd/virustotal.rs:25` | `libexec/scoop-virustotal.ps1:56-58` | 行为缺失 |
| 官方遍历 manifest 全部下载 URL（多 URL 打印 "url N"）；hok 只取 `urls[0]` | `src/cmd/virustotal.rs:48,59` | `libexec/scoop-virustotal.ps1:279-286` | 语义不同 |
| hok 总是 POST `/urls` 提交分析再轮询（等价官方仅 `-s` 的行为）；官方默认先用 manifest 哈希查 `/files/{hash}`、不提交 | `virustotal.rs:59-106` | `libexec/scoop-virustotal.ps1:301-308,348` | 语义不同 |
| hok 提交前剥掉 URL 的 `#` 片段；官方用含片段原始 URL 计算 base64 URL id | `src/cmd/virustotal.rs:59` | `libexec/scoop-virustotal.ps1:82-88,163` | 边界处理不同 |
| API key：hok 优先读 `VT_API_KEY` 环境变量；官方只读配置 `virustotal_api_key` | `src/cmd/virustotal.rs:32-34` | `libexec/scoop-virustotal.ps1:68` | 语义不同 |
| 统计口径：官方 `unsafe=malicious+suspicious`、`total=unsafe+undetected`；hok 直接展示 VT 原值 | `src/cmd/virustotal.rs:65-93`、`virustotal.rs:99-104` | `libexec/scoop-virustotal.ps1:99-104` | 语义不同 |
| 输出：hok 本地化 MALICIOUS/SUSPICIOUS/OK 且不打印报告链接；官方 `app: unsafe/total, see <report_url>` | `src/cmd/virustotal.rs:67-93`、`locales/en.yml:259-261` | `libexec/scoop-virustotal.ps1:122-135` | 输出或文案不同 |
| hok 未实现退出位 8（manifest 找不到），无匹配包时以通用错误 exit 1 | `src/cmd/virustotal.rs:27-29` | `libexec/scoop-virustotal.ps1:62,272-275` | 退出码不同 |
| 官方遇 204/429 立即 abort；hok 把任何错误统一记 bit 4 并继续后续包 | `src/cmd/virustotal.rs:95-98,102-103` | `libexec/scoop-virustotal.ps1:315-316,351-352` | 边界处理不同 |
| 限流：官方 sleep 60+requests 秒后重试一次并只提示一次；hok 固定 3s×5 次轮询后报超时 | `virustotal.rs:81-82,108-110` | `libexec/scoop-virustotal.ps1:252-258` | 边界处理不同 |
| 缺参数：官方 `my_usage; exit 1`；hok clap `arg_required_else_help` exit 2 | `src/cmd/virustotal.rs:15` | `libexec/scoop-virustotal.ps1:41` | 退出码不同 |

### 7.6 download

| 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|
| 缺 `-u/--no-update-scoop`，也无过期时默认先跑 `scoop-update.ps1` 的行为 | `src/cmd/download.rs:15-32` | `libexec/scoop-download.ps1:19,32,46-52` | 行为缺失 |
| 长选项主名 hok 是 `--no-hash-check`（`--skip-hash-check` 仅 alias）；官方是 `--skip-hash-check` | `src/cmd/download.rs:25` | `libexec/scoop-download.ps1:18,32` | 输出或文案不同 |
| `-f/--force` 时官方打印一次 `Cache is being ignored.`；hok 静默忽略缓存 | `src/cmd/download.rs:21-22`、`download_apps.rs:90` | `libexec/scoop-download.ps1:55` | 输出或文案不同 |
| 失败策略：官方只 continue 并最终 `exit 0`；hok 汇总失败集后返回错误使进程非 0 | `download_apps.rs:104-113,125` | `libexec/scoop-download.ps1:110-130,143` | 退出码不同 |
| 缓存查找：hok 只用 `name#version#<sha256前7><ext>` 单一公式；官方 `cache_path` 先复用旧的 `<app>#<version>#<下划线URL>` 迁移路径 | `crates/libscoop/src/package/mod.rs:600-621` | `lib/core.ps1:367-381` | 行为缺失 |
| 扩展名：hok 先剥 `?query` 再取（注释登记偏离）；官方直接 `Path.GetExtension($url)` | `package/mod.rs:607-618` | `lib/core.ps1:378` | 有意偏离 |
| nightly 包：官方先把版本换成 `nightly-YYYYMMDD` 再参与缓存命名；hok 直接用清单版本 `nightly` | `package/mod.rs:231-233,290-296`、`download_apps.rs:40` | `libexec/scoop-download.ps1:91-94` | 语义不同 |
| aria2：官方调用 aria2 二进制（失败回退默认下载器）；hok 把 `aria2_enabled` 当内置分片并发开关、从不调用 aria2 | `download.rs:520-524` | `lib/download.ps1:261-263,332-472` | 语义不同 |
| hok 下载只带 proxy/timeout/UA/cookie，不发 Referer 与 PRIVATE_HOSTS，也不为 api.github.com 加 Accept/Bearer | `download.rs:550-555`、`internal/network.rs:95-120` | `lib/download.ps1:88-111` | 行为缺失 |
| hok 不做 `handle_special_urls` 的 FossHub/SourceForge/GitHub 私有资源 URL 重写 | `download.rs:274,337` | `lib/download.ps1:76,595-632` | 行为缺失 |
| 缓存复用条件：官方只看 `Test-Path $cached`；hok 还要求 HEAD 到的远程大小与本地相等（无 Content-Length 时重复下载） | `download.rs:87-90,775-787` | `lib/download.ps1:56` | 边界处理不同 |
| 进度文案：官方打印 `Downloading 'app' (version) [arch] from bucket` 与 `Skipping hash verification.`；hok 只打印统一状态行 | `src/scoop_handler.rs:36-38`、`locales/en.yml:13` | `libexec/scoop-download.ps1:64,133` | 输出或文案不同 |
| 缓存命中提示：官方打印 `url_remote_filename`；hok 打印含 `name#version#hash` 的缓存文件名 | `download.rs:591-592`、`src/scoop_handler.rs:248-249` | `lib/download.ps1:60` | 输出或文案不同 |
| 哈希不匹配：官方打印 App/URL/First bytes/Expected/Actual 详情块并给 SourceForge 专门提示；hok 只发一条 i18n 错误 | `download_verify.rs:103-113` | `lib/download.ps1:737-748`、`libexec/scoop-download.ps1:126-128` | 输出或文案不同 |
| 缺 `<app>`：官方 `error '<app> missing'; my_usage; exit 1`；hok clap exit 2 | `src/cmd/download.rs:14,17` | `libexec/scoop-download.ps1:44` | 退出码不同 |

### 7.7 formatjson / ci-auto-pr / completions

| 命令 | 偏离 | hok 证据 | Scoop 证据 | 类别 |
|---|---|---|---|---|
| formatjson | 子命令名被 clap kebab 化为 `format-json`，`hok formatjson` 报 unrecognized subcommand（实测）；官方脚本名是 `formatjson.ps1` | `src/cmd/mod.rs:142`、`src/cmd/formatjson.rs:10-11` | `bin/formatjson.ps1:1` | 语义不同 |
| formatjson | `-d/--dir` 默认 `.`；官方 `$Dir` Mandatory | `src/cmd/formatjson.rs:14` | `bin/formatjson.ps1:19-27` | 默认值不同 |
| formatjson | hok 按 stem 大小写敏感匹配；官方 `-Filter "$App.json"` 大小写不敏感 | `src/cmd/formatjson.rs:18-19`、`internal/string.rs:60-71` | `bin/formatjson.ps1:36` | 边界处理不同 |
| formatjson | hok 跳过 `package.json`；官方 `*.json` 会一起重排 | `manifest_walker.rs:44-46`、`formatjson.rs:101` | `bin/formatjson.ps1:36` | 边界处理不同 |
| formatjson | 解析失败：hok 记错并保持原文件；官方 `$null` 绑定失败致 `WriteAllLines` 把 manifest 截成单空行（实测） | `formatjson.rs:102-106` | `bin/formatjson.ps1:39-43`、`lib/json.ps1:9-11` | 语义不同 |
| formatjson | hok 额外输出 schema 告警（缺 version/homepage/license、license.identifier、uninstaller oneOf、version 字符集、shortcuts 2–4 元素）；官方无校验 | `formatjson.rs:107,128-194` | `bin/formatjson.ps1:36-44` | 输出或文案不同 |
| formatjson | hok 额外打印 "Formatted N manifest(s)."/"No manifests needed formatting." 及告警；官方成功时静默 | `src/cmd/formatjson.rs:28-43`、`locales/en.yml:149-150` | `bin/formatjson.ps1:36-44` | 输出或文案不同 |
| formatjson | 序列化深度：hok 无限制；官方 `ConvertTo-Json -Depth 8` 会降级更深结构 | `formatjson.rs:206-211` | `lib/json.ps1:18` | 边界处理不同 |
| formatjson | hok 仅在字节真正变化时写盘；官方对每个匹配文件无条件 `WriteAllLines` | `formatjson.rs:65-68` | `bin/formatjson.ps1:43` | 边界处理不同 |
| formatjson | 解析宽容度不同（hok 严格 JSON→JSON5 回退，官方只 `ConvertFrom-Json`），hok 会格式化 PS 5.1 拒绝的清单（注释登记） | `formatjson.rs:30-34`、`internal/json.rs:72-75` | `bin/formatjson.ps1:39`、`lib/manifest.ps1:8` | 有意偏离 |
| formatjson | hok 有意不做官方 `ConvertToPrettyJson` 的字符串反转义与 `` -replace "`t",'    ' ``，`\t` 保持原样（注释登记） | `formatjson.rs:35-39` | `lib/json.ps1:49-69`、`bin/formatjson.ps1:42` | 有意偏离 |
| ci-auto-pr | `-u/--upstream` 在 hok 可选且只在 `-r` 时使用；官方 `$Upstream` 是 Mandatory 且每次调用（含 `-Push`）都校验 `<user>/<repo>:<branch>` | `src/cmd/auto_pr.rs:26-27,74-83`、`auto_pr.rs:96-100` | `bin/auto-pr.ps1:40-47` | 语义不同 |
| ci-auto-pr | hok 容忍缺 `:` 的 upstream（默认分支 `master`）；官方 ValidateScript 抛格式错误 | `src/cmd/auto_pr.rs:74-83` | `bin/auto-pr.ps1:41-46` | 边界处理不同 |
| ci-auto-pr | 既无 `-p` 也无 `-r` 时官方打印 usage 并 `exit 0`；hok 不打印 usage 且先校验 token，以"缺 GITHUB_TOKEN"报错 exit 1 | `auto_pr.rs:86-95`、`src/cmd/auto_pr.rs:19` | `bin/auto-pr.ps1:78-92` | 退出码不同 |
| ci-auto-pr | 无参数调用：官方交互式索要 Mandatory `-Upstream`；hok clap `arg_required_else_help` exit 2（实测） | `src/cmd/auto_pr.rs:19` | `bin/auto-pr.ps1:39-47,78-92` | 退出码不同 |
| ci-auto-pr | `-d/--dir` 在 hok 默认 `bucket`；官方无默认：仅当 `-App` 是文件路径时推导，否则抛 `'-Dir' parameter required if '-App' is not a filepath!` | `src/cmd/auto_pr.rs:21-23` | `bin/auto-pr.ps1:70-76` | 默认值不同 |
| ci-auto-pr | hok 无 `-App` 参数（永远以 app 模式 `*` 扫整目录）；官方支持应用名/占位符或 manifest 文件路径收窄 | `auto_pr.rs:147` | `bin/auto-pr.ps1:49,70-72,175` | 行为缺失 |
| ci-auto-pr | hok 无 `-ThrowError` 且显式丢弃 checkver 错误（`let _ =`）；官方暴露 `-ThrowError` 并透传 | `auto_pr.rs:154-159,180-185` | `bin/auto-pr.ps1:30-31,64,175,179` | 行为缺失 |
| ci-auto-pr | 变更判定：hok 用 checkver 前后字节快照（任何字节变化都提交）；官方用 `hub diff --name-only` 并显式跳过"只有 LF/CRLF 变化"的 manifest | `auto_pr.rs:472-484` | `bin/auto-pr.ps1:183-208` | 语义不同 |
| ci-auto-pr | 官方预检 `hub` 二进制缺失即 exit 1 并给安装提示；hok 从不检查 `hub`，改为要求 GITHUB_TOKEN | `auto_pr.rs:86-90` | `bin/auto-pr.ps1:94-104` | 语义不同 |
| ci-auto-pr | `--special` 只接受单个逗号分隔字符串；官方 `-SpecialSnowflakes` 是 `[string[]]` | `src/cmd/auto_pr.rs:49-51,85-92` | `bin/auto-pr.ps1:62,178` | 边界处理不同 |
| ci-auto-pr | 输出：hok 打 `Repository`/`Branch`(/`Upstream`) 头与最终 pushed/created/skipped 汇总；官方是 `Updating ...`/`Creating update ...`/`Pushing updates ...` 且无汇总 | `auto_pr.rs:107-113,416-429` | `bin/auto-pr.ps1:166,198,214-220` | 输出或文案不同 |
| completions | hok 用 clap_complete 生成 bash/elvish/fish/powershell/zsh 补全；官方仓库完全没有补全生成器（全树无 `Register-ArgumentCompleter`） | `src/cmd/completions.rs:16-23` | `bin/scoop.ps1:1-53`（全树无匹配） | hok 独有命令(无官方对标) |

---

## 8. 未验证清单（后续实测入口）

- **退出码 2 系列**（clap 缺参/用法错误）均按 clap 4 默认行为推断，未逐个跑二进制确认。
- **`hok update -f` 同版本重装的目录行为**：`_*.old` 备份缺失为静态读码推断，未端到端验证。
- **网络类全部结论**：未联网比对真实响应（checkver 的 github/sourceforge/fosshub 分支、checkurls 的 3xx 判定、virustotal 限流、download 的缓存复用/UA/Referer）。
- **`checkver.script` 子进程实际环境变量**、`ConvertTo-Json -Depth 8` 截断、PowerShell 5.1 下 formatjson 截断行为：静态读码 + 局部实测（formatjson 截断与 clap exit 2 子代理已实测）。
- **`info` nightly 版本显示**（hok 显示 `nightly`，官方显示 junction 目标目录名）本轮未复核代码，仅见历史记忆。
- 官方 `search`/`info`/`home` 是否有 `-b` bucket 过滤、官方 `cat`/`prefix` 是否支持多参数/`exe`：本地 0.5.3 源码中无，无从比对。
- 各命令 `--help` 文案未逐条穷尽比对。
- 未纳入（**已核实并修复 2026-09-13，S4**）：`config list --all` 把 `ignore-failures` 默认显示为 `true` 而运行时默认 `false`——`53e35f3` 只改了 getter（`false`），默认列、两份 locale 长帮助与两份 README 漏改；现全部为 `false`，并新增守卫测试（新配置下每个 bool 行的默认列必须等于 getter）。

---

## 9. 后续建议

**优先级 A（功能性缺陷，最接近 checkup 级别，建议先决策）**

1. `alias` —— 别名不可调用：要么生成 shim 脚本（对齐官方），要么明确 hok 的别名语义并在 README/帮助中写明；
2. `export` / `import` —— 与官方 Scoopfile 互通：至少让 `import` 能读官方 Scoopfile（数组形态 + Info 字段）；
3. `checkup` —— 语义决策（对齐官方环境检查 / 保留健康检查并改名），见 `.agents/docs/active/audit/scoop-gap-analysis-2026-08.md` §4.4。

**优先级 B（一致性缺口，按用户实际用法取舍）**

- `config` 键白名单：是否接受未知键（官方行为）或继续严格拒绝；
- `cleanup` 无参语义、`-k` 清空整缓存：有数据损坏风险，建议对齐官方；
- `cache rm` 的子串匹配：`cache rm zip` 误删面较大，建议改锚定；
- `which` 输出 shim 自身而非真实目标；
- 退出码体系：hok 全局 0/1 vs 官方各命令自定（影响脚本兼容），是否统一需单独决策。

**优先级 C（低风险，可批量）**

- 文案/列名/列序（`status`、`list`、`search`、`bucket`、`shim`）；
- `aria2-min-split-size` 默认值 20M → 5M；
- `download --skip-hash-check` 长选项主名；
- `hold`/`unhold` 的 `scoop` 特例。

**不建议改（有意偏离）**：§4–§7 中标注"有意偏离"的全部条目，以及 hok 单二进制设计带来的无自更新、无 `scoop_repo/scoop_branch/shim` 配置键。

---

## 10. S1 + S13 处置记录（2026-09-12）

> **范围**：S1 = 退出码与批量失败策略口径（A 全面对齐）；S13 = 安装/卸载批量行为（运行中进程 + 未安装应用对齐，`scoop` 保留）。
> **分支**：worktree `.worktrees/s1-s13`，`fix/exit-code-and-batch-policy`；提交 `4a1fd5a`（clap 层）、`e7d1e25`（逐命令）、`a4ea0b7`（批量行为）、`cfd44ed`（download）——rebase 到 S2 的 `3c90095` 后**线性 ff** 进 `refactor/edition-2024`（tip `cfd44ed`，无 merge commit）。
> **证据**：开工前会话内的官方退出码枚举（`libexec/*.ps1`、`lib/*.ps1`、`bin/*.ps1` 全量 `exit` 出现点）+ hok 侧隔离 HOME/`SCOOP` 实测。

### 10.1 已修（对应本报告行）

| 报告位置 | 原偏离 | 现在 |
|---|---|---|
| §1、§4.3/§4.4/§4.5/§5.x/§7.x 所有"缺参/用法错退出码不同" | clap 一律 exit 2 | 统一 **exit 1**；帮助/版本/裸 `hok` **exit 0**（`src/cmd/mod.rs` `exit_for_parse_error`） |
| §4.4 未安装应用（exit 1） | 官方报 `'x' isn't installed.` 后继续、exit 0 | 已对齐；成功页脚 `Everything is ok!` **2026-09-13 晚已整体删除**（install/uninstall/update/reinstall 都不再打印；`status` 保留，官方 `scoop-status.ps1:80` 确有），原「无已装目标时抑制页脚」的守卫随之删除 |
| §4.4 运行中进程中止整批（exit 1） | 官方 `continue` 跳过该包、exit 0 | 已对齐；**`ignore_running_processes` 同名异义一并修正**——⚠️ **2026-09-13 晚更正**：此处原先写成「官方语义=只决定 warn/error 文案，仍跳过」，实为误读官方 ignore 分支 `return $false`（= 继续）。现语义：默认 WARN + 跳过；该键为 true 时 WARN（两行风险提示）+ **继续**；进程名+pid 仅 `-vv` 输出（`Output::Detail`）。见 `.agents/skills/scoop-gap-audit.md`「运行中进程输出 + ignore_running_processes 语义回正」 |
| §4.5 未安装应用 exit 1 / 运行中进程中止整批 | 同上 | 已对齐（`sync_install.rs` only_upgrade 分支 + `filter_not_running`） |
| §4.6 `reset` 运行中进程 | 官方跳过、exit 0 | 已对齐（同一 `check_not_running` 语义） |
| §5.1 “空结果 exit 0” | 一个包都没装时官方 exit 1 | 已对齐（**有包但查询无匹配仍为 0**，与官方一致） |
| §5.2 `search` 无匹配静默 exit 0 | 官方 warn + exit 1 | 已对齐 |
| §6.2 `bucket list` 无桶 exit 0 | 官方 warn + exit 2 | 已对齐 |
| §6.2 `bucket add` 已存在 exit 1 | 官方 exit 2 | 已对齐 |
| §6.5 `shim add` 目标不存在照样生成（exit 1） | 官方拒绝 + exit 3 | 已对齐（`add_custom` 校验目标存在，新增 `Error::ShimTargetMissing`） |
| §6.5 `shim rm`/`info` 未找到 exit 1 | 官方 3；`info` 跨作用域 2 | 已对齐 |
| §7.5 `virustotal` 未找到 manifest 通用 exit 1 | 位掩码 **8**（可与 2/4 叠加） | 已对齐 |
| §7.6 `download` 部分失败 exit 非 0 | 官方逐包 continue、exit 0 | 已对齐（逐包错误与汇总行仍打印，进程码为 0） |

§8「未验证清单」中"退出码 2 系列"已由 `tests/exit_codes.rs` 实测锁定（裸 `hok` 0、未知命令 1、缺参 1、`which` 2、`bucket list` 2、`shim` 3、`virustotal` 8、未安装 0）。

### 10.2 有意偏离（本轮明确不改，已登记）

| 报告位置 | 偏离 | 结论 |
|---|---|---|
| §4.4 held 包静默跳过 | 官方 uninstall 无 hold 检查 | 保留（hold 防误删）；原「`hok uninstall <held>` 仍打 `Everything is ok!`」的文案误解已随**页脚整体删除**消失（2026-09-13 晚） |
| §4.4 拒卸被依赖包 | 官方无反向依赖检查 | 保留安全增益；`SyncOption::NoDependentCheck` 无 CLI 出口，若日后需要逃生口单独决策 |
| §4.4 `scoop` 自身不可卸 | 官方特判走 `bin/uninstall.ps1` | 保留（hok 单二进制，`apps/scoop` 不是 hok 自身） |
| §4.4 运行中**文件占用**导致删目录失败 → exit 1 | 官方 `error + continue`、exit 0 | 本轮未改：属"操作失败"（hok 由 `ignore_failures` 覆盖），非"跳过决策"；如需完全对齐另开 |
| §7.6 `download` 汇总错误行 | 官方只有逐包 error | 保留（信息更全；退出码已对齐） |

### 10.3 仍待各自会话的退出码（与语义绑定）

`reset <未安装>`（§4.6：官方 0 / hok 1）、`shim`/`alias` 无子命令（§6.1/§6.5：官方 `<subcommand> missing`+1 / hok 默认 list+0）、`hok -v` 等只带全局选项时 exit 1（官方无对应场景）。这些都跟着语义一起改（S7/S14）。

> 原列表中的 `cleanup` 无参与 `cache` 裸命令**已由 S6 一并解决**：`cleanup` 无参 → exit 1；裸 `cache` / `cache <app>` → 列缓存 exit 0（见 §4.1/§4.2 处置列与 §10.4）。

### 10.4 S6（cleanup / cache 数据安全，`cc2c072`）

> **分支**：worktree `.worktrees/s6-cleanup-cache`，`fix/cleanup-cache-safety`；提交 `1e6b929`（cache 选择锚定 + sidecar）、`33ba858`（`-k` 保留当前版本 + 显式目标列表）、`1c1311b`（`-g` 管理员 + `-a -g` 两趟）、`cc2c072`（无参/未安装语义 + 裸 `cache` 默认列缓存）——rebase 到 S5 的 `5ae994d` 后**线性 ff** 进 `refactor/edition-2024`（无 merge commit）。
> **证据**：隔离 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE` 环境的进程级实测（10 条）+ `cargo test -p hok`/`--test exit_codes`/`--test i18n_keys`，以及新增 libscoop 单测（cache 锚定/多 app/`-k` 保留当前版本/空名单为空操作/`Confirm-InstallationStatus` 三态）。
> **有意/微偏离**：`cache` 用字面全等而非官方 regex 元字符；缺参报错用提示文案替代官方 usage 块；broken app（`current` junction 缺失）报"未正确安装"后**跳过**（官方会删光其版本目录，hok 保留更安全行为）；`cache`/`cleanup` 输出样式归 S7。

---

## 11. S7 处置记录（2026-09-13）

> **范围**：fix plan 批次 7 = S7（低风险 文案/列名/列序/默认值）+ S14 中属**输出类**的部分；S14 的参数/退出码类已划出到另开会话。
> **分支**：worktree `.worktrees/s7s14-low-risk`，`fix/s7s14-low-risk`（base `cf1baec`），12 个主题提交，已线性 ff 进 `refactor/edition-2024`（**tip `aed0734`，无 merge commit**）。
> **报告**：`.agents/docs/active/audit/s7-s14-investigation-2026-09.md`（逐项对照、`status` 实测前后、config 别名核查、合并就绪核查）。

| 报告位置 | 处置 |
|---|---|
| §4.1 `cache` 输出行 | **已修**：`cache rm` 改为逐文件 `Removing <file>...` + 末尾 `Deleted: N files, <size>` |
| §4.2 `cleanup` 输出行 | **已修（S7）**：逐应用 `Removing <app>: <versions>` + 结尾 `Everything is shiny now!` ⇒ **2026-09-20 再改**：逐**版本**一行 spinner（`✓ app: 1.0.0` / `✗ app: 1.0.0 (err)`），失败改末尾 `WARN` 汇总、`✓ 一切正常！` 仅零失败时打（`.agents/docs/active/spec/cleanup-parallel-spec.md`）。**有意偏离（登记）**：官方逐应用黄行 + 首个失败中断整批 exit 1；hok 逐版本行 + 容错清理到底 + exit 0 |
| §4.3 `install` 已安装文案 | **已修**：`'<name>' (<version>) is already installed.` + `Use 'hok update <app>' to install a new version.`（`--global` 带后缀）；成功页脚 `Everything is ok!` **2026-09-13 晚已删除**（官方 install 无此页脚，原登记为有意偏离 → 改为对齐；`status` 的那处保留，官方确有） |
| §4.5 `update` | 全最新文案**已修**（逐应用 `app: ver (latest version)` + 结尾提示）、`--no-cache` 无 `<app>` **已修**（exit 1）、`upgrade -f` **已改 force**；`-g -a`/`-g *` 两趟仍**暂缓** |
| §5.1 `list` | 表头回显查询、`Updated` 列、Info 词按命令分语境**已修**；升级箭头 `→ <new>`、计数页脚、多查询 OR、`-V/-e/-u/-H`、本地+全局合并**登记有意偏离/暂缓** |
| §5.2 `search` | 无查询列出全部**已修**（无匹配 exit 1 已由 S1 修）；输出形态、多词 OR、默认匹配 bin、远程 known bucket **登记有意偏离/暂缓** |
| §5.3 `status` | 列名 `Missing Dependencies`、缺依赖 ` \| `、Info 词 `, `、全好文案 `Everything is ok!` **已修**；`Source` 列、attention 页脚、`:: <标题>` 分节标题、额外来源异常词**登记 hok 扩展**；`-l/--local` + Scoop/bucket 新鲜度检查**暂缓**（属新功能） |
| §6.2 `bucket` | 列序 Name/Source/Updated/Manifests、known bucket 排序、`known` 裸名称、known 名大小写不敏感**已修**；`list -k`、`rm` 多名称登记偏离；重复仓库检测、git URL 校验、添加流程 `Checking repo... ` + `ls-remote` 预检**划出到 S14 会话** |
| §6.5 `shim` 输出 | `list`/`info` 对象形状（Name/Source/Alternatives/IsGlobal/IsHidden）**已修**；无子命令默认 list、无 `alter` 登记偏离；`-g` 位置、`list -g`、`list *` **划出到 S14 会话** |
| §4.6 / §10.3 `reset <未安装>` | **已修**：报 `'<app>' isn't installed.` 后 **exit 0** |
| §7.6 `download` | 长选项主名 `--skip-hash-check`**已修**（并删除 `--no-hash-check` 别名）；`-f` 打印 `Cache is being ignored.` **已修**；进度/缓存命中/哈希失败详情文案**暂缓** |

**同批顺带的命名对齐（破坏性，详见 fix plan §6「S7 + S14 落地记录」）**：`--no-hash-check` 删别名、`-D/--ignore-cache` → `-k/--no-cache`、`--ignore-failure` 去短选项、`upgrade -f` = force、`--skip-hash-check`/`ignore_failures`/`no_cache` 内部改名、config 设置键统一下划线并删除 serde 别名。

---

## `cleanup -l/--list` + 帮助面收尾（2026-09-22，4 提交 `cd67788`/`65b83ee`/`2eba651`/`daa1002` → 主线 `daa10020`）

| 位置 | 处置 |
|---|---|
| §4.2 `cleanup` 选项集 | **hok 扩展（登记）**：新增 `-l/--list` 列出指定 app 的**现存**版本（官方 `scoop-cleanup.ps1:9-12` 的 getopt 只有 `agk`，无对应选项）。取值复用 `package::list::list_installed_versions`（= `hok list --versions` 同一入口），渲染为其减掉 源/更新时间 两列；只读 ⇒ 不要求管理员、裸 `-l` = 全部已装 app、`-k` 与它 clap 冲突。**`2eba651` 补**：解析不出当前版本时（broken 安装，如本机 potplayer 缺 `manifest.json`/`install.json`）首行报「当前版本未知」而非 `旧版本` —— 只读清单不该把 junction 目标断言成旧版本 |
| §4.2 `cleanup` 私有 `-v/--verbose` | **删除**：`-v` 是全局 verbosity（`clap_verbosity_flag`），只有 `cleanup`/`info` 自声明同名参数盖住共享文案；连同它门控的「已清理干净」报告与 `cmd.cleanup_already_clean`/`cli_help.cleanup.args.verbose` 两个键一并删除（`cleanup -v` / `-v cleanup` 仍合法，只是语义回到全局详细程度） |
| §4.2 `cleanup` 帮助文案 | **已修**：`-a/--all` 补「（相当于 '*'）」（对齐 `upgrade` 同款措辞）；`[APP]...` 删掉「（留空表示全部）」——裸 `cleanup` 自 S6 起即报 `<app> missing` exit 1，原文案是错的 |
| §4.1 `cache` 帮助面 | **已修（i18n，非官方对照项）**：顶层 `[APP]...` 缺 `cli_help.cache.args.app` 键；`list`/`remove` 的位置参数键仍是字段改名前的 `list.query`/`remove.query` ⇒ 三处都渲染英文 doc comment。补齐/改键，并把 `tests/cli_help_i18n.rs` 的位置参数检查从 `-`/`<` 扩到 `[ARG]...`（原先跳过方括号行，正是它让这三处漂移而测试全绿） |
| §5.1 `list -V` 的版本注记 | **已修（`daa1002`）**：`list --versions` 的 retained 行原先无条件打 `旧版本`，对 broken 安装（`is_current` 全 false）与 `cleanup -l` 犯同一个越权断言（实机 potplayer 的 junction 目标 260819 被标成旧版本）。判定上移到 `src/cmd/mod.rs` 的 `VersionNote`/`version_note`（与 `status_info_word` 同处，两个表共用一条规则），首行报「当前版本未知」；同时**这些目录不再计入页脚「保留了 N 个旧版本」**（`cleanup` 本就跳过这类 app，它们不是可清理的旧版本）。健康 app 的 `旧版本` 与计数不变。**遗留**：页脚 `list_summary_versions` 的 `apps` 取的是「显示的应用总数」（既有口径，非本次引入） |



## `cleanup -k` 缓存面三修（2026-09-25，2 提交 `ff05b80`/`cb75ba4` → 主线 `cb75ba43`）

**起因**：用户实机反馈「`hok cleanup * -k` 貌似没同步删缓存」。对照实验（真机 74 个 app / 25 个缓存文件按结构镜像到临时根）证明主路径正确，但沿"漏删"线索找到三个真缺口。

| 位置 | 处置 |
|---|---|
| §4.1/§4.2 缓存文件名匹配 | **已修（`ff05b80`）**：`REGEX_CACHE_FILE` 的 name/version 字符类 `[0-9a-zA-Z._-]+` 使含 `+`/`_` 的版本号（`1.9.11_2007`、`1.4.3-canary.1+29d9638da`；官方 buckets 里 250+ 个 manifest）与 CJK 包名的缓存文件**不可表示** ⇒ `cache`（列表，总账也少算）、`cleanup -k`、`cache rm <app>` 三处都跳过，只有 `cache rm *`（直接遍历目录的分支）能删。改为 `^[^#]+#[^#]+#`：对齐官方 `Remove-Item "$cachedir\$app#*` 的 glob 语义（`scoop-cache.ps1:28,45`/`scoop-cleanup.ps1:33`），并用整名锚定替换掉原先的"子串也能命中"。CJK 名挂账见 `.agents/docs/active/plan/pending-tasks.md`（已勾掉） |
| §4.2 `cleanup -k` 的 broken app | **已修（`cb75ba4`）**：current 版本解析不出的 app 原先整包跳过 ⇒ 缓存也不删。官方此时仍执行同一行 `Remove-Item`，而 `-Exclude "$app#$null#*"` 匹配不到任何文件 ⇒ 该 app 缓存**全删**。hok 对齐缓存部分（`cache::remove_all_for_app`），**版本目录仍不动**：官方 `Where-Object { $current_version -ne $_ }` 在 `$null` 下恒真、会连版本目录一起删，hok 保留"判不出当前版本就不删目录"的安全侧行为（**有意偏离，继续登记**） |
| §4.2 `cleanup -k` 的输出 | **hok 扩展（登记）**：`-k` 原先对缓存零输出（官方同样静默），"只删了缓存"与"什么都没做"在屏幕上无法区分。现在收尾打一行 `过期下载缓存已删除：共 %{count} 个文件，%{size}`（`cmd.cleanup_cache_removed`）——**零删除也打**，于是静默只可能意味着没加 `-k`。官方无此行 |
| §4.1 `cache rm <app>` 的覆盖面 | **随 `ff05b80` 一并扩大**：`*.download` 残留与含 `+`/`_` 的版本名现在同 `remove_outdated` 一起被 `cache rm <app>` 与 `cleanup -k` 看见并删除（官方 `^$app#` 本就能命中它们）。aria2 sidecar `<app>.txt` 的归属不变：只有 `cache rm` 删，`cleanup -k` 不碰 |
| §4.1 部分下载残留（`.parts`） | **hok 扩展（登记，`7b92635`）**：`remove_incomplete` 除上游的 `*.download` 外，还递归删除 `<缓存名>.parts` 目录。官方 `Remove-Item "$cachedir\$app#*"` **不带 `-Recurse`** ⇒ 非空 `.parts` 目录上游清不掉（PowerShell 对非空目录报错跳过）；而中断的分片下载**没有缓存条目可依附**，只有按目录形状扫描的兜底入口能覆盖。输出上 hok 另起一行 `残留下载分片已清理：共 N 项，X`（官方无对应输出，属 hok 扩展） |

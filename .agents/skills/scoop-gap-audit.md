---
name: scoop-gap-audit
title: Scoop 对标差距与进度总表（含 manifest 解析专题）
description: Scoop 对标差距与修复进度总表（含 manifest 解析单入口）：P0 全修、P1 剩 9、P2 已修、P3 剩约 8；带 ✅/⏸ 状态。
metadata:
  type: project
---

> **⚠️ 门禁命令的时效（2026-09-19 起）**：本文按时间追加。2026-09-19 之前的门禁记录写成
> `cargo clippy … --exclude innospect --no-deps`，理由是"vendored `crates/innospect` 的既有告警会挡住
> 整包 `-D warnings`"。**该理由已失效**——vendored fork 已退役，改用 registry `innospect 0.1.4`
> （见 §门禁更新 2026-09-19 与 `.agents/docs/legacy/nsis-issue.md`）。**当前门禁**：
> `cargo clippy --workspace --all-targets -- -D warnings`（`--no-deps` 可选，仅提速）。
> 带 `〔pre-2026-09-19 …〕` 标记的行只作历史记录，**不要照抄**。

方案 C 全面对标 Scoop 的差距审计（2026-08-10，4 个 explore 子代理，file:line 详见阶段报告）+ 修复进度（2026-08 多轮，已提交）+ manifest 解析专题。编号 P0/P1/P2/P3，✅=已修 / ⏸=暂缓 / 剩余。

**P0（不兼容/静默错误）：**
- ✅ 1. `hash: ""` 空串 → 官方 warn+继续（download.ps1:719-724）
- ✅ 2. license/version/homepage 缺失 → 官方零校验容错
- ✅ 3. BOM/注释/尾逗号 JSON → 已修（宽容链，2026-09-12 起统一为 `internal::json` 单入口）
- ✅ 4. installer.file 退出码丢弃
- ✅ 5. `autoupdate.hash` 字符串形式
- ✅ 6. bin 对象格式
- ✅ P0-3. env_set/cookie 非 string value → 已修（2026-08-16 复核代码确认）：manifest.rs 三个 env_set 字段 + cookie 字段均挂 `deserialize_string_map`（manifest.rs:92-97/170/331/389），number/bool 自动转字符串，测试 `test_env_set_coerces_non_string_values`（manifest.rs:1784-1800）锁定「COUNT:3→"3"、FLAG:true→"true"」

**P1（CLI 兼容缺口）：**
- ✅ 7. -a/--arch（install/update/download）
- ✅ 8. installer.file+script 并存、args-only fallback
- ⏸ 9. update <app> 先同步 buckets（用户暂缓）
- ✅ 10. scoop 自更新（设计决策：hok 单二进制，不做）

**P2（功能缺口）：**
- ✅ download 独立命令（2026-08-12）：`download_apps`/`DownloadOptions{force,check_hash}`，逐 query 独立（@version/URL/本地/bucket），不下依赖不查已安装；`-f/--force`、`-s/--no-hash-check`、`-a/--arch`；事件闭环用专属 `Event::DownloadDone`；失败 exit 非零；缓存命名用 url_remote_filename。细节见 [[hok-audit-fix-history]]
- ✅ install_psmodule / ensure_install_dir_not_in_path（env.rs+sync_install.rs）/ installer.keep / post_install $dir —— **2026-08-15 复核确认已实现**

**P3（低/风格）2026-08-16 复核后剩约 8 项**：
- ~~✅ #10 成功提示（2026-08-13 commit「show Everything is ok! on sync success」）~~ → **2026-09-13 晚回滚**：install/uninstall/update/reinstall 的 `✓ Everything is ok!` 页脚**已删除**（官方只在 `scoop status` 打，`libexec/scoop-status.ps1:80`）；`status` 保留
- ✅ #20 hash 前缀（hash.rs:202-217 `format_hash_value` 单点实现+测试，sha256 裸写/md5·sha1·sha512 加前缀，checkhashes+checkver 共用）
- ✅ #9 退出码机制（2026-09-13 S1 闭环）：clap 用法错/缺参 2→**1**、裸命令/帮助 0，并逐命令补官方专用码（`which` 2、`bucket list` 空 2、`shim` 2/3、`virustotal` 位掩码、`search` 无匹配/`list` 空装 1）；download 失败按官方改为 0（汇总行保留）。见文末 S1+S13 条
- ⚠️ #21 架构 truthy 降级：ARM64 回退已实现（manifest.rs:597-632，Win11→64bit/Win10→32bit），其余未核对
- ⏸ #4 重复安装文案（install.rs:108-110 已有 warn「is already installed. Skipping.」，对齐度未核对）
- ⏸ #13 非 git main 转换、#15 -q/--quiet（全局有命令级无，install/update.rs 均无）、#16 输出文案、~~#17 cooldown（config.rs:465 仍 900s）~~ **✅ 2026-09-13（update-align `c2581d4`）冷却机制整体删除**（见文末 update-align 条）、#18 hok 增强

## 2026-09-12 checkver 取哈希链路复核（base64 单实现 + Digest/hash 文件对照，已提交）
- **base64 单实现**：新增 `internal/base64.rs`（`encode`/`decode`/`to_lower_hex`），删除 `checkver_hash.rs` 手写 `simple_base64_decode`（**不合规**：`filter_map` 静默丢弃非法字符、padding 用 `take(2)` 截断、尾块剩余字符直接跳过 ⇒ `YQ$===`/`YQ==extra` 静默解出垃圾而非报错）；`internal/github.rs` 编码同源。`decode` 契约 = **实测** .NET `Convert.FromBase64String`（PS 5.1 与 pwsh 7.6.5 结果一致）：忽略任意位置 ASCII 空白、要求规范 padding、**容忍非零 tail bits**（`YR==`→`61`）、空串→空。故用 `GeneralPurposeConfig::with_decode_allow_trailing_bits(true)` + `RequireCanonical` + 自行剥空白。
- **Digest 头**（官方 `lib/autoupdate.ps1:181-206 find_hash_in_headers`）逐项判定：
  - a 解码失败语义：**有意偏离**——hok 记 Err 回落 `.meta4`；官方 `FormatException` 逸出（`catch [System.Net.WebException]` 不覆盖）会中断整个 checkver。已在代码注释写明。
  - b `SHA=`/`MD5=` 分支：原只匹配 `SHA-256=`，✅ 已补（官方顺序，先匹配者胜，`([^,]+)` 不跨逗号）。
  - c 请求形状：原来 follow redirect ⇒ 读的是最终 200（通常无 Digest），该路径**实际失效**；✅ 已补 `AllowAutoRedirect=false`（`internal::network::head_no_redirect`）+ 仅 3xx 读 Digest + 2s + `Referer=strip_filename` + UA。回环测试锁定 302+Digest/200+Digest/302 无 Digest。
  - d hash 文件 base64→hex（官方 `:82-92`）：✅ 已补，仅落在 `extract` 模式 arm（对应 `find_hash_in_textfile`），不污染共用 json/xpath 抽取。
  - e `Regex::new` 每调用重建：✅ 改模块级 `LazyLock`。
- commits：`cc21264`（base64 单实现）/ `73431b3`（Digest 对齐）/ `27669df`（hash 文件 base64）。

## 2026-08-15 全量差距分析（详细报告：.agents/docs/active/audit/scoop-gap-analysis-2026-08.md）
**命令覆盖**：hok 36 ⊇ Scoop 28 全 ✓；hok 独有 9（checkhashes/checkurls/checkver/formatjson/missing_checkver/completions/reinstall/upgrade/ci-auto-pr）。
**参数缺口（Scoop 有 hok 缺）**：~~shim（-g/alter/add/rm/`--`）~~ **✅ 2026-08-15 已补 add/rm/list(正则)/info + -g + `--` 终止符**（alter 用户决策不做，2026-08-16 移除）、~~depends -a~~ ✅ 2026-08-16（depends.rs，走 session.set_default_architecture）、~~reset -a/--all~~ ✅ 2026-08-16（reset.rs 遍历 local+global、跳过 scoop、global 无 admin warn）、virustotal（-a/-s/-n/-u/-p + 组合退出码 0/1/2/4/8/16）、~~export -c/--config~~ **✅ 2026-09-13（S4）**（export 本体 2026-09-13 已对标官方 schema，官方键名 `-c` 段随 S4 落地，见文末 S4 条）、status -l、~~info -v~~ **✅ 2026-09-12（info 全面对标，见文末专题）**、hold scoop 特例（hold_update_until）。
**config 键缺口**：use_lessmsi、use_external_7zip、show_manifest、show_update_log、update_nightly、force_update、hold_update_until、autostash_on_conflict、debug、aria2-warning-enabled/retry-wait/options（多为边缘）。**2026-09-13（S4）处置**：这些键**保持不支持并登记有意偏离**，但 `config` 对它们报 `Error::ConfigKeyUnsupported`（"官方设置项、hok 无对应实现"），与完全未知键的 `ConfigKeyInvalid` 分流，不再被当成拼写错误。
**流程**：ensure_install_dir_not_in_path ✅、~~persist_permission~~ ✅ 2026-08-16（**双层拆分**：permission.rs = 纯 ACL 原语 `grant_users_write_inherit(root: &Path)`，**windows-sys 0.52 原生实现**（GetNamedSecurityInfoW+SetEntriesInAclW+SetNamedSecurityInfoW+CreateWellKnownSid；.NET Write=FILE_ADD_FILE|ADD_SUBDIRECTORY|WRITE_EA|WRITE_ATTRIBUTES，ObjectInherit=OBJECT_INHERIT_ACE；无 Session/persist 依赖，未来进程/卸载等任意目录可复用；有单元测试读回 DACL 验证）+ persist.rs = 业务守卫 **`persist_permission`**（对齐官方命名；global+persist+admin 判定 + persist 根解析 → 调 permission 原语）；挂 install(sync_install.rs:548)+reset(sync_remove.rs:353)；session.persist_root 新增）、~~checkup 语义与 Scoop 完全不同（hok=已装包健康，Scoop=环境检查）~~ **✅ 2026-09-13 已修（S5，`5ae994d`）**：`checkup` 对齐官方环境检查，原健康检查改名 `health`（见文末 S5 条）、~~cooldown 15min vs 3h~~ **✅ 2026-09-13（update-align）冷却删除**。
**建议优先级**：virustotal 参数+退出码、export -c、shim 子命令、P1-9。

## 2026-08-15 细节核实：config 边缘键 / persist_permission / 小参数（低优先 backlog，代码级，详表见 .agents/docs/active/audit/scoop-gap-analysis-2026-08.md §7）
- **config 边缘键**：缺失集合 15 个（测试锁定 12 个被拒键 config.rs:1401-1424 + update_nightly/hold_update_until/autostash_on_conflict 3 个未建模；`set()` 未匹配键→ConfigKeyInvalid config.rs:693；迁移时静默丢弃 config.rs:106-115）。**真缺口仅 3 个**：use_external_7zip（原生解压无外部 7z）、show_manifest（无安装前交互）、autostash_on_conflict（git pull 无 stash）；aria2-options/use_lessmsi/show_update_log/debug 等为架构差异或低价值；scoop_repo/scoop_branch/shim/hold_update_until 设计 N/A（单二进制）。修入口 = ConfigInner 加字段 + set() match + list_all，单文件闭环。**2026-09-13（S4）**：键名拼写严格化（只认官方规范名——下划线，`aria2-*` 连字符；砍 `aria2_split`/`no_junctions`/`script_engine`/`ignore_failure` 别名），`canonical_key` 收敛为唯一 `SETTINGS` 清单；`ConfigInner` 的 serde alias 保留，旧配置文件仍可加载。`aria2-min-split-size` 20M→5M 与 `proxy` 三态仍**暂缓**。
- **persist_permission**：官方 install.ps1:522-531（global+persist+admin → Users SID S-1-5-32-545 Write+ObjectInherit 到全局 persist 根；调用点 install.ps1:67/reset.ps1:91，幂等）。hok persist::link（persist.rs:31-92）无 ACL；**is_admin 已就位**（session.rs:197-202→internal/os.rs:55-71，IsUserAnAdmin FFI）。补齐 = 设 DACL（SetNamedSecurityInfoW 或 icacls）挂 install persist 步 + reset。
- **persist 链接三态（2026-09-20 已修，`8aa3dc5` `16a91cd`）**：`persist::link` 原先只抄了上游 `persist_data` 三态里的两态，缺「persist 目标已存在 **且** archive 自带同名数据」分支（上游 `Move-Item -Force $source "$source.original"`），遇到 sublime-text 那种 zip 自带 `Data/KEEPME` 的包会以 `os error 183` 中断安装；`persist.rs` 现已补该分支 + 5 条单测，`internal::fs::symlink_dir` 的清理错误不再被 `let _ =` 吞掉。**未覆盖**：persist 的 `-g` 全局 ACL 之外的分支（如 `$persist_dir\$source` 双元素形式）仍按原实现。
- **小参数**：depends -a（depends.rs:12-15，小）、status -l（status.rs:9-14，小）、~~info -v~~ ✅ 2026-09-12（info 全面对标）、reset -a（reset.rs:9-15，小）、hold scoop 特例（hold.rs，中，需 hold_update_until 键联动 core.ps1:1181-1197）。
- **归档**：已完成报告已移入 `.agents/docs/reports/`（README 索引），活文档 `.agents/docs/active/audit/scoop-gap-analysis-2026-08.md`（原仓库根，随 `docs/` 迁入）；失效报告自动归档规范见 [[hok-report-archive]]。

## Manifest 解析专题（单入口架构）
**所有 JSON 读取收敛到单一入口**（2026-09-12，依赖归并 A2）：`internal::json::{parse_str, parse_str_or_none}` —— `strip_bom`（`internal/string.rs`）→ serde_json 严格 → json5 回退，两者都失败时报**严格错误**。消费者：`Manifest::parse/from_json/parse_str`、`config::read_json_file`、`formatjson::{format_manifest_file, schema_warnings}`，外加 psengine/script 的行级 BOM（非 JSON）。`grep "json5::from_str"` 全仓仅剩该模块一处；唯一吞错入口 = `parse_str_or_none`（仅 `schema_warnings`，官方无对应调用点）。I/O 与字节解码**留在调用方**（官方读 API 本就不同：`ReadAllLines` 探测编码 / `Get-Content -Encoding UTF8` / `ReadAllText`）。
**宽容度契约（实测，勿再凭印象）**：`internal/json.rs::tolerance_contract` 固化 11 条三方裁决（PS 5.1.26100 / pwsh 7.6.5 / hok，2026-09-12 实测）。**hok 不是任何宿主的超集**：注释/尾逗号/无引号键比 5.1 宽，但 `{"a":01}` 两宿主都接受、hok 拒绝（JSON5 与 JSON 同禁前导零）；`{"a":1 // c}`（无换行）三者全拒。官方 `parse_json` 是 warn→`$null`（调用方报"找不到 manifest"），hok 直接 `Err`——**有意偏离**（登记的"比 Scoop 更好"项），不夹带改 warn。
改宽容度/加调用点只改 `internal::json` = 全局生效；`formatjson` 仍是 Value 层旁路（不解析语义），但解析入口同一条。
- P0-1 hash 前缀×长度强绑定 → 已宽容修复（非标准长度不再拒收）
- P0-2 checkver.sourceforge 对象强制 path 必填 → 已修（path 可选）
- **P0-4（2026-09-13 发现并已修，`eb67941`）`autoupdate.hash.mode: github`**：官方 `schema.json:51-62` 只列 8 个枚举值，但**官方运行时不校验 schema**、`lib/autoupdate.ps1:271-302` 的 `switch` 还接受 `github`（`:298-301` 取 Releases API asset `digest`，匿名无 token）。hok 用严格 serde 枚举（`manifest.rs:536`）⇒ `unknown variant` ⇒ **`parse_file` 整条 Err**；又因 `package/query.rs:120,285` 的 `filter_map` **静默丢弃**，`install/search/info` 表现为「**找不到软件包**」（这才是它难定位的原因）。实测 4 bucket 21685 个 manifest 中 221 文件/223 处命中、**220 个不可解析**。修法 = 加 `#[serde(rename = "github")] Github` + 注释 + 离线夹具（`tests/fixtures/0x7eTeamTools.json`）；**提取实现早已在 `checkver_hash.rs:166-198` 在位**（枚举全仓无生产读点，只是 schema 镜像）⇒ 不是补空壳。验收：221/221 可解析、`search` 无查询 scoop-app 17287→17507（**恰好 +220**）、其余 bucket 零变化。
- **未做待办（P0 级可见性）**：清单解析失败**不得**在查询/安装路径静默消失（`package/query.rs:120,285`）——改它等于改单入口的错误传播策略，影响全部清单扫描语义，需单独立项。
- ~~**未做待办（归 S9）**：checkver 的 `mode: github` 缺"digest 为 null / API 限流 → 回落下载 + 本地算 SHA-256"~~ → **已解决（2026-09-13 随 S9 落地，本行 2026-09-13 复核更正）**：`checkver_hash.rs:172-187` 现在 `mode: github` 取 `.digest` 失败/为 null 时返回 `None`，随即走 `download_and_compute_sha256`（下载落 Scoop 缓存 + 本地 SHA-256），不再 `Err` 中断该清单。S9 实测"hash 链无解 1350/1387 条目 → 全部可解"已包含这条。**别再按"未做"来立项。**
- ~~**已知 flaky（与改动无关）**：`package::manifest_cache::tests::serving_a_stale_cache_warns_once`~~ → **已修（2026-09-26，`9af599a`）**：根因是**闩锁放错层**——`STALE_WARNED: AtomicBool` / `MISMATCH_WARNED: Mutex<Vec<String>>` 是进程级 static，而告警发往**每会话**的 `session.output()`，且 stale 判据排在 `swap(true)` 之后（新鲜缓存也吃掉闩锁）。两处闩锁已搬进 `Session`（`mark_stale_warned` / `mark_mismatch_warned`，后者 key 仍是 `bucket/name`），`#[cfg(test)]` 的 `reset_*_warning()` 助手删除。并行 `cargo test -p libscoop cache::` 改前 15/25 红、改后 0/20 红。**别再把它记成"重跑即过"**。
- P1 autoupdate 缺字段：官方 13 字段 vs hok 5 → 已补（architecture/extract_dir/hash/notes/url + autoupdateArch）
- P2 无影响不动（hashExtraction.type deprecated、installer._comment、shortcuts 长度、version pattern）
- 已验证兼容：license/psmodule/checkver 全键/mode 8 枚举/installer/uninstaller/architecture/autoupdate.hash 三形式/bin 对象/hook script

## 2026-09-12 `hok info` 全面对标官方（已提交）
**结构**：新 `package/info.rs`（`Lookup`/`Info`/`VersionOutput`/`InstalledSize`/`DownloadSize`）负责全部装配（目录/HEAD/磁盘统计/git），`src/cmd/info.rs` 只渲染；`format::filesize` 复刻官方 `filesize`（`n1` 一位小数、严格 `-gt`、KB 以下 `n B`）。
**对齐项**：默认字段 = Name(+`(DEPRECATED)`)/Description/Version(`(Update to X available)`)/Source/Website(去尾`/`)/License/Dependencies/Updated at/Updated by/Installed/Binaries/Shortcuts/Environment/Path Added/Suggestions/Notes；`-v` 追加 Manifest 路径、Installed 版本目录全路径、Installed size（Current/Old/Persisted/Cached/Total，仅非零行）或 Download size（默认架构 URL 逐个 HEAD + `(latest version is cached)` + `Unknown (…)`）。**查询语义改 exact**（`QueryOption::Explicit`，官方 Get-Manifest 是精确文件名；旧 hok 用正则导致 `info neovim` 列出 15 个候选）。**globals 自动优先**（`installed $app $true` → `set_global`）；**standalone**（URL/UNC/本地 manifest 路径 + `$same_source`）已支持；多 bucket 命中优先“严格已安装”（install.json 的 bucket，官方即取 `$install.bucket`），否则已安装项，再否则候选清单（hok 保留）。
**实测证据（2026-09-12，与 `scoop info` 逐字段比对，值完全一致）**：7zip（默认+`-v`）、git、nodejs、uv（多版本 + Old versions/Persisted/Cached 全行）、neovim（未安装 → Download size）、`bucket/name`、本地 manifest 路径、not-found。
**有意偏离**：(a) nightly 显示 `nightly`（hok `installed_version()` 模型）而官方显示 junction 目标日期目录名；(b) `bin` 单元素数组渲染为路径而非官方 `$_[1]`=null 的 `.ext`；(c) HEAD 有响应但无 `Content-Length`（含 4xx/5xx）→ “did not send a Content-Length header”，无响应（status 0）→ “is down”（官方按 PS 异常类型分支，语义等价）；(d) 布局用 hok 统一风格 `  Label value`（对齐标签列），非 PowerShell 列表视图 `Label : value`；(e) `-v` 复用全局 verbosity（Count），故 `hok -v info x` 也会开启详细视图。
**关键实现点（勿踩）**：`Installed` 版本 = `<app>/*/install.json` 按 mtime 升序，排除 `current` 与 `_*.old*`（versions.ps1:98）；磁盘统计**不跟随** `current` junction（`FILE_ATTRIBUTE_REPARSE_POINT`），否则重复计数；缓存命中用 `package::cache_file_name`（与下载器同一公式，已从 `download_filenames` 抽出）；未安装时 arch_specific 传 `$null` ⇒ 只用 generic 字段（`manifest.rs` 新增 `*_generic()`）；`Updated at/by` = `git log -1 -- <manifest>`（新增 `internal::git::last_commit_for_file`），非 git 回退 mtime + 文件属主（新增 `internal::os::file_owner_name`，Win32 owner SID→账户名）；`$dir/$original_dir/$persist_dir` 非 `-v` 时替换为字面量 `<root>`。
**Why:** 对标分轮进行，修复前查本记忆避免重复调研。
**How to apply:** 下一轮建议 P1-9（update 先同步 buckets）或 P3#15（-q/--quiet）。修复后更新本记忆的 ✅/⏸ 标记。修 manifest 解析 bug 改 `ManifestSpec` 层 / `internal::json`（单入口自动覆盖）。官方参照 `$env:SCOOP\apps\scoop\current\libexec\` 或本地 `../Scoop\libexec\`，勿联网。

## 2026-09-12 全命令偏离审计（checkup 之外，报告已落 `.agents/docs/active/audit/command-deviation-audit-2026-09.md`）
**口径**：4 组并行子代理**静态读码**逐命令对照本地官方 0.5.3，共约 280 条；分类 = 语义不同 / 行为缺失 / 默认值不同 / 输出或文案不同 / 退出码不同 / 边界处理不同 / hok 独有命令 / 有意偏离。**除 alias/export/aria2/config 键白名单 4 条已人工复核外，其余未实测**——动手前先按 [[investigate-first]] 复现。
**"下一个 checkup"级（语义不同，待用户决策）**：① `alias` 只写 config、**不生成 `shims\scoop-<name>.ps1`，别名不可调用** —— **2026-09-13 已修（`3c90095`）**，见文末 S2 条；② `export`/`import` 与官方 Scoopfile **schema 互不兼容**（hok `{buckets:映射, apps:按bucket分组}` vs 官方数组 + Info 字段；hok `import` 期望对象、官方是对象数组 ⇒ 官方文件导不进来）—— **2026-09-13 已修**（单一官方 schema + `--hok` 扩展；见文末 S3 条）；③ `config` 键白名单化 + `unset` vs `rm` + 无单键读取 + `proxy none` 语义相反；④ `which` 打印 shim 自身而非真实目标、不查 PATH/全局；⑤ `cleanup` 无参=清全部、`-k` 清空整缓存 —— **2026-09-13 已修（S6）**：裸 `cleanup` 报 `<app> missing` exit 1；`-k` 逐 app 删 `app#*` 保留当前版本 + 收尾清 `*.download`；`-g` 补管理员门、`-a -g` 清本地+全局；⑥ `cache` 必须带子命令 + 子串匹配（`rm zip` 命中 7zip）—— **2026-09-13 已修（S6）**：裸 `cache`/`cache <app>` 默认列缓存，匹配改按 app 名锚定并支持多 app，按 app 删除顺带清 aria2 `<app>.txt`；见文末 S6 条；⑦ `download` 的 `aria2-enabled` 在 hok 是内置分片开关（**从不调用 aria2**）；⑧ `virustotal` 默认行为相反（官方查 `/files/{hash}`，hok 总提交扫描）；⑨ `missing-checkver -s` 与官方 `-SkipSupported` 语义相反；⑩ `reset` 不识别全局安装 / 不接受 `app@version`；⑪ `uninstall`/`update` 遇运行中进程 **中止整批 exit 1** —— **2026-09-13 已修**（改为跳过继续；`ignore_running_processes=true` 时**警告后继续** = 官方 `test_running_process` 的 ignore 分支 `return $false`；held 跳过/被依赖拒绝保留为有意偏离。**同日回正**，见文末「运行中进程输出 + ignore_running_processes 语义回正」条）；⑫ `install`/`update` 的 `--independent` 是 `-I`（官方 `-i`）。
**退出码口径（2026-09-13 已更新）**：官方源码里 libexec 的 `exit N` 只有经 **`.ps1` shim 的 `exit $LASTEXITCODE`** 才成为进程码（cmd 路径丢码），`abort` 默认 1；hok 现按官方口径逐命令对齐，详见文末 S1 条。
**已知真 bug 候选** → **2026-09-13 已核实并修复（S4，`38ecfa9`）**：`53e35f3` 只把 getter 默认改成 `false`，默认列/locale 长帮助/两份 README 漏改；现全部为 `false`，并新增守卫测试（新配置下每个 bool 行的默认列必须等于 getter）。
**修复入口**：`.agents/docs/reports/command-deviation-fix-plan-2026-09.md`（原仓库根，现 `.agents/docs/`）= 新会话拆分计划 S1–S14（每条含范围/决策门/影响面/提交拆分/验收）；一条会话只做一个主题，开工引用「按该计划 S<n> 执行」。

## 2026-09-13 S2 `alias` 可用性（已闭环，提交 `3c90095`，分支 `fix/alias-availability`）
**决策**：方案 **A 官方式脚本别名**（用户 2026-09-13 拍板），附带项全选：重名 add / 不存在 rm 报错 exit 1、拒绝内建重名、去 60 字符截断、裸 `alias` 保留 list 并登记有意偏离。
**实现**：`crates/libscoop/src/alias.rs` = alias 生命周期唯一入口（`add`/`remove`/`list`/`resolve`/`run`/`script_path`）；CLI 只做参数与表格。config `alias` 值 = `"scoop-<name>"`（脚本名，官方语义），**命令正文在 `<shims>\scoop-<name>.ps1`**。原 `config::alias_add`/`alias_remove` 已删除。
**关键实现点（勿踩）**：
- 脚本字节必须与官方一致：`format!("# Summary: {desc}\n{command}\r\n")`，UTF-8 无 BOM（官方 `lib/commands.ps1:65-71` 的 `"`n"` join + `Out-UTF8File`→`WriteAllLines` 追加平台换行 ⇒ 内嵌 LF + 末尾 CRLF）。
- **分发只查文件**：`<shims>\scoop-<token>.ps1` 存在即执行（官方 `command_files`/`command_path` 同理），config 仅索引；因此 `<BROKEN>` 条目自然不可调用（与官方一致），不做 config 兜底。
- 拦截点在 `src/cmd/mod.rs` 的 `try_get_matches_from` **错误路径**（`ErrorKind::InvalidSubcommand`），happy path 不变；参数拆分 `alias_invocation` 必须跳过取值的选项（`--lang zh foo` 要得到 `foo`）。非别名回落 `err.exit()`，信息与 exit 2 与旧二进制逐项一致。
- **别名脚本会出现在 `hok shim list` 里——这是对齐官方**（`libexec/scoop-shim.ps1` 的 list 本就 `Get-ChildItem -Include '*.shim','*.ps1'`），**不要**为此加排除逻辑。
- `rm`/`s`/`u` 是 hok 的 clap 顶层别名 ⇒ 官方示例 `scoop alias add rm '...'` 在 hok 会被拒绝（`hok rm` 已存在），属有意为之。
**仍未做（有意偏离）**：裸 `alias` 默认 list（官方 exit 1）；空表文案 `No aliases configured.`（官方 `No alias found.`）；`alias rm` 成功文案 hok 自有；`hok help <alias>` 不渲染别名帮助；无别名自递归保护（官方也无）。
**Why:** 别名"看起来能用实际不能用"是最容易误导用户的语义缺陷，现已闭环；此条也是 `command-deviation-fix-plan` S2 的验收依据。

## 2026-09-13 S3 `export`/`import` 与官方 Scoopfile 互通（已闭环，提交 `a61a471`/`110e160`/`2dd84b2`/`14b9c29`，分支 `fix/export-import-interop`）
**决策**（用户 2026-09-13 拍板）：**只维护一种 schema = 官方形状**，hok 专属内容放官方会忽略的两处；扩展段开关命名 **`--hok`**；未导入项的退出码**按 S1 收口**（逐项报错、批处理不失败、exit 0）。
**落地形态**：
- `export`：官方 `{buckets:[{Name,Source,Updated,Manifests}], apps:[{Name,Version,Source,Updated,Info}]}`（`libexec/scoop-export.ps1` + `scoop-list.ps1:24-63` + `lib/buckets.ps1:104-121`）；**含本地+全局两个作用域**、isolated 无条件导出、broken 安装记 `Info: Install failed`、时间戳用 `.NET` round-trip 形状（`internal::time::format_last_update`）；`-a` 删除。
- `export --hok`：追加顶层 `hok`（`config` 原样 − `alias` 索引 + `aliases[{Name,Command,Summary}]`）与逐 app `Hok`（`Hold`/`Global`/`Architecture`/`Failed`）。**不给 app 加 `Url`**：`Source` 已完整表达，避免双权威。
- `import`：官方数组 + hok 旧 map（含最旧 `buckets` 嵌套变体）双形状；顺序＝config → buckets → apps；输入支持本地/URL/UNC；**按 (global, arch) 分组**安装（hok 的 `--global`/`--arch` 是会话级，无 arch 覆盖的组先跑，因为覆盖是进程级且公开 API 无法回退）；装后对被 held 的包 `hold`；**按实际已安装统计**（`IgnoreFailure` 下批处理仍成功，不能用提交数冒充成功数）。
- 逐条报告（原因）的跳过项：非 git/已存在 bucket、config 官方独有键与机器相关键、`Install failed` 的 app、`<auto-generated>` 缺版本、来源 bucket 不可用、别名无命令或已存在。
**关键实现点（勿踩）**：
- **先修光标转义**：`src/bin/hok.rs` 与 `eventloop.rs` 的 `CursorGuard` 曾在 stdout 被重定向时写 `ESC[?25h` ⇒ `hok export > f.json` 产物 `trailing characters`，官方 `ConvertFrom-Json` 与 hok 自己都读不了；现在只在 `is_terminal()` 时写。
- `constant.rs` 是 Scoopfile 词汇的唯一来源：`AUTO_GENERATED_SOURCE`、`INFO_*`（导出写、导入认），且必须是 `const` 而非 `static`（否则 `match` 变成绑定，编译报 E0530）。
- `hok` / `Hok` 扩展字段官方会忽略（官方只读 `config`/`buckets`/`apps` 与 app 的 5 个字段），所以带扩展段的文件仍是合法 Scoopfile —— 实测官方导入逻辑对两种文件推导出相同安装参数。
- 别名命令必须保行结构：`alias::command` 是展示用的空格拼接，重建脚本要用新的 `alias::body`（拼接会改变 PowerShell 语义）。
**仍在（有意/暂缓）**：`scoop export -c`（官方键名 config 段）随 S4；`AssumeYes`+`IgnoreFailure` 与官方"逐 app 普通 install"不同（登记为有意偏离）；非 git 本地路径 bucket 只能跳过（`bucket add` 仅 git 远程）；`Install failed` 的 app hok 跳过（官方会重装）。
**Why:** export/import 是"换机迁移"的唯一通道，此前两个方向都不通（官方文件静默空跑 exit 0）；本条的判据是"官方能读我们的文件、我们能读官方的文件并说清哪条没导入、hok 自己往返无损"。

**How to apply:** 后续 S7 文案批量可顺带对齐空表/成功文案；改 alias 行为改 `libscoop::alias`（单入口），别再在 CLI 里直接读写 shims 目录或 `alias` 配置键。详见 `.agents/docs/reports/s2-alias-investigation-2026-09.md` §7。

## 2026-09-13 S1+S13 退出码口径 + 安装/卸载批量行为（已闭环，4 commit，分支 `fix/exit-code-and-batch-policy`，worktree `.worktrees/s1-s13`）
**落地形态**：4 个提交 `4a1fd5a`（clap 层）/ `e7d1e25`（逐命令）/ `a4ea0b7`（批量行为）/ `cfd44ed`（download）——**先 rebase 到 S2 的 `3c90095`，再线性 ff 进 `refactor/edition-2024`（tip `cfd44ed`）**；本仓历史保持线性，**不使用 merge commit**。并行/合并/未跟踪文档同步的通用规则见 [[worktree-parallel]]（hok 细则见 [[hok-worktree-parallel]]）。
**决策**：S1 选 **A 全面对齐官方逐命令退出码**；S13 只对齐**运行中进程**与**未安装应用**两类，`scoop` 自身不加特例，held 跳过/被依赖拒绝**保留**为有意偏离。处置逐条映射见 `.agents/docs/active/audit/command-deviation-audit-2026-09.md` §10。
**实现要点（勿踩）**：
- **clap 层**（`src/cmd/mod.rs::exit_for_parse_error`）：不再用 `get_matches_from`（会自行 exit 2），改 `try_get_matches_from` + 自渲染；`DisplayHelp/DisplayVersion` 与「**无子命令 token** 的 `DisplayHelpOnMissingArgumentOrSubcommand`」→ **0**，其余 → **1**。`hok -v`/`hok --lang en`（只带全局选项）仍为 1（官方无对应场景）。
- **运行中进程**：`check_not_running` 打 WARN 后由调用点决定——默认返回 `RunningCheck::Running` 跳过该包，`ignore_running_processes=true` 时返回 `RunningCheck::Ignored` **继续**（官方 `lib/install.ps1:534-551`：默认分支 `return $true` → 调用者 skip，ignore 分支 `return $false` → 调用者继续）。进程明细走 `Output::Detail`，只在 `-vv` 渲染。**注**：本条最初落成"两支都跳过、该键只决定 warn/error 文案"，系把 `return $false` 误读为"仍在运行"，当日已回正，见文末同名单条。
- **未安装应用**：`remove()`/`install()`(only_upgrade) 报 `cmd.not_installed` 后 `continue`，不再 `Err(PackageNotFound)`；原先为它而设的「是否真有已装目标」抑制 `Everything is ok!` 尾行的守卫，已随该页脚整体删除而**一并移除**（2026-09-13 晚）。
- **shim add**：`add_custom` 现在校验目标存在（官方 `Test-Path`），新增 `Error::ShimTargetMissing` → CLI exit 3；`shim info` 未找到时先翻转 scope 再查，命中另一作用域 → 2 + 提示。
- **virustotal**：按 query 未命中数置位 8（可与 2/4 叠加），`queries.clone()` 传入 query（原先被 move）。
**测试**：新增 `tests/exit_codes.rs`（隔离 `SCOOP`/`USERPROFILE` 空根，锁 `hok`=0、未知命令=1、缺参=1、`which`=2、`bucket list`=2、`shim`=3、`virustotal`=8、未安装=0）；libscoop `check_not_running_skips_by_default_and_continues_when_ignored`（复制 `cmd.exe` 到 `apps/<n>` 后跑 `ping`，断言默认 skip、开键 continue，两者都只 WARN 且明细跟在消息之后）。`tests/cli_help_i18n.rs` 的用法错断言 2→1。
**已知环境噪音**：`cargo test --workspace` 里 `permission::tests::grant_users_write_inherit_*` 失败（临时目录 ACL error 5），**在未改动的主树同样失败**，与本次无关。
**⚠️ 合并注意（已解决）**：与 S2（`3c90095`）在 `src/cmd/mod.rs` 的 parse 错误路径冲突——**已按此解决**（rebase 到 `3c90095`，tip `cfd44ed`，线性 ff 无 merge commit）：保留 S2 的 `InvalidSubcommand`→alias 分发，未命中回落 `exit_for_parse_error(err, &raw)`（**没有**保留 `err.exit()`，否则退出码退回 clap 2）；`normalize_subcommand_case` 形参改为 `&[OsString]` 并复用 S2 的 `command_names`；`src/lib.rs` 的 `translate_error` 同时保留 S2 的 alias 错误臂 + 新增 `ShimTargetMissing`。合并后实测：`hok`=0 / 未知命令=1 / 缺参=1 / `search` 无匹配=1 / 空 `bucket list`=2 / `shim rm`=3 / `virustotal`=8 / 未安装=0，且 S2 别名仍可分发执行（`hok <alias>` 透传退出码）。
**Why:** 退出码是脚本兼容面；「运行中进程中止整批」与「`ignore_running_processes` 同名异义」是两处会误导用户的语义错位。
**How to apply:** 剩余语义绑定的退出码（`reset <未安装>`、`shim`/`alias` 无子命令）随各自会话（S7/S14）一起改（`cleanup` 无参、裸 `cache` 已由 S6 落地，见文末 S6 条）；新增命令若需专用码，用 `crate::ExitCode(n)` 而不是新开关。

## 2026-09-13 S5 `checkup` 语义决策（已闭环，提交 `5ae994d`，分支 `fix/checkup-semantics`，worktree `.worktrees/s5-checkup`）

**结论（用户拍板）**：**B&C 两者并存** —— `checkup` 对齐官方环境检查，hok 原已装包体检改名 `health`；`checkup` **恒 exit 0**（对齐官方 `libexec/scoop-checkup.ps1:58`）；Defender 用**纯 Rust**（WinDefend 服务状态 + `HKLM\SOFTWARE\Microsoft\Windows Defender` 实时/排除项），读不到即降级（不误报）。
**实现**：
- 新增 `crates/libscoop/src/package/diagnostic.rs`：纯 `evaluate(&EnvFacts)` 决策层（10 条离线单测）+ `diagnose(session)` OS 采集。检查项与顺序对齐官方：管理员+Defender 性能（local/global，仅 admin 且非 `WDAGUtilityAccount`）→ main bucket → 长路径（`os_major<10 || build<1607` 判"不支持"，否则看 `LongPathsEnabled`）→ 开发者模式（`AllowDevelopmentWithoutDevLicense`）→ 7zip/innounp/dark（复用 `resolve::helper_installed`，改为 `pub(crate)`）→ 全局根再本地根的 NTFS。
- `crates/libscoop/src/internal/os.rs` 新增探针：`windows_version`、`registry_key_exists`/`registry_dword`/`registry_subkeys`（winreg）、`is_service_running`（advapi32 `OpenSCManagerW`/`OpenServiceW`/`QueryServiceStatus`）、`volume_is_ntfs`（kernel32 `GetVolumePathNameW`+`GetVolumeInformationW`）。
- `package/checkup.rs` → `package/health.rs`（`CheckupIssue`→`HealthIssue`）；`src/cmd/checkup.rs` 改为环境诊断、新增 `src/cmd/health.rs`；`src/cmd/mod.rs` 加 `Health` variant；locales 中英双语新增 `cmd.checkup_*`、`checkup_issue_count`→`health_issue_count`、`cli_help.checkup.about` 改官方措辞 + `cli_help.health.about`。
- **顺带修 bug**：原 `checkup` 的 `!apps_dir.exists()` 分支不可达（`check_installed` 先返回空列表）——`health` 里改为先判 apps 目录。
**测试**：`cargo test --workspace` 仅 `permission::tests::grant_users_write_inherit_*` 2 条沙箱 ACL 环境性失败（clean HEAD 同样失败）；`diagnostic` 10 passed；隔离 `USERPROFILE` 实测 `checkup` 6 问题 exit 0、`health` 命中"无 apps 目录"分支 exit 0；clippy 新增 0、rustfmt 干净。
**Why:** `checkup` 同名异义会让按官方文档/脚本操作的用户得到完全不同的结果；hok 的已装包体检另有价值，直接删除是净损失。
**How to apply:** 环境类检查统一走 `package::diagnostic`（新增检查项加进 `evaluate` + 单测，OS 读取加进 `internal::os`）；hok 未建模 `use_external_7zip`，故 7zip 恒按需检查——若将来支持该键，需在 `diagnose` 里给它加分支。

## 2026-09-13 S6 `cleanup`/`cache` 数据安全（已闭环，提交 `1e6b929`/`33ba858`/`1c1311b`/`cc2c072`，分支 `fix/cleanup-cache-safety`，worktree `.worktrees/s6-cleanup-cache`）

**结论（用户拍板）**：三项全选 + 破坏性变更都对齐官方 —— ①数据安全（`-k`、`cache` 匹配、sidecar）；②`-g` 管理员门 + `-a -g` 双作用域；③无参语义与未安装文案（含裸 `cache` 默认列缓存）；输出样式留 S7。
**实现**：
- `crates/libscoop/src/cache.rs`：选择改 `is_wildcard`/`matches_apps`（按 `package_name()` 大小写不敏感全等 = 官方 `^<app>#` 锚定，多 app 任意命中）；`list`/`remove` 收 `&[String]`；`remove_entry` 统一容错；按 app 删除顺带清 `<app>.txt`；新增 `remove_outdated(app, keep_version)`（`cleanup -k`）与 `remove_incomplete()`（`*.download`）。
- `crates/libscoop/src/package/cleanup.rs`：新增 `CleanupOptions { ignore_failure, cache }`；**目标列表改为始终显式**——`cleanup` 不再把空名单当"全部"，新增 `installed_app_names` 供 `-a` 枚举；`confirm_installation_status` 复刻官方 `Confirm-InstallationStatus`，`install_failed` 复刻官方 `failed`（`current` junction 存在 + 版本可解析，**不要求 `install.json`**）。
- `src/cmd/cache.rs`：`command: Option<Command>` + 顶层 `app` 位置参数（`args_conflicts_with_subcommands`），裸 `cache`/`cache <app>` 默认 list；`rm` 无参报 `<app(s)> missing` exit 1。
- `src/cmd/cleanup.rs`：裸调用报 `<app> missing` exit 1；`ensure_global(session, global, "cleanup")`；`-a` 本地一趟、`-a -g` 追加全局一趟（结果合并、收尾只打一次）；命名 app 走 confirm 后清理，全部无效时不再多打"无旧版本"。
- locales 新增 `cmd.cache_app_missing`、`cmd.cleanup_app_missing`、`cmd.cleanup_not_installed_globally/locally`、`cmd.cleanup_try_local/global`；删除不再使用的 `cmd.cache_cleaned`。
**测试**：隔离 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE` 环境进程级实测 10 条（`rm zip` 不再命中 `7zip`、`-k` 保留当前版本、裸 `cache`=0、裸 `cleanup`=1、`-g` 非管理员=1、`cleanup ghost`/跨作用域不再误清全盘）；新增 libscoop 单测 6 条；`cargo test -p hok` 全绿、`--test exit_codes` 4/4、`--test i18n_keys` 3/3；clippy 新增 0、rustfmt 干净（`--edition 2024`）。
**Why:** `-k` 清空整缓存 + `cache rm` 子串匹配是**数据安全**缺陷（误删其它 app 与当前版本缓存）；裸 `cleanup` 清全部更是"想清一个、清了所有"的危险默认。
**How to apply:** 缓存选择统一走 `libscoop::cache`（内部 `matches_apps`/`is_wildcard`，勿退回 `contains`）；清理目标一律显式传名单（空名单 = 无操作，**不要**再引入"空 = 全部"）；判"装好了没"用 `install_failed` 的官方语义而非 `query_installed` 的 `Installed`（后者额外要求 `install.json`，比官方严）。

## 2026-09-13 S4 `config` 键策略与子命令语义（已闭环，提交 `38ecfa9`/`f4300aa`/`4cefb81`/`b7c1680`/`7987205`/`98e28a1`/`cf1baec`，分支 `fix/config-key-and-subcommand`，worktree `.worktrees/s4-config`）

**结论（用户拍板）**：`config` 行为对齐官方 + 保留 hok 扩展 —— ①键名**只认官方拼写**（下划线；`aria2-*` 连字符），砍掉 hok 别名；②未知键**保持严格**但分流报错（官方独有键 vs 非设置项）；③**删掉 `set`/`unset`/`list`**：写入 = `hok config <key> <value>`、删除 = `hok config rm <key>`、`edit` 保留为 hok 扩展；④输出三档：裸 `config` = 文件里保存的键（`key = value` 行）、`-j/--json` = 同一份 JSON、`-a/--all` = 全键表格；⑤`config -h` **不对标**官方（官方 `-h`=`--help`=153 行手册；hok 保持 clap 短/长，手册走 `--help`）。
**实现**：
- `crates/libscoop/src/config.rs`：`canonical_key` → 唯一 `SETTINGS` 清单（每键一个拼写；serde alias 保留以兼容旧配置文件）；新增 `pub fn value(session, key)`（读**存储值**，不解析默认）；`pub enum OfficialKey` + `map_official_key`（官方 → hok，只认规范拼写）+ `official_name`（hok → 官方，导出用）；`OFFICIAL_ONLY_KEYS` 清单驱动报错分流。
- `crates/libscoop/src/error.rs`：新增 `Error::ConfigKeyUnsupported`（`error.config_key_unsupported`）；`src/lib.rs` 加翻译分支。
- `crates/libscoop/src/package/export.rs`：`build_export(session, ExportSections { config, hok })`，`-c` 段 = `official_name` 过滤后的共享键（不含 hok 独有键与机器相关路径）。
- `src/cmd/config.rs`：`Args { json, all, key, value, command }` + `args_conflicts_with_subcommands`；`Command = Edit | Rm`；裸/`-j`/`-a` 三个视图 + `settings_lines` 对齐输出；配置路径提示改 **stderr**（`output::note`）。
- locales/README 同步（`config_removed` 文案、长帮助写明 hok 扩展与"无别名"、README 用法块与 `config language` 示例修正）。
**顺带修的两个真 bug**：①**S12** `ignore-failures` 默认值显示/帮助/README 的 `true`（运行时 `false`）；②**S3 键名表前提反了**——官方 `set_config` 原样存键、`get_config USE_SQLITE_CACHE` 读下划线名，原 kebab 表让真实 `scoop export -c` 的配置段基本读不进。
**测试**：新增 `tests/exit_codes.rs::config_views_and_usage_errors_exit_cleanly`、`cmd::config` 的 `settings_lines_align_the_value_column`、libscoop `value_reads_back_every_setting`/`boolean_default_column_matches_the_runtime_default`/`config_section_carries_the_shared_settings`；`--test exit_codes|cli_help_i18n|i18n_keys` 全绿；`cargo test --workspace` 仅 2 条 ACL 环境性失败（clean HEAD 同样失败）。
**Why:** 官方 `scoop config` 的四形态是脚本与文档的公共契约（`config <k> <v>` / `rm` / 单键读取），hok 自造 `set`/`unset`/`list` 让按官方文档操作的用户处处踩空；键名别名则让"同一个设置两个名字"，导入导出无法共用一张表。
**How to apply:** 设置键名与拼写统一走 `config::SETTINGS` / `canonical_key`（**勿再加别名**，要加键就加进清单 + `set()` arm + `list_all_rows` + 描述三元）；官方↔hok 键映射只在 `config::{map_official_key, official_name}` 一处，import/export 共用；对官方独有键要给出 `ConfigKeyUnsupported` 而非 `ConfigKeyInvalid`。**遗留**：`aria2-min-split-size` 20M→5M、`proxy` 三态（`none` 存字面值 + 直连、`default`/`currentuser` 不生成非法 URL、hok 无系统代理）尚未做。

## 2026-09-13 S7 低风险批量：文案 / 列名 / 列序 + 标志命名对齐（已闭环，12 提交，分支 `fix/s7s14-low-risk` → 线性 ff 进 `refactor/edition-2024`，**tip `aed0734`**，无 merge commit）

**结论（用户拍板）**：批次 7 只做 T1+T2（纯文案/列名/列序/默认值 + 小参数/小退出码）；并行用 **P2（单 worktree 串行）**；`status` **保留** `Source` 列与 `:: Status of installed apps` 分节标题；`shim` 无子命令**保留**默认 list；`upgrade -f` 改为 **force**；**S14 的参数/退出码类划出到另开会话**（`bucket` 重复检测 + URL 校验 + `ls-remote` 预检、`shim -g` 位置 / `list -g` / `list *`）。
**实现**：
- 标志/命名：`--skip-hash-check` 成唯一拼写（删 `--no-hash-check` 别名），内部 `skip_hash_check` / `SyncOption::SkipHashCheck` / locale 键同步；`--ignore-failure` 去掉短选项 `-f`（与官方 force 冲突），内部 `ignore_failures` / `IgnoreFailures`，帮助文案写明 `hok config ignore_failures true`；`-D/--ignore-cache` → `-k/--no-cache`（内部 `no_cache` / `NoCache`，描述用官方 `Don't use the download cache`）；`upgrade` 新增 `-f/--force`、修好之后 `execute_upgrade` 的 force 传参。
- config：三个 hok 自有设置键统一为下划线 `ignore_failures` / `no_color` / `script_engine`（S4 规则：官方拼写下划线、`aria2-*` 连字符），并**删除全部 serde 别名**；`canonical_key` 单表仍是唯一入口；`aria2-*` 的 `serde(rename)` 与 `match` 保留。
- `status`：列名 `Missing Dependencies`、缺依赖分隔 ` | `、Info 词分隔 `, `、全好文案 `Everything is ok!`。
- `list`：表头回显查询（`Installed apps matching '<query>'`）、新增 `Updated` 列（列序 Name/Version/Source/Updated/Info）、Info 词按命令分语境。
- `cache rm` 逐文件 `Removing <file>...` + 末尾 `Deleted: N files, <size>`；`cleanup` 逐应用 `Removing <app>: <versions>`。
- `search` 无查询列出全部；`download -f` 提示 `Cache is being ignored.`；`reset <未安装>` exit 0；`update` 逐应用 `app: ver (latest version)` + 结尾提示、`--no-cache` 无 `<app>` exit 1；`install` 已安装文案带版本（`prune_installed` 改为返回 `(name, version)`）。
- `bucket` 列序 Name/Source/Updated/Manifests、known bucket 排序提前、`known` 裸名称、known 名大小写不敏感；`shim list`/`info` 对象形状 Name/Source/Alternatives/IsGlobal/IsHidden。
**测试**：`cargo test --workspace` 仅 2 条 ACL 环境性失败（clean HEAD 同样失败）；`--test i18n_keys|cli_help_i18n|exit_codes` 全绿；`clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings`〔pre-2026-09-19 写法：理由已失效，见顶部注记〕 干净（内置 `innospect` 有既有告警 `clippy::chunks_exact_to_as_chunks` ×3 与 test target 的 `unused import: sha1::Digest`，会挡住整包 `-D warnings`）；`rustfmt` 在未触碰的 `build.rs` / `hok-i18n-derive/src/lib.rs` 有既有 diff ⇒ 只做**文件级** rustfmt，且不传含 `mod` 声明的文件。
**Why:** 这批全是"同一件事两个名字/两种拼写"造成的长期漂移：CLI 主名与上游不一致、设置键与内部名不一致、`list`/`status` 共用一张词表却说不同的词。
**How to apply:** ①新增 i18n 文案必须**同时**改 `locales/en.yml` 与 `zh.yml`（`tests/i18n_keys.rs` 校验叶子键集合一致 + 每个 `t!("…")` 两语种都存在）；②改 clap 参数必须同步 `cli_help.<cmd>.args.<字段名>`（键名取自 Rust **字段名**，见 `hok-i18n-derive`），否则 `tests/cli_help_i18n.rs` 报 raw key；③设置键名只走 `config::SETTINGS` / `canonical_key`，**不要**再加别名或 serde alias；④`status`/`list` 的 Info 词只经 `cmd::status_info_word(kind, InfoContext)` 一处。
**破坏性变更**：`--no-hash-check`、`-D/--ignore-cache`、`--ignore-failure` 的 `-f`、`upgrade -f` 语义，以及配置文件里的旧设置键拼写（`ignore-failures`/`no-color`/`script-engine`/`ignore_failure`/`cachePath`/`globalPath`/`rootPath`/`lastupdate` 等，现被**静默忽略**、回落默认值）。
**遗留**：上述 S14 划出项；`status -l/--local` + Scoop/bucket 新鲜度检查（属新功能）；`install`/`update` 的 `--independent` 短选项应对齐官方 `-i`、描述改用 `Don't install dependencies automatically`。

## 2026-09-13 S8 路径解析类（which / prefix / cat / home，已闭环，提交 `c5b21d2`/`41ddaeb`/`4b25087`，分支 `fix/s8-path-commands` → 已线性 ff 进 `refactor/edition-2024`，tip `4b250876`）
**结论（用户拍板）**：which=A1 完整对齐（真实目标 + PATH 程序 + 全局 shim，只输出一个）；cat/home=B1 单入口；prefix=D1；home 无 homepage=E1；**多 bucket 交互保留（暂缓/待定）**；cat 美化与 bat 回退=F2 有意偏离；**info 必须复用同一入口且零回归**。
**实现**：
- `crates/libscoop/src/package/manifest_source.rs`：`resolve_manifest` → **公开 `get_manifest` + `GetManifest`**，逐分支复刻 `lib/manifest.ps1:34-112`：①URL/UNC（UNC 按本地文件读，官方 `WebClient` 打不开）②已安装（`install.json` 的 `bucket` → bucket 清单 + `<bucket>\deprecated` 回退；无 bucket → `url` 远程/本地 → installed manifest；**一旦已安装绝不回落 bucket 分支**）③未安装（`parse_app` 拆 `bucket/name@version`、显式 bucket 只读该 bucket、否则逐 bucket 扫描、本地文件 `Test-Path`）。`GetManifest.candidates` 是 hok 扩展（多 bucket 候选，**按 bucket 名排序**，因为 `bucket_added` 并发无序），供 CLI 保留交互选择；`bucket` 形参是 hok 扩展。
- `crates/libscoop/src/package/shim.rs`：新增 `command_path`/`classify_command_path`/`shim_target`（`.shim` 首行 `path = `、其余读 `^(?:@rem|#)`、再兜底最后一段引号串；**目标不存在即 None** = 官方 `Convert-Path -ErrorAction SilentlyContinue`）+ 9 条单测。
- `crates/libscoop/src/shim.rs`：`resolve_in_path` 改返回 `Option<PathBuf>`，顺序 **`.ps1` → `%PATHEXT%`（缺省 `exe;cmd;bat;ps1`）→ 裸名**（三档均实测于 PS 5.1 + pwsh 7）；`shim add` 与 S7 的 `shim_info`(is_hidden) 共用它。
- `src/cmd/mod.rs`：`resolve_manifest_query`（唯一 CLI 侧解析助手，含多候选交互）；`cat`/`home` 只调它；`home` 在 `open_url` **之前**判空 homepage。
- `Session::app_current_dir_in(name, global)`：`prefix` 本地→全局两次 `currentdir` 探针（勿再用 `app_current_dir` 代两个作用域）。
**测试**：隔离 `SCOOP`/`SCOOP_GLOBAL`/`USERPROFILE`/`PATH` + 自建 apps/shims/buckets，22 条 which/prefix/cat/home 用例 + 3 条 download 解析用例；libscoop 单测 11+9 条；`info` 回归 12 用例（7zip/git/nodejs/uv/neovim 未安装/`bucket/name`/本地清单/not-found/`-v`）**对 base 二进制逐字节一致**（真实 root 只做元数据副本，**勿直连 `$env:SCOOP`**——hok 打开 `<root>\scoop.db` 会建表并切 WAL）。
**已知限制/有意偏离**：多 bucket 命中保留交互（**非 tty exit 1，重名包时脚本里不可用**）；不套 `friendly_path`；不建模 PS alias/函数、不查当前目录；**全局 `scoop-*.ps1` 别名反向偏离**（实测官方 `Get-CommandPath scoop-ghello` 返回 `$null`，hok 返回脚本路径）；cat 保持 2 空格/LF 与 bat 回退；UNC 按本地文件读。
**范围外副作用（勿忘）**：`download`/`install` 的 URL/本地/`@version` 解析随单入口一并改用 `Get-Manifest`（官方 `install.ps1:9`、`scoop-download.ps1:62` 亦然），即会读安装记录——**不要回退**，回退等于给 download 单开第二条解析路径。
**How to apply:** 清单来源解析只改 `package::manifest_source::get_manifest`（**不要再引入第二条 resolve 路径**；install/download 也用它）；`which` 的命令解析只改 `libscoop::shim::resolve_in_path` + `package::shim::command_path`；多候选的 CLI 交互只在 `cmd::resolve_manifest_query`；`prefix` 的作用域探针用 `Session::app_current_dir_in`。

## 2026-09-13 S10 `checkhashes` 深度对齐（已闭环，4 提交，分支 `fix/s10-checkhashes` → 已线性 ff 进 `refactor/edition-2024`，tip `e97744a`）
**结论（用户拍板）**：缓存 **A1**（**默认行为不动**——实测 hok 默认已是"每次重下 + 跑完删临时目录"＝官方默认；只把 `-k` 缓存键修为含版本 `<cache>/<version>/…`，且 `-k` ≡ 官方 `-UseCache`，`-f` 覆盖 `-k`）；空 hash **B1**（`""` 当 sha256 期望比较 → mismatch，`-u` 写回真实值；数组逐条独立、去掉 `break`；`TODO`/未知算法/对象形式改为**显式报解析失败**，**不放宽** `HashString` 解析层以免波及 install/check_hash）；url 选择 **C1**（顶层 `url` 优先，无顶层才 64/32/arm64，**新增** `Manifest::checkhashes_accessors()`，**不动** `all_urls()`）；小项全做（`-App` 大小写不敏感、`nightly` 大小写不敏感、无 URL 打印官方文案、明细补 `First bytes` + 完整 Expected/Actual、数量不匹配不再误记 failed）。
**实现**：`package/checkhashes.rs` 重写（`Entry{algorithm,expected,path,index}` + `formatted_expected()` + `collect_entries` + `update_manifest` + `magic_bytes_of` + 15 单测）；`manifest.rs` 新增 `checkhashes_accessors()`；`internal/string.rs` `matches_any_glob` 大小写不敏感（**与 S11 同源，合并保留超集**）；`src/cmd/checkhashes.rs` 状态分流。
**测试**：`cargo test --workspace` 仅 2 条既有 ACL 环境性失败；clippy 0 新增；改动文件 rustfmt 干净。
**Why:** 这 22 条里最有杀伤力的是"静默"：空 hash 让 `-u` 永远修不了、解析失败让坏 manifest 完全消失、`all_urls()` 合并让顶层+架构清单多查一遍假 mismatch、大小写敏感让 `-App` 在大写输入下静默扫 0 个。另外原审计那两条缓存结论是**静态读码误判**，实测（换服务端内容 + 数请求）才定案。
**How to apply:** ①url/hash 选择只走 `Manifest::checkhashes_accessors()`（**不要**再用 `all_urls()/all_hashes()`，那是 checkurls 的合并语义）；②空/非法 hash 一律**不静默**、不提前 `break`；③写回走 `internal::fs::write_json`（CRLF + 末尾换行），且只写有 mismatch 的清单；④`-k` 缓存**必须按版本分目录**，否则跨版本同 URL 会假 OK。
**有意偏离（登记）**：`handle_special_urls` 不做；`-d` 默认 `.` 与多 glob；末尾汇总行；缓存放 `%TEMP%\hok-checkhashes`（不污染用户真实缓存）；`-c/--cache` 扩展；下载失败继续；`-u` 只在有 mismatch 时写回（官方无条件重写**全部** manifest）；写回只动被改文件并保留原行尾。
**划出**：D4 `download_file` 缺 Scoop 请求头（UA/Referer/cookies/PRIVATE_HOSTS）→ 已另开会话 `fix/d4-download-headers`。

## 2026-09-13 S11 第一批（formatjson + missing-checkver，已闭环，3 提交，分支 `fix/s11-formatjson` → 已线性 ff 进 `refactor/edition-2024`，tip `2a18d5b`）
- **formatjson 子命令名真 bug**：`FormatJson` 未写 `#[clap(name)]`，clap kebab 成 `format-json` ⇒ `hok formatjson` unrecognized subcommand（与官方 `bin/formatjson.ps1`、hok 自身 i18n 键、`AGENTS.md` 三处矛盾）。改为 `name = "formatjson"` + `alias = "format-json"`。
- **共享管线**：`internal::string::matches_any_glob` 按官方 `Get-ChildItem -Filter "$App.json"` 语义 = **大小写不敏感 + 忽略 `.json` 后缀**；单入口 `manifest_walker::discover_matching`，`checkhashes`/`checkver`/`checkurls`/`formatjson`/`missing_checkver` 共用。
- **派生宏共享 bug（勿踩）**：`hok-i18n-derive` 用 **kebab 化的变体 ident** 调 `mut_subcommand`，而 clap 按**最终注册名**匹配 ⇒ 一旦变体显式改名，help 阶段直接 abort。改为先读变体 `#[clap(name="…")]`、无则回落 kebab ident；`ci-auto-pr` 是同类潜在坑。
- **missing-checkver 完全对齐官方**：`-s/--skip-supported`（**语义反转，破坏性**：原"只列有 checkver 或 autoupdate 的" → 现"跳过两者都有的"）；默认输出官方三行图例 + 逐行 `[C][A] name` 真值表 **并保留 hok 汇总行**（`Scanned N manifests: x missing checkver, y missing autoupdate, z unparsable.`，N = 成功解析数）；不可解析清单出行 `[ ][ ] name` + 官方 `Error parsing JSON at '<path>'.`（官方会把 `bad.json` **截成空文件**，hok 保持原样）；app 过滤落地为**位置参数 `[APP]...`**（官方 `bin/missing-checkver.ps1:13` 为 positional，且 hok 的 checkurls/checkhashes/checkver/formatjson/virustotal 全部 positional；`-a` 是 usage error，有测试钉住）。
- **有意偏离（保留）**：`formatjson` 的 `-d` 默认 `.` / schema 告警 / 成功计数输出 / **解析失败保持原文件** / 跳过 `package.json`；`missing-checkver` 的 `-d` 默认 `.` / 跳过 `package.json`（**这是与官方真值表唯一的行差异**：官方多一行 `[ ][ ] package`）/ 行序按路径稳定排序（官方 `Get-ChildItem -Recurse` 不排序）。
- **破坏性变更**：`missing-checkver -s` 语义反转；默认输出形态变更；`matches_any_glob` 变宽容后 `checkver`/`checkhashes`/`checkurls` 的 app 过滤同样放宽（仅放宽不收紧）。
- **遗留（按用户使用面定档，2026-09-13）**：`ci-auto-pr`（§7.7）**要做**——用户**在 CI 里用它更新清单**，且它写盘并提交、受 `-d` 写盘护栏约束；`checkurls`（§7.3）**暂缓**（用户用得少，只读扫描、差异多为输出/边界）；`virustotal`（§7.5，含退出位掩码）**不做**（用户基本不用），登记为不修。**余项清单与交接说明见 `reports/command-deviation-fix-plan-2026-09.md` §8**。

## 2026-09-13 S14 `bucket` / `shim` 参数与校验（已闭环，4 提交，分支 `fix/s14-bucket-shim` → 已线性 ff 进 `refactor/edition-2024`，tip `dca2b51e`）
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

## 2026-09-13 D4 下载请求头（S10 划出，已闭环，2 提交，分支 `fix/d4-download-headers` → 已线性 ff 进 `refactor/edition-2024`，tip `6db6bfbd`）
**问题**：hok 曾有 **4～6 套互不一致的 UA 语义**，且 `download_file`（`network.rs:64`）**一个头都不发**（文件头注释却谎称已注入 UA/Referer/PRIVATE_HOSTS）；官方 GET 实测带 `user-agent="Scoop/1.0 (+http://scoop.sh/) PowerShell/7.6 …"` + `referer`。这是"单入口违规"的典型。
**实现**：唯一入口 `libscoop::network::ScoopRequest` + `internal::network::HeaderProfile`（`Download`/`CheckUrls`/`Checkver`/`Autoupdate` 四变体，每个方法注释带官方 file:line）；UA 单点 = `constant::DEFAULT_USER_AGENT` = **`Scoop/1.0 (+http://scoop.sh/) Hok/0.2.0`**（**不伪造 PowerShell 段**——实测真实源只有 GitHub API「必须非空 UA」一条硬约束，①②③ 均满足）；Referer = `strip_filename(url)`、**从不使用 bucket**，下载路径抑制 `sourceforge.net`/`portableapps.com`（照抄官方不对称）；`head_url` 不再硬编码、`github.rs` 不再固定 `hok`、`checkver_hash` 无 fallback 那处一并收敛。
**顺带修的两个真问题**：①checkver 的 **GitHub token 原本会发往任意 URL**（安全项）→ 现只在 `api.github.com`；②`checkver.useragent` 解析了却从不读取 → 现生效。
**更正的假设**：原计划 D4b 的"+cookies"**不成立**——官方 `checkhashes.ps1:122` / `autoupdate.ps1:319` 都传 `$null`，Cookie 是 no-op。（对照：私有 bucket 鉴权链路的 cookie 是另一条线，见 `hok 私有 Scoop bucket 支持` 页面。）
**验证**：本地夹具 `127.0.0.1:18877` 逐字断言 18 passed；命令回归 install/download/reinstall/update/checkhashes/checkurls 全 exit 0。
**⚠️ 环境限制（重要）**：本机**出站 HTTPS 不可用**——直连与代理 `127.0.0.1:7890` 均 TLS 失败，**纯 HTTP 可用**。故一切"真实源"验证在本机不可行，只能靠本地夹具 + 回环测试；写真实源冒烟结论前先确认这一点。
**遗留/有意偏离**：`handle_special_urls` 不做；PRIVATE_HOSTS 全入口注入（未按 ①② 拆开提交）；不伪造 PS 段。
**⚠️ 新发现的两条既有问题（基线二进制同样复现，未修，建议另开会话）**
1. `hok checkver` 在**本环境挂起**（空目录也挂）——疑与子进程/pwsh 引擎或网络等待有关。
2. `USERPROFILE` 未设时 `internal/fs.rs:63` **panic**（应降级或报错，不应 panic）。
**更正（2026-09-13 回归修复，见文末「下载默认不发 Referer」）**：D4 给下载路径加的 `Referer = strip_filename(url)` **在真实站点上是错的**——腾讯 `updatecdn.meeting.qq.com` 带它 403、不带 200，`scoop-private/we-meet` 升级因此失败；`HeaderProfile::Download` 现已**恒不发** Referer（`NO_REFERER_HOSTS` 随之下线），探针三 profile 保持原样。D4 的 UA 单入口 / PRIVATE_HOSTS / token 收敛结论仍然有效。

## 2026-09-13 `-d` 写盘护栏（会话 `dflag-write-guard`，已闭环，2 提交，分支 `fix/dflag-write-guard` → 已线性 ff 进 `refactor/edition-2024`，tip `be0092df`）
**规则（一条，统一适用）**：`-d/--dir` 缺省仍为 `.`；但**本次调用会写盘**时必须显式指定目标，否则 exit 1 —— **由写盘与否决定，不由命令身份决定**。
- `formatjson` 总是写 → 恒要求 `-d`；`checkhashes` 写盘 = `-u || -f`；`checkver` = `-u || -f`（官方 `if ($ForceUpdate) { $Update = $true }`，`checkhashes.ps1:52` / `checkver.ps1:404`）。
- `checkurls`/`missing-checkver` 从不写 → 保持默认 `.`（**有意放宽**：官方这几个也强制 `-Dir`）。
- `ci-auto-pr` 未改，**规则同样适用**（其 `-d` 现默认 `bucket`，`src/cmd/auto_pr.rs:21-23`）。
- `formatjson` 写盘时**逐文件**输出 `Formatting <path>...`（官方静默 ⇒ 有意偏离/hok 增值）。
**实现入口**：`src/cmd/shared_args.rs` 的 **`write_dir`/`scan_dir`**（新增写盘命令请复用，**勿再各写一份判据**）；`dir` 字段一律 `Option<PathBuf>`（写盘 `ok_or`、只读 `unwrap_or(".")`）。
**破坏性变更**：裸 `hok formatjson`、`hok checkhashes -u|-f`、`hok checkver -u|-f` 由"扫 cwd"变 exit 1。
**实测（护栏前的危险行为）**：隔离环境裸跑 `hok formatjson` 会重写**诱饵树里所有内容有变化的合法 JSON** —— `tsconfig.json`、`package-lock.json`、`target\debug\out.json`，连隔离出的 `~/.config/hok/config.json` 也被改写。根因：`manifest_walker.rs:44-45` 递归收全部 `*.json`（仅排除 `package.json`），`formatjson.rs:57-70` 走通用 `serde_json::Value`、**不判断"像不像 Scoop 清单"**。官方四脚本 `-Dir` 均 `Mandatory=$true`（非交互 exit 1），`checkver.ps1:85` throw。
**既有问题（未修，建议另开会话）**：`hok checkver` 在**没有任何清单声明 checkver（含空目录）**时**无限挂起** —— 已在 base `2a18d5b` 复现，与本护栏无关（D4 会话也独立发现同一现象）。

## 2026-09-13 S9 `checkver` 深度对齐（已闭环，4 提交，分支 `fix/s9-checkver` → 已线性 ff 进 `refactor/edition-2024`，tip `8ad7d080`）
**决策（用户拍板）**：按 **S9-2 → S9-3 → S9-1** 逐块做；与已落地的 D4 **去重**（请求头一律走共享入口）；两条有意偏离见下。
**落地**：
- `checkver.rs`：`Substitution`（**最长键优先** + `$match<Name>` 大小写不敏感）；`$url/$baseurl/$basename(=UrlDecode(url_remote_filename))/$urlNoExt/$basenameNoExt` 按官方**逐 URL 分层**应用到 hash 配置；属性替换**驱动自 raw JSON 的 autoupdate 键**（覆盖 changelog/extract_to 等）；hash 只在 manifest 已有 hash 时写入；hash 回退下载落 Scoop 缓存；删除恒等 stub（`checkver_url.rs` 整个移除）。
- `checkver_hash.rs`：默认兜底正则 + 文件名旁 + metalink、`$sha256/$basename/$base64` 模板、**`find` 是正则取第 1 组**（不是字面子串）、"找不到"**回落下载 + 本地 SHA-256** 而不是整条失败。
- S9-1：github/sourceforge 快捷方式**显式建模**（`Checkver` 增 4 个 `#[serde(skip)]` 标记）；API 改写**仅在配置了 token 时**；`checkver.script` 复用既有子进程入口 + preamble；`--throw`（带 `--throw` 坏 URL → exit 1；不带则打印 ERROR 且 exit 0）。
- **与 D4 去重（勿回退）**：`ScoopRequest`/`HeaderProfile` 是唯一请求头入口；`ResolvedSource` 无 `user_agent`/`token` 字段；`download_bytes` 走 `HeaderProfile::Autoupdate`；**别再加内联 `RequestOptions`**。token 只认 `network::github_token`（`SCOOP_GH_TOKEN` 优先于 `gh_token`，空值视为无）。
**实测量化（21797 个真实 manifest / 4 bucket）**：hash 变量从不替换 1160/3849 manifest → 全部可解；hash 链无解 1350/1387 条目（1298/3846 manifest）→ 全部可解；github+自定义 re 被喂 API JSON 122/620 → 恢复；与已安装 Scoop 0.5.3 请求 URL 逐字一致；约 999 个 `/releases` 双写清单恢复可用。
**两条有意偏离（登记）**
1. **`Bearer` vs 官方 `token`**（用户拍板）：GitHub 官方文档标准写法是 `Authorization: Bearer`，官方 `bin/checkver.ps1:223` 用 `token`，两者 GitHub 都接受，hok 取标准拼写。
2. **`checkver.github` 已含 `/releases`（或 `/releases/latest`）时不再追加**：官方无条件追加产出 `.../releases/releases/latest`（实测 404，约 999 个真实清单受影响），hok 做**超集修复**（官方在该形态本就不工作，故不破坏任何"官方能工作"的场景）。
**其他有意偏离/未做**：Digest base64 解码失败回落 `.meta4`（§4 已登记，未碰）；`$urlNoExt`/`$basenameNoExt` 键前缀碰撞（官方实测产出 `...zipNoExt/checksum`，hok 改最长键优先）；script 子进程 vs 官方进程内；脚本失败不中断整轮；非 UTF-8 lossy；**未做** xpath 命名空间、输出文案、`checkver.url` 变量替换。
**⚠️ 过程教训**：本沙箱里提交信息一律 `-F <文件>`（PowerShell 会展开反引号与 `$`，该会话曾损坏 3 条提交信息、靠 `git fsck --unreachable` 找回重建）；**有未提交改动时绝不 `reset --hard`**（该会话曾两度丢失未提交工作）。
**How to apply**：autoupdate 属性替换只走 `Substitution`；hash 提取只改 `find_hash_in_*` 且"找不到"必须回落下载；快捷方式判定只改 `resolve_checkver_source`；请求头一律 `ScoopRequest`；token 只认 `network::github_token`。

---

## 2026-09-13 update/upgrade 对齐（会话 `update-upgrade-align`，`ab7ea6e` → `77475e3`）

**做了什么（c1–c5 + rustfmt）**：①**删 15 分钟 bucket 冷却**（`Config::update_cooldown_remaining[_at]` + 单测 + i18n 键 `bucket_cooldown` + 两份 README 的宣称 + 失败汇总里的 `--force` 提示）——官方显式 `scoop update` **无条件**同步自身+buckets（`libexec/scoop-update.ps1:397-398`），3h `LAST_UPDATE` 只门控**隐式自更新**（`lib/core.ps1:1168-1178`）；pacman 亦无客户端时间窗（`-y` 只按"库是否最新"跳过、`-yy` 才强制，节流交给 systemd timer）。`last_update` **仍写入**（官方 config 面 + `hok config` 展示），其 reader `parse_last_update` 保留 + `#[allow(dead_code)]` + 理由（房中惯例见 `constant.rs:72-78`；`status --local` 新鲜度检查是待做项）。②`-f/--force` 收敛为官方「即使没有新版本也强制更新」。③`-I`→`-i`（install / update / `SyncArgs`，描述用官方 `Don't install dependencies automatically`）。④**hold 口径**：`--escape-hold` 四处 CLI + `SyncArgs` 字段 + `SyncOption::EscapeHold` + 4 个 i18n 键**全删**；uninstall 不再跳过 held；install 路径**保留门**但改为可见警告（新键 `held_install_skip`）；`resolve_cascade` 去掉 `escape_hold` 形参、held 依赖恒报 `PackageCascadeRemoveHold`。⑤`upgrade` 补 `-i`/`-k`，**不给** `-U`/`-R`（`-U` 与引擎强制的 `OnlyUpgrade` 相抵）。⑥升级引擎 `execute_upgrade` 从 `update.rs` 搬进 `upgrade.rs`（方向 update → upgrade；官方无 `upgrade` 命令，只是建议别名 `upgrade = scoop update *`）。

**勿踩坑**：
1. **`Cascade` 是死路**：`SyncOption::Cascade`/`NoDependentCheck` 全仓**无构造者**（唯一引用是 `sync_remove.rs` 的读取）⇒ 级联卸载生产不可达，`resolve_cascade` 只被 4 个单测调用。成因：`-c/--cascade` 在 `5f2532e`（eventloop 统一）被连带删除、commit message 未提。**R5 单开会话恢复**，别顺手删掉这条已测通的逻辑。
2. **官方 hold 只有一处闸门**（`scoop-update.ps1:432-438`）：install/uninstall 都不查 hold。要改被 hold 的版本，官方路径就是 卸载 → `hok install <app>@版本` → `hok hold`（卸载后 `is_held()` 必为 false，`package/mod.rs:361-375`）。
3. `held_skip`（升级路径的"保留原因"）与 `held_install_skip`（install 路径：你点名要装却被锁）**语义不同，别合并文案**。
4. **删配置项要顺着查读者**：删 `update_cooldown_remaining` 后 `internal/time.rs::parse_last_update` 变 dead code ⇒ 要么删要么按惯例 `#[allow(dead_code)]` + 写清理由（`-D warnings` 会抓）。
5. **删 CLI flag 要连"帮助键 + README 宣称 + 报错文案里的提示"一起清**：本轮差点漏掉 `bucket_update_failed_summary` 里的 "run 'hok update --force' to retry"。

**门禁**：`cargo test --workspace` 仅 2 条既有 ACL 环境性失败（clean HEAD 同样失败，本次已在**主树**复现确认）；`clippy -p libscoop -p hok --all-targets --no-deps -- -D warnings`〔pre-2026-09-19 写法：理由已失效，见顶部注记〕 干净（**必须 `--no-deps`**，否则内置 `innospect` 的既有 clippy error 会挡住）；改动文件级 `rustfmt --edition 2024` 干净；`--test cli_help_i18n|i18n_keys|exit_codes` 全绿；进程级冒烟：`hok install/update/upgrade -h` 的 flag 面 + `hok update -S` 用法错 exit 1 + `hok update -i <未装包>` 到引擎。

**How to apply**：`hok update`/`hok upgrade` 的 flag 集 = 官方 `scoop update`（`-f -g -i -k -s -a` + hok 的 `-o/-y/--ignore-failures`）；hold 门只对**版本提升**生效（install 与 update 两条路径都算）；要绕 hold 只有 `hok unhold` 与"卸载 → 指定版本安装 → 重新 hold"两条显式路径。

## 2026-09-13 运行中进程输出 + `ignore_running_processes` 语义回正（会话 `running-process-output`）

**背景**：用户实测 `hok update && hok upgrade` 拿到 `ERROR 'PowerShell' 仍在运行！请关闭该应用后重试；跳过更新。` 外加紧跟的四行 `pwsh (pid …)`。三点诉求：① 用 WARN 不用 ERROR；② 文案回到 `Running process detected, skip updating 'x'.`；③ 进程名+pid 只在 `-vv` 输出、样式改成缩进 dim 行。用户拍板：`ignore_running_processes=true` 时=**继续操作**（"组合 1"），该键文案用两行的 C3。

**改了什么**（`sync_install.rs` 为核心）：
- `RunningCheck` 恢复三态：`NotRunning` / `Running`（默认 skip）/ `Ignored`（按配置继续）；`Error::AppRunning` 不复活。这是对 `a4ea0b7` 的**语义回正**——官方 `test_running_process` 的 ignore 分支是 `return $false`（= 继续），不是"仍在运行"。
- 新增 `RunningAction`（`Update`/`Reinstall`/`Uninstall`/`Reset`）取代 `action: &str` 形参：英文动名词无法从裸动词推导，故每个动词一个 i18n 键（`cmd.running_process_{update,reinstall,uninstall,reset}`）；`action.*` 四个键随之成为死键并删除。
- 消息选择收进 `check_not_running`：默认 `Running process detected, skip updating '%{name}'.`（WARN）；开键 `cmd.running_process_ignored` 两行 WARN（第二行的 5 空格缩进**写在 locale 里**，对齐渲染器加的 `WARN ` 前缀）。
- 进程明细不再拼进消息：新增 `Output::Detail` + `OutputHandle::detail` + CLI `output::detail(msg)`（`  {dim}`，**stderr**，与所属 WARN 同流），由全局 `-vv`（`LevelFilter::INFO`）闸门渲染；`-v` 及以下不出现。库侧仍无条件发射，保持 verbosity-agnostic。
- 顺序必须**消息在前、明细在后**（首版写成明细在前，冒烟实测抓到后修正；单测改为有序事件流钉住顺序）。

**验证**（临时 `SCOOP` 根 + 假安装 + 真跑 `app.exe`，再跑 `hok uninstall app`）：
- 默认（无 `-v`／`-v`）：`WARN 检测到进程正在运行，跳过卸载 'app'。`（en：`WARN Running process detected, skip uninstalling 'app'.`），无明细；
- `-vv`：同一条 WARN + `  app (pid N)`；
- `ignore_running_processes=true`：两行 WARN（含"可能导致数据丢失或应用不稳定，建议先关闭再重试"）+ 明细，随后**真的继续**卸载，并因 exe 被占用而 `ERROR … it may be in use: 拒绝访问。(os error 5)` —— 正是该警告所述风险。

**门禁**：`cargo test -p libscoop -p hok` → hok 侧全绿，libscoop lib 742 passed / 2 failed（仅既有 `permission::tests::grant_users_write_inherit_*` 的 ACL `error 5` 环境性失败）；`cargo clippy --fix -p libscoop -p hok` 干净；改动文件级 `rustfmt --edition 2024` 干净。

**How to apply**：改运行中进程的文案/明细只动 `check_not_running`（`crates/libscoop/src/package/sync_install.rs`）+ 那 5 个 `cmd.running_process_*` 键；`-vv` 级明细统一走 `Output::Detail` → `output::detail`（stderr），别再把列表拼进消息；`ignore_running_processes=true` 是**真的继续**，不要再把它当"只改文案"的开关。附带注意：`-vv` 同时会打开 tracing 的 INFO 日志（既有行为），所以 `-vv` 下会先看到 `INFO libscoop::session: config loaded from …`。

## 2026-09-13 「Everything is ok!」页脚删除（同一会话 `running-process-output` 的第二件事）

**背景**：运行中进程被跳过时仍打印 `✓ 一切正常。`，用户指出该页脚本身多余。核实：官方**只有 `scoop status`** 打 `success 'Everything is ok!'`（`Scoop/libexec/scoop-status.ps1:80`），install/uninstall/update 都在 per-app 成功行结束；hok 这行是 2026-08-13 自加（CHANGELOG `6fe4b8e`），一直登记为"有意偏离，保留"。

**做了什么**：删除 `install.rs` / `uninstall.rs` / `upgrade.rs` / `reinstall.rs` 末尾的 `output::done(t!("output.ok_all"))`；`status.rs:161` **保留**（官方确有）。只为该页脚而存在的守卫一并清掉：`uninstall.rs` 的 `any_installed`（连同整块 `query_installed` 调用）；`upgrade.rs` 的 `missing_target` **必须保留**——它还守着 "All apps are up to date" 的早返回（否则未安装的具名目标会被误报成"已最新"），只删末尾的 `if missing_target { return }` + 页脚；`install` 的 `downloaded_only` 页脚保留（`--download-only` 专用）。

**为什么不走 `scoop_handler.rs` 屏蔽**：`scoop_handler.rs` 只渲染事件总线上的 `Event`（`PackageCommitDone → closure_done` 等），这行是各命令末尾**直接** `output::done(...)` 打的，根本不经过它；在那里屏蔽 = 借不相干接缝掩盖消息（[[no-patchwork-fixes]]），且 4 个发射点照旧"声称一切正常"。正确入口就是发射点本身。

**验证**：临时 SCOOP 根实测 `hok uninstall app` 成功路径收在 `✓ app`（官方 per-app 行）、无页脚；`hok status` 仍收 `✓ 一切正常。`。门禁：clippy 干净、`cargo test -p libscoop -p hok` 742 passed / 2 既有 ACL 失败。

## 2026-09-13 下载默认不发 Referer（回归修复，会话 `referer-download-default`，2 提交 `cf260d9`/`4748ae1` → 已线性 ff 进 `refactor/edition-2024`，tip `4748ae1`）

**触发**：用户 `hok upgrade` 报 `scoop-private/we-meet-3.46.1.409` 先打「无需下载，均已在缓存中」、紧接着 `ERROR HTTP status client error (403 Forbidden) …updatecdn.meeting.qq.com…`，而缓存实际为空。

**根因（回归，定位到 D4 的 `e326ecf`）**：那次把官方 `lib/download.ps1:94-96` 的 `Referer: strip_filename $url` 抄进了包下载路径；而该路径在 D4 之前**从不发 Referer**（提交信息自述 "sent a User-Agent but no Referer"），用户 2026-09-03 装 3.45.10.411 成功正是那个状态。实测（Node，直连与 7890 代理一致，HEAD+GET 同结论）：**无 Referer → 200/206**；`Referer: https://updatecdn.meeting.qq.com/cos/<md5>/` → **403**；`Referer: https://meeting.tencent.com/` → 200。官方在**同一个 URL 上同样 403**（URL 带 `#/dl.7z` 使 `fname=dl.7z`，官方 Referer 算成 `…officialwebsite.exe#/`，仍是 updatecdn 域）⇒ 属**上游同款缺陷**：Scoop #6009（"manifest 里没有地方能定义/取消 Referer"，仍 open 且 blocking 两个 issue）、#6212（阿里云盘 `cdn.aliyundrive.net` 同款 403，日志里那行 `referer=…exe#/`）、Extras #11587（geeks3d 需要的是**页面** Referer，Scoop 的目录 Referer 也喂不对）。

**决策（用户拍板 A + 只改 Download profile）**：下载路径**默认不发** Referer，需要 Referer 的站点由 `PRIVATE_HOSTS` 显式给——那是官方文档化的 per-host header 机制（`libexec/scoop-config.ps1:106-109`）。探针三 profile（checkurls/checkver/autoupdate，官方 `bin/checkurls.ps1:60` / `bin/checkver.ps1:246` / `lib/autoupdate.ps1:25,55,117,145,186`）**保持与官方一致**（它们不是下载）。这是**有意偏离上游**，依据写在 `HeaderProfile::referer` 的 doc comment 里。

**顺带修的真问题**：`src/eventloop.rs` 的计划块只判断 `total > 0`、忽略 `DownloadSize::estimated` ⇒ HEAD 探测失败（403/404/超时）时 `total == 0 && estimated`，把"探测失败"报成"全在缓存里"。改用 `sync_install.rs:373-377` 已有的正确谓词（`total == 0 && !estimated` 才算全命中），未知大小打印新键 `cmd.download_size_unknown`。**注意**与 2026-08-28 `67ab5b5`（`0 == 0` 命中误判，属**判定层**）不是同一处，本次是**展示层**。

**门禁更新（2026-09-19）**：vendored `crates/innospect` 已退役（改用 registry `innospect 0.1.4`，见 `.agents/docs/legacy/nsis-issue.md`）⇒ **本文档此前各节"`clippy` 必须 `--exclude innospect` / `--no-deps` 否则被 vendored 告警挡死"的限制一律失效**：它现在是普通依赖，不再被 clippy 检查，整包 `cargo clippy --workspace --all-targets -- -D warnings` 可直接跑。
**门禁**：`cargo test --workspace` 741 passed / 2 failed（仅既有 `permission::tests::grant_users_write_inherit_*` 的沙箱 ACL `error 5`，S5 节已记录同一现象）；`cargo clippy -p libscoop -p hok --all-targets --no-deps -- -D warnings`〔pre-2026-09-19 写法：理由已失效，见顶部注记〕 干净——**不加 `--no-deps` 会被 vendored `crates/innospect` 的 3 个既有 `chunks_exact` lint 挡死**（`records/language.rs:388`、`records/registry.rs:345`、`util/encoding.rs:42`，与本次无关）；`rustfmt --edition 2024 --config skip_children=true` 干净。

**How to apply**：下载请求头唯一入口仍是 `ScoopRequest`/`HeaderProfile`；**不要再给 `HeaderProfile::Download` 加自动 Referer**，要 Referer 走 `private_hosts`。hok 的 PRIVATE_HOSTS 注入目前是 append（官方是 replace）：在 Download 路径上因"默认不发"只剩一条 Referer（新测试 `download_file_takes_its_referer_from_private_hosts` 断言 `count == 1`），但探针 profile 上仍会重复——属**未修的既有偏离**。另：`hok checkurls scoop-private/we-meet` 仍会按官方口径报 403；`hok download` 需要先 `cargo build -p hok-shim`（`libscoop/build.rs` 要求 `target/{debug,release}/hok-shim.exe`）才能编译。

---

## 2026-09-13 R1 `hok checkver` 无限挂起（已闭环，5 提交 → 主线 `43795484`）

**根因（确定性结构死锁，不是"慢"）**：`crates/libscoop/src/package/checkver.rs:383` 的 `.buffer_unordered(pending.len())` —— `pending` 为空时容量为 **0**。vendored `futures-util-0.3.34/src/stream/stream/buffer_unordered.rs:62-86`：`while in_progress_queue.len() < *max` 在 `max=0` 时**一次都不进** ⇒ 源流永不被 poll ⇒ `is_done()==false` ⇒ 返回 `Pending` 且**不注册 waker** ⇒ `collect().await` 永久挂起。无输出、无 CPU 占用；`-t/--timeout` 只是单请求超时，到不了。

**触发面（比"零 checkver 清单"宽得多）**：①空目录/无 `.json` ②清单全部未声明 `checkver`（`:233-236` `continue`）③app glob 不匹配任何 stem ④**任何 `--version` 调用**（`:241-269` 整支在循环内 `continue`，`pending` 恒空 —— 实测先打印版本行再挂死，`--version --update` 还会在挂起前先把盘写掉）。

**修法**：`.buffer_unordered(pending.len().max(1))` + 注释；非空时 `max` 是恒等 ⇒ 并发度/输出/`--throw`/hash 链/请求头逐字节不变。

**同类站点（全仓 `buffer_unordered` 仅 4 处）**：`checkver.rs` 唯一可能为 0（已修）；`download.rs` 跨包 fan-out 上游已 `jobs.max(1)`、HEAD 探测已内联 `jobs.max(1)` ⇒ **不加恒等 `.max(1)`**，改在容量消费点加 **debug-only `debug_assert!(jobs >= 1, …)`**；`download_fragmented.rs` 入口加 `debug_assert!(chunk_count >= 1, …)`（兼覆盖 `remote_size / chunk_count` 除法）。提交标题用 `chore(download)` 而非 `fix(streams)`——**无 live 缺陷**，诚实标注。

**测试**：回归钉住空目录 / 无 checkver / glob 不匹配 / `--version`（含 `-s`、`-u`）四种终止；**负对照已做**（临时改回 `pending.len()` ⇒ 新测试在 10s 预算上 `Elapsed(())` 失败，而**不会挂住测试 runner**）。

**How to apply**：任何 `buffer_unordered` / `buffered` 的容量**必须 ≥1**；给流式并发设容量时，零容量是**静默死锁**（不是空跑）。查同类问题先看 vendors 的 `buffer_unordered.rs` poll 实现，别猜调度。

**顺带**：清掉了 `checkver.rs:17,47-60` 已陈旧的 `TODO(scoop-alignment)` 块（`--throw` 已在 S9 落地、请求头已在 D4 收敛）。

---

## 2026-09-13 R2 缺 `USERPROFILE`/`HOME` 时 config 写盘 panic（已闭环，3 提交 → 主线 `9b672207`）

**现象**：清空 `USERPROFILE` 与 `HOME` 后任何建 session 的命令都 `panic`（`internal/fs.rs:63:30`，`Option::unwrap()` on `None`，exit `-1073740791`），只有 `hok --version` 正常。

**根因（曾被我误判一次，以实测为准）**：写链上的空路径来自 **`hok_config_path()`**（`config.rs:889-891`，经 `Config::default` 的 `path:` 字段 `config.rs:758`），**不是** `scoop_config_path()`（后者只喂 `ConfigBuilder::scoop_path`，`config.rs:47-49` 注释 "Never written to"，是只读回落）。`hok_config_path` 走 `home_dir()`（仅 `USERPROFILE`/`HOME`）⇒ **与 `APPDATA` 无关**（实测：保留 `HOME`、去掉 `APPDATA` 不炸）。`Path::new("").parent()` = `None` ⇒ `fs.rs:63` 的 `unwrap()` 炸。第二个可达入口：`config.rs:110` 的迁移写（exe 旁有 portable `config.json` 且无 home）。

**修法（三件）**：① `fs.rs:63` 与同型 `fs.rs:115`（`symlink_dir`）的 `parent().unwrap()` → `Error::Io(InvalidInput, …)`（复用既有 `error.io`，不新增 i18n 键）；② `config::init_default` 对空 `config.path` **跳过写盘**（它本就是 `let _ = write(...)` best-effort）；③ **空串 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE` 与未设置同义**（`config::default::env_path(name) = var_os(name).filter(|v| !v.is_empty())`），对齐官方 `lib/core.ps1:1291-1301` 的 `| Where-Object { $_ }`。

**根路径语义（用户拍板"明确报错而非静默落 CWD"）**：`require_root_path()` + CLI 启动守卫把"真无 root"（`SCOOP` 未设且无 home）变成明确 `Error::Io(InvalidInput)`（英文文案）；因 ③ 落地，`require_global_path`/`require_effective_root_path`/`ensure_global` 守卫**变为不可达并被删除**（不留死代码）。

**How to apply**：`config.rs` 里 `hok_config_path`（写）与 `scoop_config_path`（只读）同源易混——判断"谁进了写链"必须看**写入目标实测**，不能看函数名。空 `PathBuf` 的连锁风险（`shortcut.rs:41`、`psmodule.rs:87` 的 CWD 相对路径）**未修**，属登记项。

**门禁口径**：沙箱为 `danger-full-access` 时那两条 `permission::grant_users_write_inherit_*` **会通过**（落地后新树 747 passed / 0 failed）；沙箱退回 `workspace-write` 时它们因 ACL `error 5` 失败。**别把后者当回归。**

**How to apply（流程侧）**：本仓新会话开工时，若需 gpg 签名提交而沙箱是 `workspace-write`，`git commit` 会失败（gpg 需写 workspace 之外的 `~/.gnupg`）——**不要**用 `--no-gpg-sign` 绕过；本会话已验证放宽到 `danger-full-access` 后 `sig=G` 正常。

---

## 2026-09-22 `cleanup -l/--list` + 删私有 `--verbose` + cache 帮助面翻译（2 提交 `cd67788`/`65b83ee` → 主线 `65b83ee4`，无 merge commit）

**范围（用户 2026-09-22 点名）**：① `cleanup` 删命令私有的 `-v/--verbose`——`-v` 本是全局 verbosity（其他命令都显示共享文案），只有 `cleanup`/`info` 自己声明同名参数把它盖住；连同它门控的「已清理干净」报告与 `cmd.cleanup_already_clean`/`cli_help.cleanup.args.verbose` 两个键**彻底删除**；② 新增 `-l/--list` 列出**现存**版本（用户明确：不是替代 `-v`）；③ `-a/--all` 描述补「（相当于 '*'）」（对齐 `upgrade`）、`[APP]...` 去掉「（留空表示全部）」（裸 `cleanup` 早已报 `<app> missing` exit 1）；④ `cache` 位置参数三处未翻译修掉。

**hok 独有扩展（登记）**：`cleanup -l/--list` 官方 Scoop 无对应选项（`scoop-cleanup.ps1` 的 getopt 只有 `agk`）。取值语义 = `list --versions`（复用同一 `package::list::list_installed_versions` 入口，杜绝第二套版本枚举），渲染是它减掉 源/更新时间 两列：当前版本为首行带 app 名的加粗行，其余为暗色续行 + `旧版本`，无版本目录的 app 打 `-` + `无版本目录`。**只读** ⇒ 跳过 `-g` 的管理员门；`-k` 与它 clap `conflicts_with` 拒绝组合；**裸 `-l` = 全部已装 app**（用户拍板），而破坏性路径继续拒绝裸 `cleanup`。**解析不出当前版本时（broken 安装：`current\manifest.json` 与 `install.json` 双缺）首行报「当前版本未知」（`cmd.cleanup_list_current_unknown`）而不是 `旧版本`**（`2eba651`）—— 只读清单不该把 junction 目标断言成"可清理的旧版本"；实机案例 potplayer（本机只有它缺这两份元数据，`hok list` 同时记为「安装失败」）。该判定**已上移到 `src/cmd/mod.rs` 的 `VersionNote`/`version_note`**（与 `status_info_word`/`InfoContext` 同处 = 共享显示词单入口），`list --versions` 同样走它（`daa1002`），且这类目录**不再计入** `list -V` 页脚「保留了 N 个旧版本」—— 否则新行说「当前版本未知」、下一行又说「保留了 1 个旧版本」，且 `cleanup` 本就跳过这类 app。**改这两处措辞只动 `version_note`**，别在两个表里各写一份判断。

**cache 那一侧的真相**：顶层 `[APP]...` 从来没进过 locale（缺 `cli_help.cache.args.app`）；两个子命令则是**字段改名后的过期键**（locale 里仍是 `list.query`/`remove.query`，字段早已叫 `app`）⇒ 位置参数一直渲染英文 doc comment。三处一并修，并删掉过期键。

**顺带加固**：`tests/cli_help_i18n.rs` 的位置参数检查从 `-`/`<` 扩到 `[ARG]...`——原先跳过方括号行，正是它让 cache 的英文位置参数在测试全绿下漂移。扩检查后全命令帮助面重扫仍绿（漏网的只有 cache 那三处）。

**How to apply**：① 只读子命令的「空选择 = 全部」必须与破坏性路径的「空选择 = 报错」**分开算**：首版沿用 `all = args.all || any('*')` ⇒ 裸 `cleanup -l` **静默无输出**；已抽 `lists_every_app()` 并用 `Args::try_parse_from` 单测钉死。② 往 `locales/*.yml` 加键时，值里含 `": "`（如 `(default: all)`）**必须加引号**，否则 `hok-i18n` 的 build.rs 直接 panic（编译都过不去）。③ 命令帮助面的位置参数翻译键是 `cli_help.<cmd>.args.<field>`，**字段改名不会自动跟着变**，改字段名要顺手改两份 locale。④ 门禁：hok 侧全绿（lib 49 / `cli_help_i18n` 3 / `exit_codes` 9 / `i18n_keys` 3），libscoop 760 passed / 2 failed（仍是沙箱 ACL 那两条），clippy + 改动文件 rustfmt 干净。

---

## 2026-09-25 下载超时/哈希详情/缓存命中提示三点修正 + 延迟校验竞态登记（3 提交 `4b2004b`/`d49d6dd`/`cdcfab1`，base `6620d69`）

**背景**：用户实测 `hok download` 在"服务器只接连接不回包 + 2s 超时"下报 `download failed: error sending request for url (...)`，**读不出超时**；另两项是中文环境哈希详情仍英文、缓存命中提示显示缓存文件名。三项各自先报告（含"不改"选项）后实施，一主题一提交。

**① 超时分类（`4b2004b`）**：reqwest 的 `Display` 只写 `error sending request for url (...)`（`reqwest-0.12.28/src/error.rs:231`），超时原因在 `source()`（`TimedOut` → `operation timed out`），hok 此前只取顶层 Display。改为 `Network::download_stream` 返回类型化 `DownloadError{Timeout{secs},Other}`，超时映射 `Error::DownloadTimedOut{url,timeout_secs}` + 新键 `error.download_timeout`。**单流不加重试**：官方 `Start-Download`/`Invoke-Download`（`lib/download.ps1:71-196`）单流不重试（重试只在 aria2 分支），hok 已有 `.download` + Range 续传。

**② 哈希详情本地化（`d49d6dd`）**：`HashMismatchContext` 的 `Display` 是写死的英文四行，`translate_error` 无分支 ⇒ 中文环境英文详情。**关键**：`cmd.failed_verify` 把 `error = e` 直接嵌进模板（内联 `download.rs:306`、尾部 `download_verify.rs:157`），只补 `translate_error` 覆盖不到；新增 `error.hash_mismatch_detail`（en/zh）+ `HashMismatchContext::localized()` + `verify_error_message()`，三处共用；`Display` 保持英文（库/日志面）。

**③ 缓存命中提示（`cdcfab1`）**：`PackageCacheHit(String)` 承的是缓存名 `app#1.0#<sha7>.ext` 且**每文件**发一条。改结构体载荷 `{ident,name,version}` + 每命中包一次，`loading_cache` 渲染 `name version`。（官方 `lib/download.ps1:60` 印的是 URL 远端文件名，同样不印缓存名，但 hok 这里取可读身份更适合前端。）

**④ 登记（未改码）**：`download_verify.rs:211-222` 的 `try_send(PackageDownloadDone)`（异步事件总线）与紧随的 `session.output().error(...)`（**同步直印**，`output.rs:16-18`）之间**无顺序保证**，延迟校验消息可能压在仍活跃的 bar 行上（bar 还有 100ms steady tick 重绘）。**判定：无确定性复现**（窗口是"入队→事件循环被调度"，无注入点，ConPTY 只能放大可见性），故按用户裁决只登记、不臆造修复。**同源更硬一洞**：`EVENT_BUS_CAPACITY = 20` + 控制事件一律 `try_send` ⇒ 队列满时 `PackageDownloadDone` 被静默丢弃，bar 永不 `finish()`。可选修法（将来）：延迟消息改走专属事件，顺序由 FIFO 单消费者确定性保证。**另登记**：hok 的 120s 是 reqwest **总 deadline**（含响应体），官方 .NET 是 `Timeout`=100s（响应头）+ `ReadWriteTimeout`=300s（每次读），慢链路大文件语义不同。

**门禁**：`cargo clippy --workspace --all-targets` 0 警告；`cargo test --workspace` exit 0（libscoop 783 绿，含 3 条新测试：静默服务器超时分类 / translate_error 两个分支 / 双文件包缓存命中仅一条事件）；改动文件 `rustfmt --edition 2024 --config skip_children=true` 干净（含 `mod` 声明的文件必须带该配置）。详细叙述见 `.agents/docs/devlog/2026-09-25-download-timeout-hash-cache.md` 同日条目。

**How to apply**：① 错误串"看不懂"先分清是谁丢的信息——reqwest 的 `Display` 天生不含 source 链，超时/连接/DNS 长得一样，要么分类成类型要么别指望文案诊断，**别在错误串上做字符串识别**。② 本地化要顺着"谁把错误嵌进模板"找全：`translate_error` 只覆盖顶层传播的错误，`hok_i18n::t!("...", error = e)` 这类嵌套嵌入是额外表面。③ 事件载荷是接口：让前端反解 `app#1.0#hash.ext` 等于把缓存命名公式变成公开契约。④ 并发顺序问题别用 flaky 测试"复现"——能确定性给出的只有通道/调用序的结构事实，该登记就登记。



## 2026-09-25 `cleanup -k` 缓存面三修（2 提交 `ff05b80`/`cb75ba4` → 主线 `cb75ba43`，无 merge commit；worktree `.worktrees/s-cleanup-cache`，分支 `fix/cleanup-cache-names`）

**起因**：用户实机反馈「`hok cleanup * -k` 貌似没同步删缓存」。**先证伪再修**：把真机 `$env:SCOOP` 的 74 个 app + 25 个缓存文件按结构（目录 + junction + `install.json`/`manifest.json` + 缓存文件名）镜像到临时根，用真机同款二进制跑 `cleanup * -k` —— 旧版本目录与过期缓存都被正确删除；`-k *` / `* -k` 两种顺序、经 `shims\hok.exe` 调用、persist junction 三类用例全过。主路径无 bug，但三个真缺口成立（用户拍板全修）。

**已修**：
- `REGEX_CACHE_FILE`（`crates/libscoop/src/constant.rs:80-95`）name/version 字符类 `[0-9a-zA-Z._-]+` → `^[^#]+#[^#]+#`。**过窄的字符类 = 文件在 hok 眼里不存在**：含 `+`/`_` 的版本号（`1.9.11_2007`、`1.4.3-canary.1+29d9638da`；官方 buckets 250+ 个 manifest）与 CJK 包名的缓存文件被 `cache`（列表 + 总账）、`cleanup -k`、`cache rm <app>` 三处跳过，只有 `cache rm *` 能删。官方用 glob（`scoop-cache.ps1:28,45`、`scoop-cleanup.ps1:33`），从不限字符。这条把 `pending-tasks` 里挂账的「CJK 包名缓存文件被拒」一并清掉。
- 坏安装（current 解析不出）的缓存也删（`cache::remove_all_for_app`）：官方 `-Exclude "$app#$null#*"` 匹配不到文件 ⇒ 该 app 缓存全清；**版本目录仍不动**（官方 `$current_version -ne $_` 在 `$null` 下恒真会连目录一起删，hok 保留安全侧行为 = 有意偏离）。
- `cleanup -k` 补一行汇总 `cmd.cleanup_cache_removed`（**hok 扩展**，官方静默；零删除也打 ⇒ 静默只可能意味着没加 `-k`）。`cleanup()` 改返回 `CleanupOutcome { apps, cache_removed }`。

**验证**：worktree `cargo test -p libscoop` 786 passed / 0 failed、`cargo test -p hok` 全绿、`cargo clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings`〔pre-2026-09-19 写法：理由已失效，见顶部注记〕 干净、改动文件 `rustfmt --edition 2024 --check` 干净；隔离 `SCOOP` 根端到端 5 条（列表由「共 2 个文件」变 6 个、`cleanup * -k` 删 3 个并打汇总、broken app 缓存被清、当前版本缓存 + aria2 sidecar 保留、二次运行打 0 行）。真机 `$env:SCOOP` 实跑一次：16 个旧版本目录 + 6 个过期缓存（≈156 MiB）被删，当前版本缓存 19 个文件 / 2 GB 按设计保留。

**How to apply**：① 「`-k` 没删缓存」这类反馈先对照实验再改码——本次主路径正确、观感来自**零输出** + 留下的都是当前版本缓存（设计保留），修法是给可见性而不是加开关。② 共用正则既是"这是不是缓存文件"的判据又被三处复用 ⇒ 放宽容忍度时记得在另一头收紧（本次补 `^…$` 整名锚定，替掉原先"子串也算命中"）。③ broken app 的语义要分层看：删缓存可逆（重下）、删版本目录不可逆，官方两层都删、hok 只对齐前者。④ 沙箱里 `cargo test -p libscoop <filter>` 曾会踩 flaky 用例 `manifest_cache::tests::serving_a_stale_cache_warns_once`（进程级 warn-once + 并行运行；**2026-09-26 `9af599a` 已修**）——遇到别的 flaky 仍照此办：**必须在 clean HEAD 上用同一 filter 复现同样失败**才判定无关，验收以全量 `cargo test -p libscoop` 为准。

**追加（2026-09-25，`7b92635`）**：`.parts` 残留分片清理 —— `remove_incomplete` 补扫 `<缓存名>.parts` **目录**（分片下载被中断后只剩分块、连缓存条目都还没生成，`list`/`remove_outdated`/官方非递归 `Remove-Item` 三条路全都够不着），返回 `IncompleteSweep { entries, bytes }`（字节**递归求和**：目录自身 `metadata().len()` 是 0），CLI 另起一行 `cmd.cleanup_incomplete_removed`；并发下正在写的分片由写入方文件锁保护（删除失败走既有 `cmd.failed_remove_cache` 通道）。**有意扩展（登记）**：官方 `Remove-Item "$cachedir\$app#*"` 不带 `-Recurse`，非空 `.parts` 目录上游同样清不掉。

# Scoop 对标差距 —— 现状与修复进度

> **这是什么**：与官方 Scoop 对标的**逐项现状**（P0/P1/P2/P3 的 ✅/⏸ 与依据）。
> **方法论与入口**见技能 [[scoop-gap-audit]]；**逐次落地记录**见 [`devlog/`](../../devlog/)（一条一文件，按日期）。
> **维护**：每修完一项，改本表那一行 + 写一条 devlog（**不要**把落地记录堆回技能）。

方案 C 全面对标 Scoop 的差距审计（2026-08-10，4 个 explore 子代理，file:line 详见阶段报告）+ 修复进度（2026-08 多轮，已提交）。方法与 manifest 解析单入口专题见技能 [[scoop-gap-audit]]。编号 P0/P1/P2/P3，✅=已修 / ⏸=暂缓或有意不做。

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
- ✅ #9 退出码机制（2026-09-13 S1 闭环）：clap 用法错/缺参 2→**1**、裸命令/帮助 0，并逐命令补官方专用码（`which` 2、`bucket list` 空 2、`shim` 2/3、`virustotal` 位掩码、`search` 无匹配/`list` 空装 1）；download 失败按官方改为 0（汇总行保留）。见 devlog [`2026-09-13-s1-s13-exit-code-batch-policy.md`](../../devlog/2026-09-13-s1-s13-exit-code-batch-policy.md)
- ⚠️ #21 架构 truthy 降级：ARM64 回退已实现（manifest.rs:597-632，Win11→64bit/Win10→32bit），其余未核对
- ⏸ #4 重复安装文案（install.rs:108-110 已有 warn「is already installed. Skipping.」，对齐度未核对）
- ⏸ #13 非 git main 转换、#15 -q/--quiet（全局有命令级无，install/update.rs 均无）、#16 输出文案、~~#17 cooldown（config.rs:465 仍 900s）~~ **✅ 2026-09-13（update-align `c2581d4`）冷却机制整体删除**（见 devlog [`2026-09-13-update-upgrade-alignment.md`](../../devlog/2026-09-13-update-upgrade-alignment.md)）、#18 hok 增强

## 全量差距分析（现状，2026-08-15；详细报告 [`scoop-gap-analysis-2026-08.md`](scoop-gap-analysis-2026-08.md)）
**命令覆盖**：hok 36 ⊇ Scoop 28 全 ✓；hok 独有 9（checkhashes/checkurls/checkver/formatjson/missing_checkver/completions/reinstall/upgrade/ci-auto-pr）。
**参数缺口（Scoop 有 hok 缺）**：~~shim（-g/alter/add/rm/`--`）~~ **✅ 2026-08-15 已补 add/rm/list(正则)/info + -g + `--` 终止符**（alter 用户决策不做，2026-08-16 移除）、~~depends -a~~ ✅ 2026-08-16（depends.rs，走 session.set_default_architecture）、~~reset -a/--all~~ ✅ 2026-08-16（reset.rs 遍历 local+global、跳过 scoop、global 无 admin warn）、virustotal（-a/-s/-n/-u/-p + 组合退出码 0/1/2/4/8/16）、~~export -c/--config~~ **✅ 2026-09-13（S4）**（export 本体 2026-09-13 已对标官方 schema，官方键名 `-c` 段随 S4 落地，见 devlog [`2026-09-13-s4-config-keys.md`](../../devlog/2026-09-13-s4-config-keys.md)）、status -l、~~info -v~~ **✅ 2026-09-12（info 全面对标，见 devlog [`2026-09-12-info-official-alignment.md`](../../devlog/2026-09-12-info-official-alignment.md)）**、hold scoop 特例（hold_update_until）。
**config 键缺口**：use_lessmsi、use_external_7zip、show_manifest、show_update_log、update_nightly、force_update、hold_update_until、debug、aria2-warning-enabled/retry-wait/options（多为边缘）。**2026-09-13（S4）处置**：这些键**保持不支持并登记有意偏离**，但 `config` 对它们报 `Error::ConfigKeyUnsupported`（"官方设置项、hok 无对应实现"），与完全未知键的 `ConfigKeyInvalid` 分流，不再被当成拼写错误。**2026-10-09**：`autostash_on_conflict`（默认 `false`）已实现。**与上游的差异在作用范围**：上游只在 `Sync-Scoop` 里认这个键（`libexec/scoop-update.ps1:113-121`，即 Scoop 自身仓库；`master` 与 v0.6.0 同），`Sync-Bucket` 是裸 `git pull -q`、不读它；hok 则把该键接到**每一次仓库 pull**（`bucket::update` 逐桶透传，`bucket.rs` 是唯一生产调用点）——理由是用户的 bucket 工作树里可能有**自己改过、尚未 push** 的清单，此时中止整个 update 对用户没有帮助。行为：命中 `BucketUpdateLocalChanges` 时以 `-u` stash（`WIP at <round-trip>`）后继续，stash **不 pop**（同官方）并 warn 提示 `git stash pop` 恢复；stash 自身失败即报错、不继续。键名与官方同名、同在 `SHARED_KEYS`，故 Scoop 配置可 import/export 往返。**2026-10-11**：`use_external_7zip` 判为**有意偏离**（键的上游原意是"不装 Scoop 的 `7zip` helper、改用 PATH 上的 `7z.exe`"，而 hok 主力解压链不走 `7z.exe`，落点比上游窄且与 X2/X4 同面）⇒ 真缺口只剩 `show_manifest`；同轮另登记两条大小写偏离（清单 hash 算法前缀、nightly 判定），三条见 [`command-deviation-audit-2026-09.md`](command-deviation-audit-2026-09.md) 的本轮处置与 [`pending-tasks.md`](../plan/pending-tasks.md) §5。
**流程**：ensure_install_dir_not_in_path ✅、~~persist_permission~~ ✅ 2026-08-16（**双层拆分**：permission.rs = 纯 ACL 原语 `grant_users_write_inherit(root: &Path)`，**windows-sys 0.52 原生实现**（GetNamedSecurityInfoW+SetEntriesInAclW+SetNamedSecurityInfoW+CreateWellKnownSid；.NET Write=FILE_ADD_FILE|ADD_SUBDIRECTORY|WRITE_EA|WRITE_ATTRIBUTES，ObjectInherit=OBJECT_INHERIT_ACE；无 Session/persist 依赖，未来进程/卸载等任意目录可复用；有单元测试读回 DACL 验证）+ persist.rs = 业务守卫 **`persist_permission`**（对齐官方命名；global+persist+admin 判定 + persist 根解析 → 调 permission 原语）；挂 install(sync_install.rs:548)+reset(sync_remove.rs:353)；session.persist_root 新增）、~~checkup 语义与 Scoop 完全不同（hok=已装包健康，Scoop=环境检查）~~ **✅ 2026-09-13 已修（S5，`5ae994d`）**：`checkup` 对齐官方环境检查，原健康检查改名 `health`（见 devlog [`2026-09-13-s5-checkup-health.md`](../../devlog/2026-09-13-s5-checkup-health.md)）、~~cooldown 15min vs 3h~~ **✅ 2026-09-13（update-align）冷却删除**。
**建议优先级**：virustotal 参数+退出码、export -c、shim 子命令、P1-9。

## 细节核实：config 边缘键 / persist_permission / 小参数（现状，2026-08-15；详表见 `scoop-gap-analysis-2026-08.md` §7）
- **config 边缘键**：缺失集合 14 个（测试锁定 12 个被拒键 + update_nightly/hold_update_until 2 个未建模；`set()` 未匹配键→ConfigKeyInvalid config.rs:693；迁移时静默丢弃 config.rs:106-115）。**真缺口仅 1 个**：show_manifest（无安装前交互）；use_external_7zip 已于 2026-10-11 判为有意偏离（原生解压无外部 7z 是 hok 的结构性差异，非缺口），见上条与 [`pending-tasks.md`](../plan/pending-tasks.md) §5；aria2-options/use_lessmsi/show_update_log/debug 等为架构差异或低价值；scoop_repo/scoop_branch/shim/hold_update_until 设计 N/A（单二进制）。修入口 = ConfigInner 加字段 + set() match + list_all，单文件闭环。**2026-09-13（S4）**：键名拼写严格化（只认官方规范名——下划线，`aria2-*` 连字符；砍 `aria2_split`/`no_junctions`/`script_engine`/`ignore_failure` 别名），`canonical_key` 收敛为唯一 `SETTINGS` 清单；`ConfigInner` 的 serde alias 保留，旧配置文件仍可加载。`aria2-min-split-size` 20M→5M 与 `proxy` 三态仍**暂缓**。
- **persist_permission**：官方 install.ps1:522-531（global+persist+admin → Users SID S-1-5-32-545 Write+ObjectInherit 到全局 persist 根；调用点 install.ps1:67/reset.ps1:91，幂等）。hok persist::link（persist.rs:31-92）无 ACL；**is_admin 已就位**（session.rs:197-202→internal/os.rs:55-71，IsUserAnAdmin FFI）。补齐 = 设 DACL（SetNamedSecurityInfoW 或 icacls）挂 install persist 步 + reset。
- **persist 链接三态（2026-09-20 已修，`8aa3dc5` `16a91cd`）**：`persist::link` 原先只抄了上游 `persist_data` 三态里的两态，缺「persist 目标已存在 **且** archive 自带同名数据」分支（上游 `Move-Item -Force $source "$source.original"`），遇到 sublime-text 那种 zip 自带 `Data/KEEPME` 的包会以 `os error 183` 中断安装；`persist.rs` 现已补该分支 + 5 条单测，`internal::fs::symlink_dir` 的清理错误不再被 `let _ =` 吞掉。**未覆盖**：persist 的 `-g` 全局 ACL 之外的分支（如 `$persist_dir\$source` 双元素形式）仍按原实现。
- **小参数**：depends -a（depends.rs:12-15，小）、status -l（status.rs:9-14，小）、~~info -v~~ ✅ 2026-09-12（info 全面对标）、~~reset 位置参数~~ **✅ 2026-10-09（对齐官方：位置参数全为 app + 全局自动识别）**、hold scoop 特例（hold.rs，中，需 hold_update_until 键联动 core.ps1:1181-1197）。
- **归档**：已完成报告已移入 `.agents/docs/reports/`（README 索引），活文档 `.agents/docs/active/audit/scoop-gap-analysis-2026-08.md`（原仓库根，随 `docs/` 迁入）；失效报告自动归档规范见 [[hok-report-archive]]。

## 全命令偏离审计（现状，2026-09-12；报告 [`command-deviation-audit-2026-09.md`](command-deviation-audit-2026-09.md)）
**口径**：4 组并行子代理**静态读码**逐命令对照本地官方 0.5.3，共约 280 条；分类 = 语义不同 / 行为缺失 / 默认值不同 / 输出或文案不同 / 退出码不同 / 边界处理不同 / hok 独有命令 / 有意偏离。**除 alias/export/aria2/config 键白名单 4 条已人工复核外，其余未实测**——动手前先按 [[investigate-first]] 复现。
**"下一个 checkup"级（语义不同，待用户决策）**：① `alias` 只写 config、**不生成 `shims\scoop-<name>.ps1`，别名不可调用** —— **2026-09-13 已修（`3c90095`）**，见 devlog [`2026-09-13-s2-alias-availability.md`](../../devlog/2026-09-13-s2-alias-availability.md)；② `export`/`import` 与官方 Scoopfile **schema 互不兼容**（hok `{buckets:映射, apps:按bucket分组}` vs 官方数组 + Info 字段；hok `import` 期望对象、官方是对象数组 ⇒ 官方文件导不进来）—— **2026-09-13 已修**（单一官方 schema + `--hok` 扩展；见 devlog [`2026-09-13-s3-export-import-interop.md`](../../devlog/2026-09-13-s3-export-import-interop.md)）；③ `config` 键白名单化 + `unset` vs `rm` + 无单键读取 + `proxy none` 语义相反；④ `which` 打印 shim 自身而非真实目标、不查 PATH/全局；⑤ `cleanup` 无参=清全部、`-k` 清空整缓存 —— **2026-09-13 已修（S6）**：裸 `cleanup` 报 `<app> missing` exit 1；`-k` 逐 app 删 `app#*` 保留当前版本 + 收尾清 `*.download`；`-g` 补管理员门、`-a -g` 清本地+全局；⑥ `cache` 必须带子命令 + 子串匹配（`rm zip` 命中 7zip）—— **2026-09-13 已修（S6）**：裸 `cache`/`cache <app>` 默认列缓存，匹配改按 app 名锚定并支持多 app，按 app 删除顺带清 aria2 `<app>.txt`；见 devlog [`2026-09-13-s6-cleanup-cache-safety.md`](../../devlog/2026-09-13-s6-cleanup-cache-safety.md)；⑦ `download` 的 `aria2-enabled` 在 hok 是内置分片开关（**从不调用 aria2**）；⑧ `virustotal` 默认行为相反（官方查 `/files/{hash}`，hok 总提交扫描）；⑨ `missing-checkver -s` 与官方 `-SkipSupported` 语义相反；⑩ ~~`reset` 不识别全局安装 / 不接受 `app@version`~~ **✅ 2026-10-09（`eddc682`）**：位置参数全为 app（`app` / `bucket/app` / `app@version`，经 `parse_app`），全局自动识别 = 官方 `installed $app $true`，单应用补 admin 门；`[version]` 位置参数删除（破坏性改动，版本切换改 `app@version`）；⑪ `uninstall`/`update` 遇运行中进程 **中止整批 exit 1** —— **2026-09-13 已修**（改为跳过继续；`ignore_running_processes=true` 时**警告后继续** = 官方 `test_running_process` 的 ignore 分支 `return $false`；held 跳过/被依赖拒绝保留为有意偏离。**同日回正**，见 devlog [`2026-09-13-running-process-output.md`](../../devlog/2026-09-13-running-process-output.md)）；⑫ `install`/`update` 的 `--independent` 是 `-I`（官方 `-i`）。
**退出码口径（2026-09-13 已更新）**：官方源码里 libexec 的 `exit N` 只有经 **`.ps1` shim 的 `exit $LASTEXITCODE`** 才成为进程码（cmd 路径丢码），`abort` 默认 1；hok 现按官方口径逐命令对齐，详见 devlog [`2026-09-13-s1-s13-exit-code-batch-policy.md`](../../devlog/2026-09-13-s1-s13-exit-code-batch-policy.md)。
**已知真 bug 候选** → **2026-09-13 已核实并修复（S4，`38ecfa9`）**：`53e35f3` 只把 getter 默认改成 `false`，默认列/locale 长帮助/两份 README 漏改；现全部为 `false`，并新增守卫测试（新配置下每个 bool 行的默认列必须等于 getter）。
**修复入口**：`.agents/docs/reports/command-deviation-fix-plan-2026-09.md`（已归档）= 新会话拆分计划 S1–S14（每条含范围/决策门/影响面/提交拆分/验收）；一条会话只做一个主题，开工引用「按该计划 S<n> 执行」。
## manifest 解析专题（现状）

方法与单入口架构见技能 [[scoop-gap-audit]]，逐次落地见 devlog。

- ✅ **P0-1 hash 前缀 × 长度强绑定**：非标准长度不再拒收（宽容）。
- ✅ **P0-2 `checkver.sourceforge` 对象强制 `path` 必填**：`path` 改为可选。
- ✅ **P0-4 `autoupdate.hash.mode: github`**：官方 `schema.json:51-62` 只列 8 个枚举值，但官方运行时不校验 schema，
  `lib/autoupdate.ps1:271-302` 的 `switch` 还接受 `github`（`:298-301` 取 Releases API asset `digest`）。
  hok 用严格 serde 枚举（`manifest.rs:586`）⇒ `unknown variant` ⇒ `parse_file` 整条 Err；
  又因 `package/query.rs:270,275` 的 `filter_map` 静默丢弃，表现为「找不到软件包」。
  修法 = 加 `#[serde(rename = "github")] Github` + 离线夹具（`crates/libscoop/tests/fixtures/0x7eTeamTools.json`）。
  验收：命中文件全部可解析、`search` 无查询包数恰好 +220、其余 bucket 零变化。
- ⏸ **清单解析失败不得在查询/安装路径静默消失**（`package/query.rs:270,275`；`:285-313` 解析失败落 `None`）：
  P0 级可见性；改它等于改单入口的错误传播策略，影响全部清单扫描语义，需单独立项。
- ✅ **`mode: github` 取 digest 失败/为 null 的回落**：`checkver_hash.rs:172-187` 返回 `None` 后走
  `download_and_compute_sha256`（下载落 Scoop 缓存 + 本地 SHA-256），不再 Err 中断该清单。
- ✅ **`serving_a_stale_cache_warns_once` 假 flaky**：根因是闩锁放错层——`STALE_WARNED` / `MISMATCH_WARNED`
  曾是进程级 static，而告警发往**每会话**的 `session.output()`；两处已搬进 `Session`
  （`mark_stale_warned` / `mark_mismatch_warned`，key 为 `bucket/name`）。
- ✅ **P1 autoupdate 缺字段**：官方 13 字段 vs hok 5 → 已补 architecture/extract_dir/hash/notes/url + autoupdateArch。
- ⏸ **P2 无影响不动**：`hashExtraction.type`（deprecated）、`installer._comment`、shortcuts 长度、version pattern。
- ✅ **已验证兼容**：license / psmodule / checkver 全键 / mode 8 枚举 / installer / uninstaller / architecture /
  autoupdate.hash 三形式 / bin 对象 / hook script。


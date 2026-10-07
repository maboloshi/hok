# 上游 Scoop 0.5.3 → 0.6.0 影响评估与本仓处置

Status: active（P0/P1/P3 已落地；P2 中 `63cb8f84` 已于 2026-10-07 落地，余两条只留差距记录）
Updated: 2026-10-07

> **是什么**：本地官方源码 `../Scoop` 于 2026-10-02 从 v0.5.3（`b588a06e`，2025-08-12）快进到 v0.6.0（`e6aa3b36`，2026-09-30），共 **76 个提交**。本文是这些改动对 hok 的影响逐项评估与本会话的处置记录。
> **口径**：官方侧一律引用 `../Scoop` 的 file:line（v0.6.0）；hok 侧引用本仓 file:line。**本节为静态读码 + 少量实测**，未做端到端复现的条目标注为「未实测」。
> **与旧审计的关系**：`command-deviation-audit-2026-09.md`、`scoop-gap-analysis-2026-08.md`、`scoop-gap-status.md` 的对标口径是 **0.5.3**，其结论在其口径内仍有效，但**不再是当前上游**；本文覆盖 0.5.3 → 0.6.0 的增量。

## 0. 一句话结论

P0 一项（元数据文件名）是**单向互操作破坏**，会随 0.6.0 普及自然爆掉；P1（checkver GitHub 模式）是**一个整体的行为移植**，hok 现有测试反而锁定着上游已删除的旧行为；其余为新增键/新增能力缺失与文案级差异。

## 1. P0 — 元数据文件名（必改）

`145381fc` 把落盘元数据改名，并加了「新名优先、旧名回落」的读链：

| 项 | 官方（v0.6.0） | hok（改前） |
|---|---|---|
| 清单写到 | `scoop-manifest.json`（`lib/manifest.ps1:128-131`） | `manifest.json`（`crates/libscoop/src/package/sync_install.rs:898`） |
| 安装信息写到 | `scoop-install.json`（`lib/manifest.ps1:146`） | `install.json`（`sync_install.rs:970`） |
| 读清单 | 新名优先、旧名回落（`lib/manifest.ps1:134-139`） | 只读旧名（`install_state.rs:190`、`query.rs:118`、`manifest_source.rs:256`、`sync_remove.rs:696`） |
| 读安装信息 | 新名优先、旧名回落（`lib/manifest.ps1:149-154`） | 只读旧名（`install_state.rs:189`、`query.rs:119,217`、`manifest_source.rs:209`、`hold.rs:49`、`list.rs:82`、`export.rs:251`、`info.rs:605`、`health.rs:102`） |
| 版本解析 | `current/scoop-manifest.json` 回落 `current/manifest.json`（`lib/versions.ps1:54-56`） | 只读 `current/manifest.json`（`install_state.rs:126`） |
| 版本枚举 | 两名皆 glob（`lib/versions.ps1:99-100`） | 只按 `install.json` 的 mtime（`install_state.rs:153`） |

**方向性**：官方能读 hok 写的旧名（兼容），**hok 读不到官方 0.6.0 写的新名** ⇒ `list` / `status` / `update` / `hold` / `uninstall` 会把该应用当作未安装或损坏。

**更正（2026-10-07，实测）**：上表「版本枚举」行原记的「P0 落地后仍未动」**不成立**——`newest_version_dir`（`install_state.rs:210-225`）的排序键是 `install_info_mtime`（`:93`），它经 `install_info_path`（`:73`）→ `metadata_path`（`:61`）**新名优先、旧名回落**，所以只带 `scoop-install.json` 的版本目录一直是候选；`resolve_current_version_reads_scoop_names_only`（`:597`）钉着这条，`cargo test -p libscoop --lib install_state::` 14 条实测全绿。当时的误判源自把表内「hok 改前」列当成了 P0 之后的状态。

**唯一残留（有意偏离，不照抄）**：**同一版本目录两名并存**时取哪条记录的 mtime。官方是双 glob 后 `Sort-Object | Select-Object -Unique`（`lib/versions.ps1:99-100`），该目录按**较早**的那条记录排位；hok 按解析出的（新名优先）文件排位。前者是上游管道顺序的副产品而非设计语义——它会把刚写过新记录、历史上有旧记录的目录往旧位置排。`install_info_mtime` 的文档注释已登记该偏离。并存形态只可能由「0.6.0 前后各写一次同一版本目录」产生。

**实测（2026-10-02，本机 `$SCOOP` 已升级到 v0.6.0）**：74 个已装应用的 `current\` **仍全是 `install.json,manifest.json`** ⇒ 官方**不做存量迁移**，只在新增/升级/重装时写新名。所以现在还没爆，但**任何应用被 0.6.0 重装/升级后**即触发。

**处置（用户 2026-10-02 拍板）**：hok **写新名 + 读新名优先回落旧名**，与官方落盘一致。代价：旧版 Scoop（< 0.6.0）会读不到 hok 新装的包——上游自己也接受这个取舍。

## 2. P1 — checkver GitHub 模式（一个整体移植）

上游四个提交是**一次行为变更**，必须一起移植；hok 现有测试恰好锁定被删掉的旧行为。**已落地**（2026-10-02 随 `baf4c53` `507aab2`，2026-10-07 逐条复核）。下表「hok 改前」列是 2026-10-03 的快照，「判定」列已按落地结果更新。

| 上游 | 官方行为 | hok 改前 | 判定 |
|---|---|---|---|
| `cbd7c64b!` | API 模式默认 `$jsonpath='$.tag_name'` + 正则 `(?:v\|V)?([\d.-]+)`；HTML 模式默认正则 `/releases/tag/(?:v\|V)?([\d.-]+)`（`bin/checkver.ps1:180-191`） | `checkver.rs:139` 默认正则为 `([\d.]+)`（无 `-`）；`resolve_checkver_source` 从不注入默认 jsonpath、无 API/HTML 分流（`:530-600`） | **已对齐**：API 模式默认 jsonpath + API 正则、HTML 模式默认正则（`checkver.rs:804-817`，常量 `:228-232`；测试 `github_api_mode_defaults_to_tag_name_and_the_api_regex`） |
| `35334422` | 校验 `$inputGithubUrl`（homepage 或 `checkver.github`）对 `^https://((www\.)?github\.com/…\|api\.github\.com/repos/…)$`，不匹配则报错并**跳过该 manifest**；非 api 才追加 `/releases/latest`；token 重写不再受 `$useGithubAPI` 门控（`bin/checkver.ps1:155-191`） | `checkver.rs:549-556` 只查 `starts_with("https://github.com/")` 且只出非致命 `pre_message`；map 形式完全不校验；`:589-593` 重写仍被 `use_github_api` 门控 | **已对齐**：`GITHUB_URL_PATTERN` 校验并给出 `skip_reason`（`checkver.rs:778-787`）；非 api 才追加 `/releases/latest`（`:789-794`）；token 重写不再受 `use_github_api` 门控（`:795-801`） |
| `bdf63444` | `checkver.github` 的 `$useGithubAPI` 改为无条件（删掉 `PSObject.Properties.Count -eq 1` 门） | `checkver.rs:560-563` 仍有 `if cv.github_solo` 门；测试 `:1899-1918`、`:3090-3118` 断言这个已被删除的行为 | **已对齐**：`github_solo` 门已删（2026-10-07 全仓无该符号），map 形式无条件走 GitHub 分流 |
| `d452e8f6` | `$.tag_name` 默认仅在**没有** `checkver.script` 时注入（`bin/checkver.ps1:182-184`） | `checkver.rs:278` `jsonpath = cv.jsonpath.clone()`，本就无默认 | **已对齐**：默认 jsonpath 注入以 `cv.script.is_none()` 为前提（`checkver.rs:808`） |
| `20a178b1` | fetch 失败但存在 `checkver.script` 时，警告后**仍执行脚本**（`bin/checkver.ps1:307-321`） | `checkver.rs:336-339` 直接返回 `CheckverFetch::DownloadError`，脚本分支（`:340-364`）永远到不了 | **已对齐**：下载失败且有脚本时返回 `CheckverFetch::ScriptFallback`（`checkver.rs:494-519`），上层先出警告再跑脚本（`:589-622`） |
| `c9f0728f` | `$originurl = strip_fragment $url` 同时喂 5 个版本变量与 github 模式的 jsonpath 字面量（`lib/autoupdate.ps1:214-218,303`） | `Substitution::with_url`（`checkver.rs:1165-1181`）已去 fragment；但 `checkver_hash.rs` 的 github jsonpath 用**原始 url** | **已对齐**：github 模式的 jsonpath 走 fragment 剥离后的 url（`checkver_hash.rs:160,174-176`，测试 `github_digest_jsonpath_uses_the_fragment_stripped_url`、`github_hash_lookup_strips_the_url_fragment`） |
| `bd2e93df` | `find_hash_in_json` 仅在 URL 匹配 `^https?://api\.github\.com/.*` 且有 token 时加 `Authorization: Bearer` + `X-GitHub-Api-Version: 2022-11-28`（`lib/autoupdate.ps1:121-124`；实际条件以源码为准，CHANGELOG 还提到 HTTPS 要求） | 对**任意** json hash URL 都传 token（`internal/network.rs` 的 opts 覆盖）；从不发 `X-GitHub-Api-Version` | **已对齐**：`HeaderProfile::uses_github_token` 按 host 门控（`network.rs:136-176`），`X-GitHub-Api-Version` 按 profile 与 URL 发（`:232`，测试 `github_token_never_leaks_to_other_hosts`、`github_api_version_…`） |

**已对齐（无需动）**：`3bee8159`（substitute 最长键优先，`checkver.rs:1107-1121` 已按长度倒序）、`3a79532d`（`$page` 跨迭代；hok 每次取出都是新值，结构上免疫）、`874e8e6f`（脚本无输出时的返回；hok 空页即报错、不返回旧版本）、`79669717`（`hash.mode: github`；`checkver_hash.rs:172` 早已分发，且 hok 不随仓带 `schema.json`）。

## 3. P2 — 新增能力/新键缺失（**不采纳，只记录差距**，2026-10-03 用户拍板）

> **2026-10-07 更正**：`63cb8f84` 的历史清单链（含 `use_git_history`）**已改为落地**——同库新表
> `hok_manifest_history`（只存 `(bucket,name,version) → blob_oid,path,committed_at`）＋ `hok update` 上的种子/增量，
> 读路径接 `manifest_source::generate_user_manifest` 第 2 段；方案
> [`../plan/manifest-history-index-plan.md`](../plan/manifest-history-index-plan.md)，成本优化
> [`../plan/manifest-seed-tree-cost-plan.md`](../plan/manifest-seed-tree-cost-plan.md)。
> `db8d554f` / `198818ae` 两条仍为「不采纳，只记录差距」，见下表与 §6。

| 上游 | 官方行为 | hok 现状（2026-10-03 核对） | 影响 |
|---|---|---|---|
| `63cb8f84` | 新增 config 键 `use_git_history`（默认 `$true`）+ SQLite→git→autoupdate 的历史清单链（`lib/manifest.ps1:183-335`、`libexec/scoop-config.ps1:33-36`） | **已落地（2026-10-07）**。原状：`use_git_history` 已登记为「官方键、hok 无实现」（`crates/libscoop/src/config.rs:1158-1176` 的 `OFFICIAL_ONLY_KEYS`，非报「未知键」）；**无任何调用点**。`@version` **本身是支持的**，只是唯一手段是 autoupdate（`package/manifest_source.rs:422-479` → `checkver.rs:1035-1131`） | 原缺口（重建不出的版本无解：当前 autoupdate 模板套不出该版本的 URL、或 manifest 无 `autoupdate` 段）**已由历史索引补上**：读路径命中即取回原文，未命中/失效才回落 autoupdate |
| `db8d554f` | bucket pull 改为 `git pull --tags --force -q`（`libexec/scoop-update.ps1:139`） | libgit2 fetch，refspec 只含 heads（`internal/git.rs:146-172`），全文无 tag 处理 | **不构成互锁**（见 §3.1）：0.6.0 的历史查询不使用 tag，真正的互锁在 git 历史 provider 的缺失 |
| `198818ae` | `scoop update -f` 把 `@version` pin 升到 bucket HEAD；`$pin_broken`、`Find-AppBucket` 重解析（`libexec/scoop-update.ps1:271-284,443-451`） | pin/URL 安装即 isolated；`-f` 在 installed 集上原地重装当期版本（`sync_install.rs:137-144,328`） | `hok update -f` 不会升到 bucket HEAD（**与上游同口径**：两者都不能"升级"一个 pin，上游的做法是 `-f` 时**丢掉 pin 重新解析**） |
| `e02633c7` | 配置键 `aria2-fallback-disabled` 反转为 `aria2-fallback-enabled`（`libexec/scoop-config.ps1:142`） | **已登记**（`config.rs:1159`，随 `4168ba8`）⇒ 报「官方键、hok 无实现」而非「未知键」 | 仅"无法设置"（用户拍板接受），非能力缺口 |
| `327040cf` | `scoop download` 任一失败即 `exit 1`（`libexec/scoop-download.ps1` 末尾 `exit [int]$download_failed`） | **已落地**：`download_apps.rs:106-148` 返回失败标志，`src/cmd/download.rs:53-60` 转非零退出码（`4168ba8`） | 无。附带**更正**：`scoop-gap-status.md` 里「download 失败按官方改为 0」的口径已过期（0.6.0 起官方为 1） |

**登记去向**：前三条（`63cb8f84` / `db8d554f` / `198818ae`）转入
[`pending-tasks.md`](../plan/pending-tasks.md) §3「对标差距里的暂缓 / 有意未做」（其中 `63cb8f84` 已于 2026-10-07 标为已落地）；后两条已闭环，仅在本表留更正。

### 3.1 上游意图（为什么这么改）

**一条链 + 一条退路**，服务于同一件事：`scoop install app@<旧版本>` 要在 bucket HEAD 已无该版本时**拿到那个版本的原文清单**，而不是"让这个用法从不支持变成支持"。

**先厘清现状（2026-10-03 更正）**：`@<版本>` **本来就是支持的**——官方 0.5.3 与 hok 现在都靠 `generate_user_manifest` 在版本不等时跑 autoupdate 重建（官方 `lib/manifest.ps1:318-334`，hok `package/manifest_source.rs:422-479` → `checkver.rs:1035-1131`：替换 `$version` 后重算 hash，产物落在 workspace（`<root>/workspace/<app>.json`），本机实测 `..\Scoop\workspace\reasonix-hermes.json` 即此产物）。重建成功与否**不取决于旧版本还在不在 HEAD**，而取决于当前模板能否套出那个版本的 URL。0.6.0 的改动是：**在"重建不出"时，把原文按版本号找回来**。

1. **`63cb8f84` 的三级链解决的是"找原文"，而 autoupdate 只是重建**（`lib/manifest.ps1:306-335`）：
   - ① SQLite 精确匹配（`:183-208`，`use_sqlite_cache`，opt-in）—— 快，但**只能命中 git 历史里也有的东西**（索引由 `Sync-Bucket` 按本次 diff 建，`libexec/scoop-update.ps1:202-258`）⇒ 是加速层，不是权威层；
   - ② git 历史（`:210-275`，`use_git_history`，**默认 `$true`**）—— 权威来源，代价高。`log --follow` + pickaxe 按**版本号**回溯提交，拿到的正是当时的 URL/hash/extract_dir：模板改过结构、产物换过名、URL 规则变过时，autoupdate 会失手而原文仍然可用。内部三步也有意图：先 `git show HEAD:<内层相对路径>` 快路径（HEAD 就有时不查历史）→ 双探针（`-G` 正则避开 Windows 命令行吞引号，失败再 `-S` 字面）→ 命中提交的 `<hash>^` 与 `<hash>` 各试一次（覆盖"改版本号那次提交本身与其前一状态"）；
   - ③ autoupdate（`:318-334`）—— 仍保留为最后手段，装出来的是**重建**而非原版。所以三级是"原文优先、重建兜底"，不是"新功能"。
   配置默认值本身是口径：`use_git_history` 默认 `$true` ⇒ 上游认为 git 查询的磁盘/时间代价可接受；SQLite 反而保持 opt-in。上游自述（`libexec/scoop-config.ps1:33-36`）是 *first search the bucket's git history for the exact version before falling back to autoupdate*。
2. **`db8d554f` 的 `--tags` 是同步层的鲁棒性补丁，不是历史查询的前置**：它只加在 `Sync-Bucket` 的「仓库/分支未变」分支（`:139`），`--force` 用于强制对齐本地被改过的 tag；而 ② 的三处 git 调用（HEAD show / pickaxe / show）**一处都没用 tag**。即"按 tag 找历史清单"并不是上游设计——历史按 **bucket 仓库提交历史**找，对应 hok 的等价物是"每个已添加 bucket 的 git 仓库"，而非 tag。
3. **`198818ae` 修的是"pin 之后永久卡死"，判据比看上去窄**：`$force -and $null -eq $bucket -and $url`（`libexec/scoop-update.ps1:271-284`）—— 官方把"URL 安装"与"`@version` 在 workspace 生成的清单"都表示成 **`bucket == $null` + `url`**，所以一条判据同时覆盖两种来源；命中即 `Find-AppBucket` 重解析、清空 `$url`，升到 bucket HEAD。hok 用 `__isolated__` 哨兵表示同一类安装（`constant.rs:41`），因此移植时的等价判据是 `is_isolated()`/bucket 为空，**不能照抄 `url` 非空**。
4. **`e02633c7` / `327040cf` 不是新能力**：前者跟随 `lib/download.ps1` 的实际语义把负向键翻正；后者是一批 code review 修补，把 `download` 的失败写进退出码（顺带更正了 0.5.3 时代"按 0"的旧口径）。
5. **hok 侧的差距与代价**（采纳与否的判据，本次结论仍是**不采纳**）：hok 缺的是"找原文"这一步——`@version` 只有 autoupdate（`manifest_source.rs:422-479`），重建不出即报错（无 `autoupdate` 段时是 `does not have autoupdate capability`）。要接 ②，需要在 bucket 仓库里按版本号回溯提交并读旧 blob；hok 已有现成零件（见 §3.2）。三条都要动 `manifest_source` 这个热文件，故**不排期**——注意这是"少了一条兜底"，不是"`@旧版本` 不能用"。

### 3.2 缓存与 git 的两点实测（2026-10-03）

**① 缓存层不是"另存的历史"，是"扫过的当前版本"——优先它只因为快。**
`database.ps1:93-104` 的表是 `app(name, description, version, bucket, manifest JSON, …)`，主键 `(name, version, bucket)` ⇒ **同一 app 的多版本是并列多行**，`manifest` 存整份清单文本，所以它**确实能**充当"按版本取原文"的提供层。但内容来源只有两处：`Sync-Bucket` 的差分导入（`libexec/scoop-update.ps1:202-258`，仅 `use_sqlite_cache` 开时）与首次构建时的整桶扫描（只扫 HEAD 一次，每 app 一行）。本机实测（`$SCOOP\scoop.db`，51 MB、22311 行）与这一判断吻合：7zip 只有 `26.03` 一行，多数 app 也只有 1–2 版；**只有长期开缓存并持续差分同步，旧版本才会自然沉积下来**。结论：先查缓存是为了快（本地索引点查），权威性属于 git；缓存关了或没积到那一版，仍会落到 git。
（hok 侧的对称事实：hok 也写**同一个** `scoop.db`，`app` 表 schema 与上游兼容、另有 hok 专属的 `hok_manifest_sync(bucket, synced_commit)` 同步基线表，见 `package/manifest_cache.rs:48-79`；其 `INSERT OR REPLACE` 同样按 `(name, version, bucket)` 键——**结构上能并留多版本**，但 `populate()` 只在 HEAD 扫当前版本，所以它不是历史来源。）

**② 提交名约束帮不到查询；"git2 有问题"的说法要收窄。**
main 桶 81855 个提交的命名确实是机器约定（实测样例：`7zip: Update to version 26.03` / `kapacitor@1.8.7: Fix hash (Closes #8584)`），但上游的 pickaxe 查的是**文件内容的变化**（`-G 'version.: .<ver>'`），与提交名无关；实测把同样查询换成 `--grep '^7zip: Update to version 25\.01' -- <path>` 反而更慢（1229 ms vs 976 ms）⇒ 该约定是 Excavator/CI 的规范，**不是**查询加速手段。

耗时实测（本机 main 桶，7zip.json 共 32 次改动；未预热）：

| 路径 | 实测 | 说明 |
|---|---|---|
| HEAD 快路径 `git show HEAD:<rel>` | 192 ms | 命中率取决于 HEAD 版本是否就是请求版本 |
| `log --follow -G`（正则，上游主探针） | 976 ms | 命中 `b14bfe9a`（26.00 那次）；`--follow` 有重命名检测开销 |
| `log --follow -S`（字面兜底） | 3639 ms | 未命中时仍要付这个代价，所以"主探针命中"很重要 |
| SQLite 点查 | ~0 ms | 索引点查（本机缓存关闭时不可用） |

对 git2 的判断应精确为：「读某个提交的某个路径」hok 已具备（`internal/git.rs:414-421` 的 `blob_at`，已被 `last_commit_for_file` 用于 `scoop info` 的 `Updated at/by`），且**沿单文件历史回退的用法很便宜**（该函数只走 32 次提交，注释里 0.63 s 那条即此）；缺的是**内容搜索**（等价 `-G`/`-S`）与重命名跟随（`--follow`）。`git2` 0.19 没有 diff pickaxe API，自己实现等于遍历 8 万提交 × 逐 blob 比对；加上上游 `Invoke-Git` 走的是**原生 git**（`lib/core.ps1:237-279`，`& $git @ArgumentList`）而 hok 的桶操作走 libgit2，所以已存在的 `git log --follow` **进度显示**（上游 `:381-386`）也拿不到。真正要接 ② 时，先决定是加原生 git 依赖还是接受一个更窄的"只在 `blob_at` 链上按版本号找"的实现。

## 4. P3 — 零散差异（本会话在主树做）

| 上游 | 官方行为 | hok 现状（2026-10-03 核对，三条均随 `4168ba8` 已对齐） |
|---|---|---|
| `4c55e7ae` | 卸载脚本里 `$bucket = $install.bucket`，URL 安装为 `$null`（`libexec/scoop-uninstall.ps1:62`） | 已对齐：`operations/script.rs:136-150` 的 `script_bucket()` 把哨兵折成空串，三个注入点（PS 前导 env、`-File` 环境、`expand_scoop_vars`）共用 |
| `7e86336a` | 增删环境变量有可见输出（`lib/install.ps1:341,352`） | 已对齐：`PackageEnvVarSet/Remove` 带 name/value/scope（`event.rs`），`src/scoop_handler.rs:171-190` 每个变量一行，不再只走 `tracing::debug!` |
| `b592b38a` | notes 前后各有 `Notes` / `-----` 分隔（`lib/install.ps1:355-362`），建议行 `DarkYellow`（`:421`） | 已对齐：`output::notes` 出 `Notes` / `-----` 框（`src/scoop_handler.rs:254-256`，复用既有 `cmd.info_notes`），建议行恢复 DarkYellow |

（本表之外的 P3 条目在两处登记：两个 config 键与 `download` 退出码见 §3 表末两行，`locales/*` 侧随 `507aab2`。）

## 5. 判定为无影响 / 已对齐（不必动）

`3bee8159`、`aa3f9aa9`、`25d35007`、`2120c28c`、`ff38652e`、`d9b4d28a`、`2a637361`、`03d831d9`、`a8d4027a`、`a5c3b58e`、`7a537586`、`41ccd2c0`、`ca0506cf`/`0e698ccb`（输出函数化：仅 `Error.`/`Done.` 大小写与 `error` 前缀）、`0f8cd488`（WoW64 路径重写只对 x86 shim 生效，hok-shim 的 stub 是 x64 PE ⇒ 不适用）、`52a035b5`（hok `internal::path::normalize_path` 已解析 `..`，`is_in_dir` 归一化语义等价）。

## 6. 本会话处置与排期

| 优先级 | 内容 | 落点 |
|---|---|---|
| P0 | 元数据文件名（写新名 + 读新名优先回落旧名 + 版本解析/枚举双名） | worktree `.worktrees/p0-metadata-names`，分支 `feat/metadata-scoop-prefix`，base `22b9e68`；**枚举行同属本次落地**（`install_info_mtime` 新名优先，测试 `resolve_current_version_reads_scoop_names_only`），原「遗留」已更正，见 §1 |
| P1 | checkver GitHub 模式整体移植（上表 7 条 + 改掉锁定旧行为的测试） | worktree `.worktrees/p1-checkver-github`，分支 `fix/checkver-github-mode`，base `22b9e68` |
| P3 | `$bucket` 语义、环境变量输出、notes 分隔/高亮、两个新 config 键登记、`download` 退出码 | 父会话主树直接改（`locales/*` 是必冲突热点，不派分支）。**已落地**（`4168ba8` + `507aab2`）：两个 config 键已登记、`download` 退出码已非零；审计文档旧行已就地更正（§3、§4） |
| 已改判 | P2 的 `63cb8f84`（历史清单链 + `use_git_history`）：2026-10-03 记为「不采纳」，2026-10-06 立方案、**2026-10-07 落地**（`615a6bb`…`9db5d13`） | 方案 [`../plan/manifest-history-index-plan.md`](../plan/manifest-history-index-plan.md)；行状态见 [`pending-tasks.md`](../plan/pending-tasks.md) §3 |
| 不采纳 | P2 的 `db8d554f`（`git pull --tags --force`）/ `198818ae`（`update -f` 丢 pin）两条（2026-10-03 用户拍板只记录差距） | 已登记进 [`pending-tasks.md`](../plan/pending-tasks.md) §3；设计与意图见 §3.1 |

**门禁策略（用户 2026-10-02 选定）**：每条子分支自测一次（`cargo fmt --check` / `clippy -D warnings` / 目标测试），落地后父会话在最终树上跑一次完整门禁；不在每条分支重复跑完整套。

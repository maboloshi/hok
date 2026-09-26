# H3 设计报告：清单解析失败的可见性策略（2026-09-18）

> **会话/任务**：W2-A（= 本表 §1 H3 / fix plan §8 R3）
> **worktree**：`.worktrees/w2a-parse-visibility`，分支 `fix/parse-visibility`，base = 主线 `refactor/edition-2024` tip `4379548`
> **状态**：**决策门——本报告之后停下等确认，尚未改任何生产代码**
> **硬禁区（用户给定）**：① 不改 `internal::json` 的宽容度契约；② 不把严格解析改回 `warn→null`。

---

## 0. 一句话结论

hok 的**类型化** `ManifestSpec` 解析比官方 `ConvertFrom-Json` **严格得多**：本机 4 个真实 bucket 21710 个清单里 **80 个 hok 拒收**，而 **80/80 官方两个宿主都接受** ⇒ 当前"解析失败静默消失"的**全部**影响都是 hok 自身解析器过严造成的，不是清单真的坏了。
因此建议：**本次只做"可见性"（显式查询逐条警告 + 全量扫描汇总计数 + `checkver`/`checkurls` 对齐官方警告），不做硬报错；解析器宽容度的 80 个缺口另立工作流。**

---

## 1. 现象与复现（隔离夹具，实测）

夹具：临时 scoop root，`buckets/testb/bucket/` 放 4 个清单（`good` 正常；`broken` 语法错；`typedbad` 合法 JSON 但 `license` 缺 `identifier`；`dupkey` 合法 JSON 但重复键 `bin`）。设 `SCOOP=<夹具根>` 后逐命令实测（二进制为 worktree 自建副本 `target/verify/hok.exe`）：

| 命令 | 遇到不可解析清单的实际行为 | 用户可见？ |
|---|---|---|
| `hok search`（全量） | 该包**从列表消失**，无任何提示 | ❌ 静默 |
| `hok search -e <name>` | `WARN 未找到匹配项。` exit 1 | ⚠️ 误导 |
| `hok info <name>` | `ERROR 找不到匹配 '<name>' 的任何包。` exit 1 | ⚠️ 误导 |
| `hok depends <name>` | `ERROR 找不到名为 '<name>' 的包。` exit 1 | ⚠️ 误导 |
| `hok install <name>` | `ERROR 找不到名为 '<name>' 的软件包` exit 1（`-vv` 才多一行 tracing WARN） | ⚠️ 误导（默认静默） |
| `hok virustotal <name>` | `ERROR 没有找到包。` exit 8 | ⚠️ 误导 |
| `hok cat <name>` / `hok home <name>` | `ERROR missing field \`identifier\` at line 1 column 102` | ✅ 已响亮 |
| `hok checkhashes -d <dir> <app>` | `ERROR <name>` + `清单解析失败：<reason>` | ✅ 已响亮 |
| `hok missing-checkver -d <dir>` | 官方文案 `Error parsing JSON at '<path>'.` + `N 无法解析` 汇总 | ✅ 已响亮 |
| `hok checkurls -d <dir> <app>` | **该清单被跳过**，输出"没有需要检查的清单" | ❌ 静默 |
| `hok checkver -d <dir> <app>` | **该清单被跳过**，零输出、exit 0 | ❌ 静默 |
| `hok formatjson -d <dir>` | `ERROR <path>: parse error: ...`（走得是 raw `Value` 路径） | ✅ 已响亮 |

复现命令（夹具）：
```powershell
$hok = ".worktrees/w2a-parse-visibility/target/verify/hok.exe"
$env:SCOOP = "$env:TEMP\w2a-fixture"
& $hok info typedbad          # → 找不到匹配 'typedbad' 的任何包
& $hok install typedbad -vv   # → 才看得到 "failed to parse manifest ... missing field `identifier`"
```

真实 bucket（用户本机，`SCOOP=D:\App\Scoop`）同型：
```powershell
& $hok info 5ire        # → ERROR 找不到匹配 '5ire' 的任何包。(exit 1)
& $hok search -e bibclean  # → WARN 未找到匹配项。(exit 1)
& $hok search 5ire      # → 只列出 5ire_windedge（正则命中别的包），5ire 本身不在列表
```

---

## 2. 官方对照（这是"要不要做"的关键）

| 路径 | 官方 Scoop | hok | 判定 |
|---|---|---|---|
| 单文件解析入口 | `lib/manifest.ps1:5-12` `parse_json`：**`warn "Error parsing JSON at '$path'."` 然后 `$null`** | `package/manifest.rs:843-845` tracing `warn!`（`-vv` 才可见） | hok **更安静** |
| `install` 命中坏清单 | `Get-Manifest` → `parse_json` **先 warn**，再报 "Couldn't find manifest for 'x'." | 无 warn（默认），只报"找不到软件包" | **偏离** |
| `checkver` / `checkurls` | 同上，**每个文件一条官方 warning** | `checkver.rs:212-215` / `checkurls.rs:44-47` 的 `Err(_) => continue` | **偏离**（审计已记） |
| `search` / `list --available`（DB 路径） | `lib/database.ps1:173-176` `ConvertFrom-Json -ErrorAction SilentlyContinue` + `$null -ne $manifest.version` ⇒ **同样静默丢弃** | `manifest_cache::insert_manifest_row:282-284` `Err(_) => return Ok(None)` | **不偏离**（但官方解析器几乎不失败，故其静默是"惰性"的） |
| `missing-checkver` | 逐文件 warning + `[ ][ ]` 行 | 已完全对齐（S11 落地） | ✅ |
| `checkhashes` | hash mismatch；非法 hash 官方能解析 | 已显式报解析失败（S10 决策） | ✅ |

> **要点**：官方唯一的"响亮点"就是 `parse_json` 的那条 `warn`；hok 把它搬进了 `parse_file` 的 **tracing** `warn!`，于是同时失去了三个可见性：
> 1. 默认 `-v` 不够，要 `-vv`；
> 2. **缓存路径根本不调用 `parse_file`**（用户本机 `use_sqlite_cache: true`）；
> 3. `parse_file` 的 **非 UTF-8 分支没有 `warn!`**（`manifest.rs:837-842` 的 `from_utf8` 早退，`inspect_err` 只挂在 `parse_str` 上）。

### 2.1 ⚠️ 更正：`use_sqlite_cache` 默认是 `false`（`config.rs:486`）⇒ 存在两条失败形态

实测（夹具，分别用 `use_sqlite_cache=false/true` 跑同一组命令）：

| 配置 | `info typedbad` | `search -e typedbad` | 信号 |
|---|---|---|---|
| **缓存关（默认）** | `找不到匹配 …` | `未找到匹配项` | `parse_file` 的 tracing warn **确实发了**，`-vv` 可见 |
| **缓存开（本机）** | 同上 | 同上 | **`-vvv` 也没有任何信号**（不经 `parse_file`） |

⇒ 默认路径（缓存关）是"信号存在但埋太深"；本机路径（缓存开）是"信号根本不存在"。
**同一个决定必须同时覆盖两条路径，且两条路径的修法不同**（§4.1 S1 vs S2）。

---

## 3. 量化：本机真实 bucket（2026-09-18）

### 3.1 规模与不可解析数量

| bucket | `bucket/*.json` | 不可解析 | 占比 |
|---|---|---|---|
| extras | 2387 | **0** | 0 |
| main | 1649 | **0** | 0 |
| scoop-app | 17616 | **80** | 0.46% |
| scoop-private | 58 | **0** | 0 |
| **合计** | **21710** | **80** | 0.37% |

- 命令（用 worktree 自建副本，`RUST_LOG=libscoop=warn` 抓路径+原因）：
  ```powershell
  $env:RUST_LOG="libscoop=warn"
  & $hok missing-checkver -d D:\App\Scoop\buckets\<bucket>\bucket
  ```
  `missing-checkver` 汇总行：`已扫描 17535 个清单: … 80 无法解析。`
- **对照 P0-4**（`manifest-parse-audit.md`）：修复前 220 不可解析，`eb67941` 之后仍有 **80** ⇒ 这是**持续渗漏**，不是一次性事故。

### 3.2 ★ 关键发现：80/80 官方都接受

用官方 Scoop 的**同一个解析器**（`ConvertFrom-Json`，即 `parse_json` 的实现）逐文件复跑那 80 个：

```powershell
# 对 80 个路径逐个： Get-Content <p> -Raw -Encoding UTF8 | ConvertFrom-Json -ErrorAction Stop
pwsh       -NoProfile -File w2a-official-parse.ps1   # → ps=7.6.6      total=80 ok=80 fail=0
powershell -NoProfile -File w2a-official-parse.ps1   # → ps=5.1.26100  total=80 ok=80 fail=0
```
（对照组：`{ not json` 在 PS 7 下按预期报错 ⇒ 脚本本身有效。）

**结论**：当前 80 个"不可解析"**没有任何一个是官方 Scoop 装不上的**。也就是说，hok 现在表现出的"找不到软件包"，在官方那里是"能搜到、能装"。这正好复刻了 P0-4 的形状（官方接受 → hok 整条拒收 → 查询层静默 → 用户看到"不存在"）。

唯一需要打折扣的两个：
- `FeiQ.json`（603 B）**确实不是合法 UTF-8**（偏移 492 处一个 GBK 字节 `0xB7`）；PS 只是**有损替换**成 U+FFFD 才"接受"。hok 拒收这里**更正确**，但**完全静默**（见 §2 要点 3）。
- `PolicyPlus.json` 是 **0 字节空文件**；`ConvertFrom-Json` 对空输入不报错。

### 3.3 失败原因分类（80 条）

| # | 原因（serde 报的） | 数 | 形态 | 官方为何接受 |
|---|---|---|---|---|
| 1 | `invalid type: map, expected a string`（多在 `checkver.jsonpath`，行 18-67） | **24** | `"jsonpath": { "version": ..., "download": ..., "hash": ... }` 对象形式 | schema 只允许 string，但**官方运行时不校验 schema**；对象被 `[String]` 参数强转，install 阶段根本不看 `checkver` |
| 2 | `invalid hash string: …` | **12** | 8 个是 `"hash": " "` 空白占位；1 个 `---`；2 个裸 40-hex（sha1 长度）；1 个 65-hex | **P0-1**（`REGEX_HASH` 强绑定长度组合）官方 hashPattern 不校验 |
| 3 | `duplicate field \`bin\` / \`url\` / \`installer\` / \`extract_dir\` / \`post_install\`` | **7** | JSON 重复键 | `ConvertFrom-Json` **取最后一个**，serde 拒绝重复键 |
| 4 | `missing field \`identifier\`` | **7** | `"license": { "url": ... }` | 官方 schema 有 required 但不校验 |
| 5 | `invalid type: string "true", expected a boolean` | **6** | `"keep": "true"` | PS 自动把 `"true"` 转 bool |
| 6 | `invalid type: string "X.exe", expected a sequence` / `invalid type: map, expected a sequence` | **6** | `shortcuts` 扁平数组、`shortcuts` 对象数组 `{name,target}` | 官方 install 逐项 `@()` 包装，容错 |
| 7 | `invalid type: string "openssl"/"imagemagick"/"wavedb", expected a map` | **3** | 该处应为 map 却给了 string | 官方忽略 |
| 8 | `invalid type: string "…", expected struct Uninstaller / Autoupdate` | **2** | `"uninstaller": "------TODO------"`、`"autoupdate": "<url>"` | 官方忽略 |
| 9 | `checkver hash extraction 'regex' must be a string` | **2** | hok 自定义 visitor 的额外约束 | 官方无此约束 |
| 10 | `invalid type: map, expected a valid hash string` | **2** | 顶层 `"hash": { "url": "$baseurl/checksums.txt" }` | 官方忽略 |
| 11 | 其它单例（`floating point 1.43`、`integer 2`、`EOF`、map/string 等） | **9** | 混杂 | 官方忽略 |

> 这张表**不是**本次要修的东西（见 §5 O5），它说明的是：**"不可解析"在 hok 里几乎等价于"hok 解析器还不够宽"**，所以"静默"的代价被放大到了 80 个真实包。

### 3.4 缓存层的额外静默（新增发现）

用户本机 `use_sqlite_cache: true`，`search`/`info`/`depends`/`list -a` 走缓存路径：

```powershell
sqlite3 "file:D:/App/Scoop/scoop.db?mode=ro&immutable=1" "select bucket,count(*) from app group by bucket;"
# extras|2387  main|1649  scoop-app|17597  scoop-private|58   → 合计 21691 行
& $hok search   # 实际输出 21618 行
```
- **缓存里有 21691 行，`hok search` 只列出 21618 ⇒ 73 行"缓存认为存在、但任何查询都看不见"，且无任何提示。**
- 80 个不可解析里 **79 个在缓存中留有行**（`select name from app where name in (…)` → 79）。它们不可见的链路是 `query.rs:388`：
  ```rust
  let mut manifest = manifest_cache::entry_to_manifest(entry)   // 缓存里的 raw JSON 也过不了类型化解析
      .or_else(|| load_bucket_manifest(&root_path, bucket, name))?;  // 回读文件也失败 ⇒ 整条 `?` 丢掉
  ```
- 实测确认：`search` 输出里这 80 个 **一个都不在**（0/80）；`search -e 5ire`、`search -e bibclean`、`info bibclean` 全部"找不到"。

---

## 4. 影响面：谁被静默吃掉

### 4.1 静默点清单（worktree = 主线 tip `4379548`）

| # | 位置 | 机制 | 影响命令 |
|---|---|---|---|
| S1 | `package/query.rs:285` | `filter_map` + `if let Ok(...)`, else `None` | `install` / `update` / `upgrade` / `reinstall`（`sync_install.rs:199`、`resolve.rs:111`） |
| S2 | `package/query.rs:388` | `entry_to_manifest(..).or_else(load_bucket_manifest(..))?` | `search`(默认) / `info` / `depends` / `virustotal`（缓存路径） |
| S3 | `package/query.rs:120` | `if let Ok(..) = parse_file(manifest.json)`（apps 目录侧） | `list` / `--broken` 之外的已安装查询 |
| S4 | `package/install_state.rs:216` | `parse_file(..).ok()` | 可升级判定（`maybe_fill_upgradable:243` ⇒ 当成"不可升级"）|
| S5 | `package/manifest_cache.rs:282-284` | `Err(_) => return Ok(None)` | `hok update` 建缓存时**静默不入库** |
| S6 | `package/manifest_cache.rs:587` | `entry_to_manifest` `.ok()` | 缓存行不可用 ⇒ 回落文件 ⇒ 再失败即丢 |
| S7 | `package/checkver.rs:212-215` | `Err(_) => continue` | `checkver` / `auto-pr` |
| S8 | `package/checkurls.rs:44-47` | `Err(_) => continue` | `checkurls` |
| S9 | `package/manifest_source.rs:227/230/247/254/256` | `.ok()` 变体 | isolated/URL 清单回落链 |
| S10 | `package/sync_install.rs:730` | `parse_file(..).ok()` | 旧清单读取 |

### 4.2 已响亮的点（保持不变，避免重复告警）

`checkhashes.rs:92-110`、`missing_checkver.rs:84-114`、`formatjson`（raw `Value` + `schema_warnings`）、`cat`/`home`（经 `manifest_source::get_manifest` 返回类型化错误）。

---

## 5. 候选方案（含"不做"）与代价/风险

### O0 — 保持静默（不做）
- **代价**：0。
- **风险**：P0-4 那类事件会重演——hok 解析器的一个缺口 + 查询层静默 = "220 个包凭空消失"，且用户没有任何线索；上一次定位花了一整个会话。当前仍有 80 个 + 73 条不可达缓存行。
- **理由**：官方 `search` 其实也静默，看似"不偏离"；但官方解析器几乎不失败，**它的静默是惰性的，hok 的不是**。

### O1 — 硬报错（任一清单不可解析即中止命令）
- **代价**：小（`filter_map` 改 `collect::<Fallible<Vec<_>>>()` + 各调用点传播）。
- **风险**：**极高**。scoop-app 有 80 个坏清单 ⇒ `hok search` / `hok install` 对整桶直接不可用。作默认不可接受。仅"**显式查询命中**"这一种作用域下等价于 O2a，没必要单列。

### O2 — 逐条警告（对齐官方 `parse_json`），继续
- **O2a（推荐）显式查询作用域**：用户点名 `<name>`，桶里**存在**该文件但解析失败 ⇒ 除现有"找不到"外，追加一条 `warn`：`清单解析失败 <path>：<reason>`。每次用户查询**最多 1 条**。
  - 覆盖 P0-4 场景（`install`/`info`/`depends`/`search -e`/`virustotal`）。
  - 代价：中。`Session` 不是 `Sync` ⇒ rayon 扫描里只能收集，出循环后再 emit（`query_synced_cached` 已有 `CheckedHit`+`warn_if_cache_mismatch` 的先例可照抄）；`query_synced_files` 需加一个 `Vec<…>` 累加器；需要判断"名字存在于桶里但文件坏了"——显式模式下可按名探测 `<bucket>/<name>.json`（有界的单次 stat/read，不扫全桶）。
  - 风险：中。文案进入 CLI 契约；`tests/exit_codes.rs`、i18n 测试可能要更新；`locales/{en,zh}.yml` 是**热点文件**（与 W2-B 必须错开）。

- **O2b 全量扫描作用域**：`search`(无查询)/`list -a`/`checkver` 全量，每个坏清单一条警告。
  - 代价：同 O2a。
  - 风险：**噪声**。本机语料一次 `hok search` 会刷 80 行，而其中大部分是 hok 自己太严（§3.3），用户无法据以行动。

### O3 — 计数 + 一行汇总（细节留 `-v`）
- **代价**：中（同 O2 的管道 + 每命令一个汇总渲染）。与 `missing-checkver` 的 `unparsable` 一档、`checkhashes` 的 `failed` 计数**同构**，有先例。
- **风险**：低噪声，但"N 个清单无法解析"不给路径，容易被忽略；且必须在**每一条**扫描命令上都发，否则"永不静默"的承诺破功。与 `checkhashes`/`missing-checkver` 已有的逐项报告有重复计数风险，需明确分工。

### O4 — 在单入口（`internal::json` / `parse_file`）暴露
- **代价**：高。`parse_file` 是**无 `Session` 的自由函数**，被 rayon worker 调用，也被多个**合法 best-effort** 站点用（`formatjson::schema_warnings`、isolated 回落链、缓存回落）。要把 sink 传进去得改遍调用点，并违反该模块"只做解析"的文档契约。
- **风险**：高，且属于 `no-patchwork-fixes` 明确禁止的"借用别的层的语义"。
- **结论**：**否决**。可保留的只有一处**小修**：给 `parse_file` 的**非 UTF-8 早退分支**补上同款 `warn!`（见 §2 要点 3），这是补漏不是改契约。

### O5 — 收窄解析器（把 80 降到接近 0）——**本次不做，另立工作流**
- 事实基础就是 §3.2/§3.3：80/80 官方接受 ⇒ 这些是 hok 的**解析缺口**，逐个都是 P0/P1 级决策（`manifest-parse-audit.md` 的 P0-1 仍在、P0-3 未修；第 3 名"重复键取最后一个"是 **hok 独有**的严格点，官方明确取末值）。
- 与 `internal::json` 宽容度契约**无关**（那是 JSON 语法层；这 80 个绝大多数是**语法合法、类型/shape 不匹配**），所以不违反用户禁区；但它是**独立主题、独立风险**，混进本次会让"可见性"这个决定被解析器行为变化淹没。

---

## 6. 推荐方案

**O2a + O3 组合，一次只做可见性：**

1. **显式查询**（`install`/`info`/`depends`/`search -e`/`virustotal`/`replace`）——当"桶里存在该名字的文件、但它解析失败"时，输出一条 **warn**：路径 + 真实原因，**保留现有的 not-found 错误与退出码**（脚本行为不变，只是不再误导）。
2. **全量扫描**（`search` 无查询 / `list -a` / 缓存路径）——**计数**并在末尾打 **一行汇总**；逐条路径只在 `-v` 展开。
3. **`checkver` / `checkurls`**——补官方原文 `Error parsing JSON at '<path>'.`（纯对齐 `lib/manifest.ps1:10`；审计已把这条记为偏离）。
4. **`parse_file` 非 UTF-8 分支**补同款 `warn!`（目前完全无告警，`FeiQ.json` 即此例）。
5. **缓存层**：至少让"缓存行存在但不可用"（`query.rs:388` 的两个 `?`）落入同一套可见性；是否顺带在 `hok update` 的 `populate`/incremental 做汇总见 D3。
6. **不做**：不硬报错；不动 `internal::json`；不恢复 `warn→null`；**不在本次动解析器宽容度**。

**预期落地拆分**（确认后再开）：
1. `fix(query): surface manifest parse failures instead of dropping them`（query/files+cached、install_state、checkver、checkurls、parse_file UTF-8 分支）
2. `test(query): pin the visibility policy`（隔离夹具逐命令断言 + 全量扫描汇总）
3. `i18n`（`locales/en.yml` + `locales/zh.yml`）

---

## 7. 需要你拍板的点（一次问清）

- **D1**：默认输出形态选哪个？
  - (a) **显式查询逐条警告 + 全量扫描仅汇总**（推荐，噪声可控）
  - (b) 全量扫描也逐条警告（最响，本机一次 `hok search` 80 行）
  - (c) 只做汇总，逐条一律留 `-v`
- **D2**：显式查询命中坏清单时，**退出码**保持 1（仅文案变化）还是改成另一种解析错误码？**推荐保持 1**（不破坏脚本）。
- **D3**：缓存/`hok update` 侧是否一并提示？现状 `scoop.db` 有 **73 行不可达**、79 行残留。**推荐本次至少覆盖"缓存行不可用"这一条**；`populate` 的汇总可留到后续。
- **D4**：80 个解析缺口（§3.3）**确认另立工作流**吗？**推荐是**——本次只做可见性，否则"可见性决定"会被解析器改动淹没，也无法单独验证。
- **D5**：`checkver`/`checkurls` 的警告文案用**官方原文**（`Error parsing JSON at '<path>'.`）还是 hok 风格（带 reason）？**推荐官方原文 + 附加 reason**，与 `missing-checkver`（S11 已对齐）保持一致。

---

## 8. 可复跑的证据命令（父会话/用户复核用）

```powershell
# 0) 构建（worktree 内，先 hok-shim 再 hok）
cd .worktrees/w2a-parse-visibility
cargo build -p hok-shim ; cargo build -p hok
Copy-Item target\debug\hok.exe target\verify\hok.exe -Force

# 1) 真实语料：不可解析数量 + 逐条原因（RUST_LOG 抓 manifest.rs 的 warn!）
$env:RUST_LOG="libscoop=warn"; $env:SCOOP="D:\App\Scoop"
.\target\verify\hok.exe missing-checkver -d D:\App\Scoop\buckets\scoop-app\bucket 2>&1 |
  Select-String "failed to parse manifest|Error parsing JSON|无法解析"

# 2) 官方宿主裁决（80/80 接受）
pwsh       -NoProfile -File $env:TEMP\w2a-official-parse.ps1 -OutFile $env:TEMP\w2a-ps7.xml
powershell -NoProfile -ExecutionPolicy Bypass -File $env:TEMP\w2a-official-parse.ps1 -OutFile $env:TEMP\w2a-ps51.xml

# 3) 缓存不可达行
sqlite3 "file:D:/App/Scoop/scoop.db?mode=ro&immutable=1" "select count(*) from app;"   # 21691
.\target\verify\hok.exe search | Measure-Object -Line                                   # 21618

# 4) 每命令可见性（夹具）：见 §1 表
```

**worktree 现状**：`.worktrees/w2a-parse-visibility`，分支 `fix/parse-visibility`，HEAD = `4379548`（**零提交、零代码改动**）；`target/verify/hok.exe` 为自建副本。

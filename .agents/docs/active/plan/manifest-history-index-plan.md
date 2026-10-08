# 清单历史索引正式方案

Status: active
Updated: 2026-10-06

> **这是什么**：「hok 能在 bucket HEAD 已无该版本时，按版本号取回原文清单」这条能力的正式方案，
> 承接调研草稿 [`.agents/docs/research/manifest-history-index.md`](../../research/manifest-history-index.md)。
> 调研已定的三条结论（只存 OID 不存文本、同库新表、git2 驱动不走 pickaxe）在本方案里被继承，
> 本文补齐草稿 §4 列出的六件事：索引表 schema、种子与增量构建、失效回落、`manifest_source` 接入点、
> 提交拆分、验收标准。**状态：实现中——提交 1–2 已落地（§5），提交 3–5 待做**。
>
> **2026-10-06 修订**（实现反馈）：§1/§2/§3 各有一处原设计会在实现里出错，已按下文更正
> （基线列语义、增量路线、入口门控），并把两条**原方案未登记**的已知弱化写进 §1/§2.1。
> hok 侧的行号锚点是写作当日快照，改动后可能漂移；能指名函数的一律指名函数。

## 0. 问题一句话

`hok install app@<旧版本>` 目前只有一条路：HEAD 清单的 autoupdate 段重建（`manifest_source.rs:422-479`
→ `checkver.rs:1035-1131`）。重建不出时（模板改过结构、产物改名、URL 规则变过、manifest 无
`autoupdate` 段）**没有退路**，直接报错。上游为此加了三级链（`../Scoop/lib/manifest.ps1:306-335`）：
`sqlite 缓存 → git 历史 → autoupdate`；本方案实现其中第二级，第一级改写为「同一张库里的轻量索引」。

官方锚点（`../Scoop/lib/manifest.ps1`）：
- `Find-HistoricalManifest`（`:277-303`）编排三级链，受 `USE_SQLITE_CACHE`（`:184`、`:281`）与
  `USE_GIT_HISTORY`（`:211`，默认 `$true`；`:292` 再查一次）门控；
- `Find-HistoricalManifestInGit`（`:210-275`）走 `git log --follow -n 1 -G/-S` pickaxe——实测
  976 ms / 3639 ms 每次查询（草稿 §2）；本方案用**预建索引**替代，点查毫秒级。注意它只取
  **最新**命中（`-n 1`），且 `--follow` **会追重命名**——这两点分别对应 §2.3 与 §1 的已知局限；
- 清单路径由 `Find-BucketDirectory`（`../Scoop/lib/buckets.ps1:3-29`）解析：有 `bucket/`
  子目录就用它，否则用仓库根——即**扁平桶的清单在根目录**，索引的路径过滤必须覆盖这种布局；
- `generate_user_manifest`（`:306-335`）的顺序是：版本相符直接返回 → **历史链** → autoupdate；
  历史链落空且无 autoupdate 时 `abort`（`:320`）。

## 1. 索引表 schema

同库 `{SCOOP}/scoop.db`（`manifest_cache::open` 已开的连接），新表：

```sql
CREATE TABLE IF NOT EXISTS hok_manifest_history (
    bucket      TEXT    NOT NULL,
    name        TEXT    NOT NULL COLLATE NOCASE,
    version     TEXT    NOT NULL,
    blob_oid    TEXT    NOT NULL,   -- 40 位 hex，git blob OID
    path        TEXT    NOT NULL,   -- 仓库相对路径，'/' 分隔
    committed_at INTEGER NOT NULL,  -- Unix 秒（把它带进本谱系的提交时间）
    PRIMARY KEY (bucket, name, version)
) WITHOUT ROWID;
```

- **主键 `(bucket, name, version)`**：与草稿 §3.2 一致；`name COLLATE NOCASE` 与 `app` 表对齐
  （见 `manifest_cache::open` 的 `app` 建表）——bucket 侧文件名大小写不敏感是 Windows 的既有事实。
  该列的 COLLATE 确实参与主键比较（已由行为测试钉住：同 `(bucket,name)` 不同大小写会撞主键）。
- **`WITHOUT ROWID`**：主键即行存储，16 万行的点查少一次 rowid→叶页跳转；对 20–30 MB 的表收益正、代价零。
- **`blob_oid` 存 hex 文本（40 B）而非整数**：与 git 输出同形，读出即可与 `git show` / `git cat-file`
  对照；表体积估算 100 B/行（草稿 §3.1）已含此取舍。不用 `BLOB(20)`——省 20 B/行却让人工排查
  「这个 OID 对不对」需要解码，不值。
- **`path` 必须留**（草稿 §4 重命名问题）：`diff_manifest_changes` 不做重命名检测
  （`internal/git.rs`），清单改名 = 旧路径 deleted + 新路径 added，历史版本会挂在旧名下。
  索引键用 `(bucket, name)` 保留**按文件名**的查询能力，`path` 保留回指（人工排查时能直接定位到
  该版本落在仓库的哪个路径）。
  **第一期不做 `find_similar` 重命名检测**——名字从 `old.json` 变 `new.json` 时「同名查询」本来就
  查不到，把版本挂到新名下反而需要跨文件内容比对，超出第一期范围。
  ⚠️ **这是一条明确的弱化，不是等价**：上游的 `--follow` 能沿重命名回溯到旧路径的历史，所以
  `install new@<改名前的版本>` 在上游能命中、在本索引会落空并回落 autoupdate。触发再评估的信号：
  真实用户报告「改名后旧版本装不上」。
- **不存清单文本**（方案 B，草稿 §3.1）：文本仍在 git 里，索引只回答「哪个版本在哪」。
- **不进 `use_sqlite_cache` 门控**（草稿 §3.2 约束 1）：读写路径均不查该开关。
  ⚠️ 这是**有意偏离上游**：上游的 sqlite 层是受该开关门控的（`manifest.ps1:184`、`:281`），
  而本表是「按版本找原文」的对标能力，不该因为没开上游缓存而失效。
- **`app` 表的 `DELETE … version <> ?` 绝不套到本表**（约束 3）：`app` 是「当前版本加速器」，
  本表是「历史索引」，语义不同。本表只在两处删行：**读路径的显式失效**（§4.2）与
  **桶被移除时的清理**（下一条）。
- **桶移除要清理**：`app` 表有 `drop_buckets_not_added`，本表需要同名逻辑。否则桶被删（或同名桶
  改指另一个仓库）后旧行仍在——桶名可复用，旧行的 OID 若恰好在新仓库里存在，就会把**别人的清单**
  当作请求版本返回。清理时索引行与基线行一并删。
- **基线**：与 `hok_manifest_sync` 同表模式，另设一行键，**两列**：

  ```sql
  CREATE TABLE IF NOT EXISTS hok_manifest_history_sync (
      bucket         TEXT PRIMARY KEY,
      indexed_commit TEXT NOT NULL,
      seeded         INTEGER NOT NULL DEFAULT 0
  );
  ```

  `indexed_commit` 在一生中承担两种语义，`seeded` 是它们的判别位：
  - `seeded = 0`（**续跑游标**）：种子沿第一父链向下推进时，它表示**已扫到的最老提交**，
    重跑从它开始（含）继续向下；
  - `seeded = 1`（**增量锚点**）：种子走到根之后，它表示**已覆盖的最新提交**，增量从新 HEAD
    向下扫到它为止。

  **为什么必须有这一位**：只有一列时，一个被中断的种子会看起来「已锚定」——下一次增量把
  `indexed_commit` 一改成 HEAD，就把**没扫过的更老半段**永久标记为完成，静默丢历史。
  与索引行**同库同事务**推进，不另开文件（草稿 §3.2 的决定性理由：状态只有一份）。

## 2. 种子构建（冷启动）

**入口**：`hok update` 里、清单缓存刷新之后，检测到某已加桶没有可用基线 ⇒ 走种子路径。
不新增独立命令——种子是一次性成本，挂在既有的「桶前进」时机上最自然，也避免用户学新动词。

**算法**（草稿 §3.3 的「遍历相邻提交 diff」路线，不逐 app 回溯）：

1. 从 `HEAD` 开始，沿**第一父链**游走（`commit.parent(0)`），每步 `diff_tree_to_tree(parent_tree, commit_tree)`。
   游走的第一个提交就是 HEAD，所以「只出现过一个版本、之后再没改过」的清单也会被它的根提交
   diff 捕获（**不需要**额外的「先扫一次当前树」）。
   只走第一父的已知代价：**merge 解析时被丢掉的分支版本查不到**。具体反例——基线 v1 → 侧枝 v2 →
   主线 v3 → merge 保留 v3：v2 在 git 里可达（上游 `:249` 的 pickaxe 能查到、能装），而第一父链在
   merge 处看不到变化 ⇒ 索引无此行 ⇒ 回落 autoupdate，可能直接 `abort`（`:320`）。
   接受该取舍的理由：桶历史近线性，且失败方向是「回落」而非「装错」。触发再评估的信号：真实用户
   报告某个侧枝版本装不上；届时的选项是对全部父链建索引（成本显著上升）。
2. 每个 delta 的路径是 `.json` 且非 `package.json`（同 `diff_manifest_changes` 的过滤），
   游走直接给出**新侧 blob OID**；用该 OID 读文本，**只取 `version`**：一个窄探针
   `VersionProbe { version: Option<String> }` 走 `internal::json::parse_str`（与全仓同一条
   严格 JSON → json5 链，所以上游接受、只有 JSON5 才成立的清单也照样入索引），
   解析失败或 `version` 为空 ⇒ 跳过（与 `insert_manifest_row` 的可缓存判据一致）。
   **不要退回整份 `Manifest::from_json`**：那是 2026-10-06 实测过的写法，代价更高，而且把
   `License`/`Sourceforge` 那两个自定义 visitor 拉进了「不可信历史文本」这条路径——
   `license` 里一个未知键就足以让 json5 的 `Map::next_value_seed` `unwrap` 空队列，
   两个 profile 都是 `panic = "abort"` ⇒ 整个 `hok update` 进程 abort（见 §5 的前置修复提交）。
   窄探针的派生反序列化不会多做一次 `next_value`，构造上不可能踩到那条路径。
   注意：**不能用 pathspec 表达这个过滤**——libgit2 的 pathspec 最终走字面 fnmatch，
   `*` 不跨 `/`，`bucket/*.json` 会静默一条都不匹配（实现期实测），必须无 pathspec 建 diff、
   逐 delta 判路径。同理**不要**要求路径以 `bucket/` 开头：扁平桶与分类桶的清单分别在仓库根和
   分类子目录（`Bucket::path_of_manifest` 与 `manifest_walker::discover` 都接受），加前缀会让
   这两类桶一行都建不出来。
3. 写入：`INSERT OR IGNORE`（主键冲突 = 同 `(bucket,name,version)` 已有记录，保留**先写入者**，
   即离 HEAD 更近的提交——游走从新到旧，先遇到的更新，与上游 `-n 1` 取最新命中一致）。同事务推进
   游标，**每 N=2000 提交提交一次事务**：中断后重跑从游标处（含）继续——该提交被重复处理，
   但行是 `INSERT OR IGNORE`，等价幂等。**只有走到根提交**才把 `seeded` 置 1 并锚定到 HEAD；
   因 50 万提交安全上限而中止时保留游标，下次接着扫。

**并行（2026-10-06 实测后改为实现）**：父链**游走**不可并行（链式依赖），但它便宜到可以忽略——
只读 commit 对象、连一棵树都不读（`HistoryRepo::walk_chain`）。贵的那半（每提交一次
`diff_tree_to_tree` + 读 blob + 取 version）**与提交之间互相独立**，所以按 500 提交一块丢给
rayon 线程池，每个 worker 自开一个 `Repository`（libgit2 句柄既非 `Sync` 也不可共享），
行仍在单线程里**按链序**写库、按块边界推进游标——`INSERT OR IGNORE` 的「先写入者=更新者」
语义与中断续跑语义都不变。草稿 §3.4 的「按 app 切分并行」仍然**不做**：那会让每个 worker 每提交
都去解压同一棵大 `bucket/` 树，是 O(app × commit) 的倒退。

**成本归因（2026-10-06 实测，原稿的归因被推翻）**：单线程跑 main/extras 时
（临时埋点，已移除）：

| 桶 | 总计 | walk+diff | blob 读 | version 解析 | INSERT |
|---|---|---|---|---|---|
| main | 119.7 s | 109.6 s（92%） | 3.9 s | 0.5 s | 3.1 s |
| extras | 190.3 s | 177.0 s（93%） | 5.5 s | 0.7 s | 4.0 s |

- 原稿「大头不是树 diff，而是把约 16 万个 blob 解压出来再解析 JSON」**方向反了**：
  解析便宜到可以忽略（<1 s），成本几乎全在逐提交的树 diff。
- 窄探针因此**不是**性能手段（省不到 1 s）。留它的理由是正确性：不让 `License`/`Sourceforge`
  那两个自定义 visitor 接触不可信历史文本（见 §5 提交 0）。
- 树 diff 为什么这么贵：同一副本上原生 `git log --first-parent --raw` 只需 26.9 s(extras)、
  16.8 s(main)，hok 慢 **6.5×**；把 `core.deltaBaseCacheLimit` 置 0 后 git 掉到 111.8 s（4.2×）
  ⇒ 成本本质是 pack 里相似 tree 对象的 **delta 链解析**。
- **调 libgit2 缓存是负收益**：tree 对象默认只缓存 ≤4 KB
  （`GIT_OPT_SET_CACHE_OBJECT_LIMIT`），调到 16 MB + `GIT_OPT_SET_CACHE_MAX_SIZE`=1 GB 后
  main 从 119.7 s 变成 **229.2 s**。默认值是有道理的，这条路已排除。

## 3. 增量构建

**入口**：`hok update` 里、清单缓存刷新之后（与种子同一个入口，见 §2）。**不挂在
`manifest_cache::refresh` 内部、也不以 `changes.synced` 为驱动**：`refresh` 在
`use_sqlite_cache = false` 时直接返回，而本索引按 §1 不受该门控，挂进去会让关闭上游缓存的用户
索引完全不建。驱动改为「遍历全部已加桶」，每个桶用**索引自己的基线**（`hok_manifest_history_sync`，
不是 `hok_manifest_sync`）判断走哪条路：

- 基线 `seeded = 1` 且**在该仓库里仍可达** ⇒ 增量：从新 HEAD 沿第一父链扫到该锚点为止（含），
  锚点提交被重复处理但行是 `INSERT OR IGNORE`；随后把锚点推进到新 HEAD。
  - 锚点**不可达**（桶被重新克隆、历史被重写）⇒ 当作种子重扫，而不是从一个陌生提交往下走。
  - 锚点等于 HEAD ⇒ 无事可做，直接返回。
- 否则走种子（见 §2），`seeded = 0` 的游标作为续跑起点。
- 增量与种子共用同一条「走链 + 分块并行 diff + 按序写库」管线（§2），区别只有两点：
  增量**不轮换事务**（一次扫描通常只有几个提交）且扫到锚点就停，随后把基线推进到新 HEAD。
- **不删行**：清单从 HEAD 消失不丢历史——这正是本表与 `app` 表的语义分界。
- 非清单 JSON（`package.json` 过滤同上）跳过。
- 单桶失败只 `tracing::warn!` 并**保留旧基线**（下次 update 重试），继续处理其他桶；
  **绝不让索引失败使 `hok update` 报错**（草稿 §3.2 约束 2 的写侧一半；读侧一半见 §4）。

⚠️ **不要用 `diff_manifest_changes(anchor) + 读新侧 blob` 实现增量**（本方案原写法）：
该函数的新侧永远是 HEAD，所以「锚点→HEAD」之间的多个提交只会产出**最后那一版**，
中间版本全丢。必须逐提交游走（`HistoryRepo::walk`），与种子共用同一条原语。

**为什么基线分开**：`hok_manifest_sync` 是「`app` 表反映了哪个提交」，索引基线是「历史索引扫到哪个提交」
——两者失败模式不同（前者失败会重建、后者失败只推迟增量），绑在一起会让 `app` 的重建逻辑意外
重置索引进度。两张表都小，各记各的。

## 4. 读取与失效回落（`manifest_source` 接入点）

**唯一接入点**：`manifest_source::generate_user_manifest`（`manifest_source.rs:422`）。三段顺序对齐
上游 `generate_user_manifest`（`../Scoop/lib/manifest.ps1:306-335`）：

```
1. resolved.manifest.version() == version  → 原样返回（现有逻辑，不变）
2. 历史索引命中                            → 取回原文，跳过 autoupdate（新，本方案）
3. 无 autoupdate 段 → 报错；有 → apply_autoupdate（现有逻辑，不变）
```

第 2 步的内部顺序（对齐上游 `Find-HistoricalManifest` 的 `cache → git → miss`）：

1. **索引点查**：`SELECT blob_oid, path FROM hok_manifest_history WHERE bucket=? AND name=? AND version=?`
   —— bucket 取 `resolved.bucket`（`None` 则跳过历史链，直接进第 3 步：无桶上下文无从定位）。
2. **读 blob**：开该桶 `Repository`，`find_blob(oid)` → UTF-8 文本。
   - **blob 读不到 / 非 UTF-8 / JSON 解析失败 / `version` 与请求不符** ⇒ **失效**：删掉该行
     （`DELETE WHERE bucket,name,version`），记 `tracing::warn!`，**回落第 3 步 autoupdate**，
     不是报错——索引是派生物，坏了不许让命令失败（草稿 §3.2 约束 2）。`gc --prune` 后 OID 失效是
     这条路径的主要触发场景。
   - **桶不是 git 仓库 / 打不开** ⇒ 读路径直接回落，不删行（没读到 ≠ 行失效）。
3. **写回工作区**：`<root>/workspace/<name>.json`（与现有 autoupdate 输出同一位置，上游
   `Write-ManifestToUserCache` `manifest.ps1:345-354` 语义），`Manifest::parse_str` 后按
   `generate_user_manifest` 现有返回形状组装 `GetManifest`（`manifest_source.rs:480-487`）。
   - 命中时**不打**「generating manifest」警告（`:439-443`）——历史取回不是「生成」；
     可选 `info` 一行（上游 `:314` 有 `info "Resolving historical manifest…"`），**决策项 A**。

**不改 `get_manifest` 的解析顺序**（URL → installed → not-installed 三段，`:141-177`）：历史索引只在
「显式请求了一个与 HEAD 不同的版本」时介入，bare name / URL / 本地路径查询的语义零变化。

**`use_git_history` 开关**：上游默认 `$true`（`manifest.ps1:211`）。hok 目前把它登记为「官方键、
无实现」（`config.rs:1174` `OFFICIAL_ONLY_KEYS`）。接入后它**变成有实现的官方键**，必须从
`OFFICIAL_ONLY_KEYS` 移除、加进 `SETTINGS` 并实现读取（`config.rs:1023-1051`、`set()` 的 match 表）：
默认 `true`；置 `false` 时 §4 第 2 步整体跳过（索引照常构建——构建是无害的派生数据，门控只管读取，
与上游 `:292` 只门控 git 层一致）。**决策项 B**：确认这个键的落地。

## 5. 提交拆分

一个主题 = 一个 commit（`git-commit-practice`）；本方案 5 个主题 + 1 条前置修复 = 6 个提交，
每步可单独 revert：

| # | 提交 | 内容 | 依赖 | 状态 |
|---|---|---|---|---|
| 0 | `fix(libscoop): stop eating the following member after an unknown license/sourceforge key` | 前置修复：`License`/`Sourceforge` 两个 `visit_map` 在 `next_entry`（已消费 key+value）之后又补一次 `next_value`。未知键在**末尾**时严格 `serde_json` 先失败（`ExpectedColon`）⇒ 落到 json5 兜底 ⇒ json5 的 `Map::next_value_seed` 对空队列 `unwrap`（`json5-0.4.1/src/de.rs:469`）⇒ 两个 profile 都是 `panic = "abort"` ⇒ **整个进程 abort**；未知键在中间则先吞掉下一个成员。回归测试两条（未知键在末尾/在开头/值为非字符串） | — | ✅ 待落地 |
| 1 | `feat(libscoop): add the manifest history index tables` | 建表 DDL（`open()` 里 `CREATE TABLE IF NOT EXISTS`，含两列表基线表）+ DDL 行为测试（NOCASE 参与主键、`WITHOUT ROWID` 真生效、`seeded` 非空且有默认值） | 0 | ✅ 待落地 |
| 2 | `feat(libscoop): seed and incrementally maintain the manifest history index` | `internal/git.rs` 的 `HistoryRepo` 原语（句柄、`head`/`walk_chain`/`manifest_deltas`/`is_ancestor`/`blob_text`，OID 走 40 位 hex，`package/` 不碰 git2 类型）+ §2 种子（断点续跑，版本走窄探针，**rayon 分块并行**：走链串行、每 500 提交一块并行 diff，worker 自持 `Repository`）+ §3 增量 + §1 的桶移除清理 + 幂等/续跑/多版本/同版本覆盖/扁平与分类布局/窄探针测试 + `hok update` 接入（失败只 warn） | 1 | ✅ 待落地 |
| 3 | `feat(libscoop): resolve @version from the history index` | §4 读路径（`generate_user_manifest` 第 2 段 + blob 失效删除回落）、`config.rs` `use_git_history` 迁移（决策项 B） | 2 | ⏳ |
| 4 | `test(libscoop): pin the historical manifest resolution chain` | 端到端：夹具桶三个提交三版本 → 旧版本取回原文；失效（删 blob）回落 autoupdate；`use_git_history=false` 跳过 | 3 | ⏳ |
| 5 | `docs(agents): land the manifest history index plan` | 本方案文件 + `.agents/docs/README.md` 索引行 + 调研草稿头部状态（`active → superseded-by` 本方案）——首次入库该主题的 `.agents/**` 走独立提交（`.agents/AGENTS.md` → 提交约定 1 的同理：让主题 diff 不被文档淹没）。根 `docs/README.md` **不改**：它是面向仓库外读者的英文文档，且明确声明不作为 `.agents/**` 的索引 | 0–4 | ✅ |

> 提交 1/2 在 2026-10-06 被**重建**过一次，两点原因：
> ① 原提交 1 里三处实现缺陷（路径过滤、可达性判定的方向、基线列语义）是提交 2 才修的，
> 会让「提交 1 单独可 revert」这条保证失真；
> ② 原拆分把「第一父链游走原语」放在提交 1、把它的唯一消费者放在提交 2，于是提交 1 必然带着
> 一个**没有消费者的 crate 内 API**——`dead_code` 门禁下只能挂 `#[allow(dead_code)]`（这正是
> 前一轮出现的 7 处 allow 的成因）。现在把 DDL 单独作为提交 1（纯加法、无代码、可单独 revert），
> 原语与它的消费者同处提交 2，**两个提交都不需要任何 lint 例外**。
>
> 提交 0 是**真实数据种子**跑出来的第三个发现：scoop-app 的第 114318 个 blob
> （`bucket/pianobar_MCOfficer.json`，`"license": {"identifier": "Freeware", "url:": ...}`——
> 官方 `validator.exe` 也判它 3 处 schema 错误，该桶确实没跑校验）让种子进程 abort。
> 根因在 `manifest_parse.rs`，是**主线既有**缺陷（`git blame` → `f166d19`，2026-08-01），
> 与本次主题无关，但它是种子的前置条件：不修就永远扫不完 scoop-app。故单独成一条、排在提交 1 之前，
> 提交 1/2 通过 `rebase --onto` 重放到它之上。

收尾：一条 devlog（`docs(agents): log …` 自成一条，引用上述 sha）+ 回填 `pending-tasks.md`
（「上游 0.6.0 历史清单链」行已随落地出清；残余的已知取舍登记为 EN6）。

## 6. 验收标准

**行为**（决策门 §4 的路径全中）：

1. 夹具桶 3 个提交各改一版清单，`hok install app@v1`（v1 不在 HEAD）→ 取回的是 **v1 提交时的原文**
   （逐字节比对 blob），且**未跑 autoupdate**（无 workspace 写、无网络——断言离线可过）。
2. HEAD 清单版本即请求版本 → 与现有行为逐字节相同（第 1 段短路，不触索引）。
3. 索引行存在但 blob 被 `git gc --prune` 移除 → **不报错**，回落 autoupdate 路径，且该行被删除
   （下次查询重新走回落，不重复读坏 blob）。
   ⚠️ 测试手法：夹具是自建仓库（对象为**松散**对象），删 `.git/objects/xx/yyy…` 才真正使对象失效；
   clone 出来的打包仓库删文件无效、会**假绿**。测试里应先断言 `blob_text` 返回 `None`，
   证明「删得掉」再断言回落行为。
4. 索引表不存在 / 全空（旧库升级）→ 第 2 步静默跳过，现有行为不变。
5. `use_git_history=false` → 跳过第 2 步；置回 `true` 恢复（决策项 B 若否决则 5 作废）。
6. `cargo test -p libscoop -p hok` 全绿；`cargo clippy --workspace --all-targets -D warnings` 干净；
   改动文件逐个 `rustfmt --edition 2024 --check`（不跑整包 fmt，见 `hok-rust-gate-practice`）。

**构建侧**：

7. 种子幂等：对同一桶连跑两次，第二次零新增行、基线不动（`INSERT OR IGNORE` + 基线已是 HEAD）。
8. 增量：种子后新增一个提交（改一版），`hok update` → 索引多一行、`indexed_commit` 推进；
   两次更新之间落**多版**时每一版都要进索引（这是 §3 禁止用单次 diff 的原因）；
   中断在事务边界前重跑 → 结果与不中断一致。
9. 索引构建失败（测试注入写库错误）→ `hok update` **仍 exit 0**，只留 warn 日志。
10. 种子实测耗时记录进 devlog。**通过线：单桶 ≤ 60 s** —— ✅ 2026-10-06 达成（隔离副本、真实桶）：

    | 桶 | 提交数 | 单线程 | 分块并行 |
    |---|---|---|---|
    | hp | 180 | 0.1 s | 0.1 s |
    | scoop-private | 1,523 | 0.9 s | 0.3 s |
    | scoop-app | 2,326 | abort（修提交 0 前）/ 110.5 s | 39.6 s |
    | main | 82,028 | 119.8 s | 19.2 s |
    | versions | 106,541 | 96.9 s | 18.1 s |
    | extras | 93,602 | 192.9 s | 30.2 s |

    即 2.8–6.6×。⚠️ 实现陷阱（已踩过）：`Repository` 必须**每个 worker 线程只开一次**
    （`thread_local` 缓存句柄）。按「每块每线程 `open`」实现时 main/versions 反而退化到
    146.2 s/191.6 s——每次 open 约 28 ms × 数百块 × 参与线程数，比并行省下的还多。
11. 桶被移除 → 其索引行与基线行一并清除，且不影响其他桶。
12. 扁平桶与分类桶（清单在仓库根 / 分类子目录）都能建出索引。

## 7. 决策项（实现前要拍板）

- **A**：历史命中时打不打一行 `info`（对齐上游 `manifest.ps1:314` 的
  `Resolving historical manifest…`）？推荐：**打**（可观测，且与上游一致）。
- **B**：`use_git_history` 是否落地为真实配置键（从 `OFFICIAL_ONLY_KEYS` 迁出，默认 `true`）？
  推荐：**落地**（上游默认开，登记为「无实现」会在接入后说谎）。
- **C**：种子构建的时机——**已定**：`hok update` 后自动（本方案 §2；提交 2 已实现）。
- **D**（如 A/B 有异议再议）：`app` 表的 `use_sqlite_cache` 门控与本表无关已定（草稿 §3.2 约束 1），
  此处仅确认无异议。

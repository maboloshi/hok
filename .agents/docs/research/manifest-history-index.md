# 清单历史索引（git 库 → 索引库）调研草稿

Status: superseded-by [清单历史索引正式方案](../reports/manifest-history-index-plan.md)
Updated: 2026-10-06

> **这是什么**：为「hok 能在 bucket HEAD 已无该版本时，按版本号取回**原文清单**」这条能力做的调研与决策记录。
> 起因是上游 Scoop 0.6.0 的 `63cb8f84`（历史清单链），评估见
> [`upstream-0.6.0-impact-2026-10.md`](../active/audit/upstream-0.6.0-impact-2026-10.md) §3.1–3.2。
> **状态：只有结论与方案，没有实现**。新对话按 `investigate-first` 出正式方案（提交拆分 + 验收标准）后再动代码。
> **口径**：官方侧引用 `../Scoop`（v0.6.0）file:line；hok 侧引用本仓 file:line。数据为 2026-10-03 本机实测。

## 1. 要解决的问题（与上游三级的差别）

`hok install app@<旧版本>` **本来就支持**：版本与 HEAD 不符时由 autoupdate 重建
（`crates/libscoop/src/package/manifest_source.rs:422-479` → `package/checkver.rs:1035-1131`）。
缺口只有一处：**重建不出时没有退路**——模板改过结构 / 产物改过名 / URL 规则变过 / manifest 无 `autoupdate` 段。

上游为此加了「按版本号找**原文**」的三级链（`../Scoop/lib/manifest.ps1:306-335`）：`sqlite 缓存 → git 历史 → autoupdate`。
本方案对应其中的第二级，并把第一级改写为「同一张库里的轻量索引」而不是「再存一份清单文本」。

## 2. 关键事实（实测，2026-10-03）

| 项 | main | extras | 合计 |
|---|---|---|---|
| 清单数 / 提交数 | 1662 / 81855 | 2404 / 93471 | 4066 / 175k |
| 全历史**去重后**版本数 | 74092 | 86674 | **≈160,800** |
| 单版本均值（近 300 提交取样） | 1.47 KB | 1.58 KB | ≈1.5 KB |
| 已本地克隆、非 shallow | 是 | 是 | 前提条件 |

耗时实测（同一台机器，未预热）：

| 路径 | 实测 |
|---|---|
| `git show HEAD:<rel>`（上游快路径） | 192 ms |
| `log --follow -G`（正则 pickaxe，上游主探针） | 976 ms |
| `log --follow -S`（字面兜底） | 3639 ms |
| `git log --grep '^7zip: Update to version …'`（不用） | 1229 ms（比 pickaxe 还慢） |
| 走全历史收集版本号（PowerShell 侧） | main 52.7 s + extras 87.2 s |
| git2 单文件父链游走（`last_commit_for_file` 注释留存值） | 0.63 s / 文件 |

引擎版本（评估过的"是不是工具旧"）：hok 发布形态是 **winsqlite3**（本机 `3.51.1`），cargo 的 `bundled`
（`libsqlite3-sys 0.28`）是 **3.45.0**，`sqlite3` CLI 3.42.0、python 内置 3.50.4。
⇒ **耗时在 git 遍历，不在 SQLite**；索引落地建议走 `bundled`（版本已知、随二进制、不随用户 Windows 漂移）。

## 3. 已定的结论

### 3.1 存什么：只存「版本 → blob OID」，不存清单文本（方案 B）

| | 全量存清单文本（A） | **索引库（B）** |
|---|---|---|
| 存什么 | 16 万份原文 | `(bucket, app, version) → blob_oid, path, committed_at` |
| 存储 | ≈300–400 MB（SQLite 不压缩，同一 blob 被多提交携带还会重复） | **≈20–30 MB**（每行 ~100 B 估算） |
| 取一个版本 | 点查 ~0 ms | 点查 + 读一个 1.5 KB blob（毫秒级） |
| 首次构建 | 遍历一次 + 插入 16 万条文本，约 10–20 min | 遍历一次 + 每变更读一个 blob |

选 B 的理由：git 本来就是内容的存储，索引只需回答"哪个版本在哪"。**否掉**"复用 `app` 表保留多版本行"
——`app` 的 schema 锁定为必须存整份 `manifest` 文本，那等于选了 A。

### 3.2 放哪里：同库、新表（`scoop.db`）

hok 已经在 `{SCOOP}/scoop.db` 里加了 hok 专属表 `hok_manifest_sync`（`package/manifest_cache.rs:48-79`），
所以"在上游文件里加 `hok_` 前缀表"是既有先例。选同库的决定性理由是**状态只有一份**：

- 增量维护依赖"每个桶上次同步到哪个提交"，而这个基线已在同库（`stale_buckets()`，`:136-159`）；
  拆成独立文件会让索引库与基线各记一份进度，跨文件失败即漂移——派生数据最不该有的性质。

三条配套约束（缺一就不该同库）：

1. **新表不跟 `use_sqlite_cache` 门控**。上游的 `app` 缓存受该开关门控（`refresh()`/`query()`），
   但"按版本找原文"是对标能力，不该因为没开上游缓存而失效。
2. **索引是派生物，必须能重建**。桶被外部 `gc --prune` 后 OID 失效 ⇒ 读不到就回落 git 搜索 / autoupdate，
   索引条目失效或整表重扫；**绝不允许索引坏了导致命令报错**。
3. **`app` 表的两条 `DELETE … version <> ?`（`:325`、`:400`、`:407`）不得顺手套到新表**。
   那正是 hok 现在主动丢弃历史的机制：`app` 继续做"当前版本加速器"，新表做"历史索引"，语义不同。

### 3.3 怎么建：git2 驱动，不需要 pickaxe

hok 已有 `diff_manifest_changes()`（`internal/git.rs:454-484`，git2 `diff_tree_to_tree`），
`internal/git.rs:414-421` 的 `blob_at` 也能取任意提交某路径的 blob——都不依赖 git on PATH。

- **种子**：遍历历史里相邻提交的 diff，每个变更清单读出新版本的 blob OID。
  **不要**对每个 app 逐条回溯（那是 O(175k×每 app)，不可行）。
- **增量**：`hok update` 后只用 `UpdateChanges` 里那几个文件各做一次父链游走
  （git2 原生 `log -n 1 -- <path>`，即 `last_commit_for_file` 那套，0.63 s/文件）；
  涉及文件通常个位数到几十，≈0 成本。

### 3.4 能不能并行：只有「提取/解析」那一段

| 阶段 | 并行度 |
|---|---|
| ① 走历史收集 OID（父链顺序游走） | **不可并行**（链式依赖）；靠算法而非线程 |
| ② 读 blob、解析版本、写库 | **可并行**（互相独立，纯 I/O + 解析） |

可按 app 切分并行（4000 个天然任务单元），每个 worker **自开 `Repository` 句柄**——git2 的 `Repository`
不是 `Sync`，不能跨线程共享（`manifest_cache` 已有 rayon + 每线程自开的先例可照抄）。
三个坑：**只有并行读、没有并行写**（SQLite 单连接单写入者 + 批量事务，事务拆分对总耗时的影响比并行还大）；
重跑必须幂等（`app` 已有版本、索引已有 `(bucket,name,version)` 直接跳过）；**别一上来就并行**——
绝大多数构建是增量的，并行只压缩一次性的冷启动。

另一个「更快但换原则」的选项：直接用 `git.exe` 跑一次 `git log --raw --all` 把整个历史一次吐出再并行解析。
原生 git 的 diff 机器比 git2 走树快得多，还能拿到 `--follow` 重命名；代价是破 hok「桶操作全走 libgit2、
不依赖 git on PATH」的原则（`internal/git.rs:338-341`）。Scoop 的 `supporting/git` 自带 git，所以**自包含不破，破的是这条一致性原则**——留作决策项。

## 4. 新对话要处理的事

1. 正式方案：索引表 schema（建议 `hok_manifest_history(bucket, name, version, blob_oid, path, committed_at)`，
   主键 `(bucket, name, version)`）、种子与增量两条路径、失效回落策略、`manifest_source` 接入点、提交拆分、验收标准。
2. **重命名**：`diff_manifest_changes` 明确不做重命名检测（文件注释），清单改名会变成"旧路径 deleted + 新路径 added"，
   历史版本会挂在旧名下 ⇒ 索引键要用 `(bucket, app)` 并保留路径回指，或加 `find_similar`。
3. **是否接入 `@version` 解析**：只做"可查询"还是真正接入 `manifest_source`（后者才改行为）。
4. **引擎形态**：索引走 `bundled`（见 §2）；SQLite 磁盘格式向下兼容，只要不用新语法即可。
5. 上游 `use_git_history` 键的语义是否要在 hok 侧对应一个开关（hok 现将其登记为"官方键、无实现"，`config.rs:1174`）。

## 5. 复跑入口

```powershell
# 版本数：distinct blob per manifest（逐行取第 5 列，去重）
git -C <bucket> log --format='%H' --raw --no-abbrev -r --no-renames -- 'bucket/*.json'

# 单版本体积：抽样 blob 取 size
git -C <bucket> cat-file -s <oid>

# 上游缓存里现存多少行（只读，不动库）
python -c "import sqlite3;c=sqlite3.connect('file:<SCOOP>/scoop.db?mode=ro',uri=True);print(c.execute('select count(*) from app').fetchone())"
```

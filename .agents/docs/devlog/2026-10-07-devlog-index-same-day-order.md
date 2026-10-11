# devlog 索引同日顺序改按入库时间

> 2026-10-07 ｜ 提交 `02222f9` ｜ worktree `D:\App\github项目\Scoop\hok`（主树，非 worktree）｜ 分支 `refactor/edition-2024`

**现象（用户报"索引存在插错位置"）**：`.agents/docs/devlog/README.md` 里最新那条被插到旧条目下面——
`2026-10-07-manifest-history-index-spinner`（`7441be7`，10:24 落地）排在 `2026-10-07-manifest-seed-tree-cost`（`6a93ecc`，00:08）之下；
`2026-09-13` 批次更明显，读成 S9、S8、S4、S2、S14、S11、S10、S1+S13……

**根因**：生成器只按**完整文件名**降序，文件名精确到「日」，同一天就退化成 slug 字母序；
R7 把这条实现写成「按文件名排序即时间序」，于是没人再怀疑。三条门禁都查不出——`--check` 只比对脚本产物，
`doc-links` / `docs-index-check` 只管链接与覆盖，**顺序错不属于任何门禁的范畴**。

**改法**：同日 tiebreak 换成该条目**入库提交**的 committer date——一次
`git log --diff-filter=A --pretty=format:%ct --name-only -- .agents/docs/devlog` 建 `文件名 → 时间` 表；
取「新增」而非「最后一次触碰」，是为了让事后改错字不会把老条目顶到当日最前。
未提交的新条目视为最新（否则「写完日志 → 先跑索引 → 再提交」会把它排到当日末尾）；
git 取不到（非仓库 / 无 git / 沙箱挡死）时打警告并退回旧的 slug 序，不静默改语义。

**坑（沙箱）**：Node `spawnSync` 的默认管道 stdio 在 DSH 沙箱下会 EPERM，故 stdout 直接重定向到临时文件再读回，
让门禁脚本在受限模式下也能跑。

**验证**：`devlog-index.mjs --check` / `doc-links.mjs` / `docs-index-check.mjs` 全部 exit 0；
重生成后 14 行位置变化，10-07 两条恢复「后落地在上」；其余变动集中在 2026-09-26 一次性迁移写入的旧条目
（同一次迁移入库 ⇒ 同日时间相同 ⇒ 按 slug 兜底，与旧行为一致）。R7 措辞同步改为「日期降序 + 同日按入库时间降序」。

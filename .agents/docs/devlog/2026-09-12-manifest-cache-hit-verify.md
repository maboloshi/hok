# 清单缓存：命中即校验 + 桶更新失败可见性

> 2026-09-12 ｜ 提交 `c2e06df` `5515989` `3d4969f`

**背景**：上一条留下三个开放项，本次只做 ① 命中即校验、③ 非 TTY 桶 pull 失败可见性（② 见文末）。按 [[hok-investigate-first]]：先出报告与"模拟输出"，用户逐项拍板后才动手。

**① 命中即校验**（`c2e06df`）
- 现象：缓存行落后于文件时，`info` / `search -e` / `cat` 一律拿缓存作答；上轮的 stale WARN 只比 git HEAD，看不见"工作树脏改动"和"无 git 的目录桶"。实测：文件 2.0 / 缓存 1.0 → `info foo` 报 1.0 且无提示。
- 做法：`query_synced_cached` 对 **Explicit 命中**（info / search -e / cat / home / depends / virustotal）多读一次文件、比**版本**；不一致 ⇒ 整份换文件版（其余字段一并刷新）+ `WARN …缓存 X，文件 Y（已改用文件值）`；文件不在 ⇒ 丢弃该行（幻影）+ `WARN …已不存在（已忽略缓存行）`。通配/正则查询不校验（`list --available`、`search <正则>` 仍是纯缓存）。WARN 按 `bucket/name` 每进程去抖（`depends` 会对同一包查两次）。
- 成本：每条命中 3 次 stat + 1 次读解析，不随桶规模放大。
- 实测：`info foo` → 2.0 + WARN；`search -e foo` → 2.0 + WARN；`search foo`（正则）→ 1.0 无 WARN；删文件后 → WARN + `ERROR 找不到匹配`（exit 1）。

**③ 非 TTY 桶更新可见性**（`5515989`）
- 根因：`update_buckets` 无条件用 indicatif `RowUI`，而 `indicatif/draw_target.rs:167` 在非终端直接 `return None` ⇒ 管道/CI 里连 `finish_with_message` 都不画，pull 失败零输出、exit 仍 0。
- 做法：`cui::BucketRows`（终端 = 原 spinner 行；管道 = 只打"失败/跳过"行）；`BucketUpdateState` 增 `Skipped`，无 `.git` 的桶发跳过事件（对齐官方 `'x' is not a git repository. Skipped.`），有 `.git` 但无 origin 的桶**记为失败**（官方会真 `git pull` 报 "No configured remote"，hok 过去两者都不出声）；末尾 `WARN N 个仓库更新失败：…；运行 'hok update --force' 重试。`（stderr），退出码保持 0（对齐官方不检查 pull 返回值）。
- 顺带：libgit2 报错自带换行（`…\n; class=Os (2)`）会把单行报告撑成两行 ⇒ `one_line()` 归一（TTY 行同样受益）。
- 文案收尾（`3d4969f`）：`remote_url()` 返回 `None` 既可能是"没有 origin"，也可能是"git 元数据读不了"，原文案 `未配置远程仓库（origin）` 断言了它并不知道的原因 ⇒ 改为 `无法确定远程仓库 origin（缺失或 git 元数据损坏）`；同类的"无 remote 的 git 桶"在官方是交给 `git pull` 报 `There is no tracking information for the current branch.`（stderr、exit 1 被忽略），hok 这边不尝试 pull 而是直接给出人话结论，结果语义一致（没更新 + 可见 + exit 0）。
- 实测（三桶：pull 失败 / 无 .git / 无 origin；中英双语文案已核对）：

```
:: 正在更新仓库
  'localb' 不是 git 仓库。已跳过。
  ✗ orphan 更新失败：无法确定远程仓库 origin（缺失或 git 元数据损坏）
  ✗ bad 更新失败：failed to send request: 无法解析服务器的名称或地址 ; class=Os (2)
  正在刷新清单缓存...
  ✓ 清单缓存已刷新（重建仓库：localb, orphan, bad）。
WARN 2 个仓库更新失败：orphan, bad；运行 'hok update --force' 重试。
```

**未做：② 本地桶 mtime/size 行级记账** —— 唯一真收益场景是无 git 的本地目录桶；本机 `buckets/` 四个桶全是 git 仓库，收益为 0，而 ① 已从读侧兜住正确性。实测参照：3000 清单的本地桶每次 update 全量重扫 **1.31s** 并打 `重建仓库：localbig`；真要做时按"目录 stat 差分 → 只重读变化文件"落地（读路径的漂移检测还得每次查询走一遍全桶 stat，比 ① 贵）。

**未修（本轮未选）**：无 remote 的 git 桶每次 `info` 仍打 `WARN …1 个仓库未同步`（`stale_buckets` 只跳过无 HEAD，而 `changes.synced` 只覆盖有 remote 的桶）——本轮只让它"更新时不静默"，该假告警仍在。

---

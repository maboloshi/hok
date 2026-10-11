# `info` 的 git 历史查询重写：树条目跟随，不 spawn 外部 git

> 2026-09-12 ｜ 1.69s → 0.63s

**问题**：`hok info <pkg>`（含 `-v`）要显示 `Updated at` / `Updated by`，走 `internal::git::last_commit_for_file`（对齐 `scoop-info.ps1:123-134`）。旧实现用 `revwalk` 遍历全历史、每个 commit 跑一次 `diff_tree_to_tree(pathspec)`，在 main bucket 上 `hok info git` 实测 **1.69s**（`-v` 1.97s）。

**先排除误判（避免修错对象）**
- `-v` 自身增量极小：73 个已安装包全扫，`info -v` vs `info` 差 ≤0.03s（warm）。`-v` 独有的重活只有两处，均未改动：未安装包的 `probe_download_size`（`info.rs:709`，30s/URL 顺序 HEAD，无并发）、已安装包的递归体积统计（`measure_installed_size`）。
- 慢的是**两个模式共用**的 git 历史查询，所以它一修，「`-v` 慢」的观感一并消失——这也解释了「不用 `-v` 也秒返回」。

**方案（libgit2 only）**
1. 树条目比较替代 tree-diff：blob 与父相同 → 沿该父继续（复刻 `git log -- <path>` 的 history simplification），根提交只在路径已存在时计数；只比第一父会为 merge 报错作者。
2. blob **跨迭代携带**：线性历史里父的 blob 就是下一轮的当前 blob，树查找直接减半。
3. `tree_id` 相同即 treesame → 连对象库都不读。
4. `MAX_COMMITS_SCANNED` 5000 → 200000（现在是单链跟随，不再是全量扫描，仅作病态历史兜底）。

**实测（release，`hok info git`，5 次取最小）**

| 实现 | `info` | `info -v` |
|---|---|---|
| 旧：revwalk + 每 commit diff | 1.687s | 1.966s |
| 新：树条目比较（尚未携带 blob） | 1.037s | 1.327s |
| **新：+ 携带 blob + tree_id 快判（本次）** | **0.631s** | **0.907s** |
| 对照：外部 `git log` 优先（已放弃） | 0.512s | 0.683s |

**决策**：**不引入外部 git**。外部 git 只快约 0.12s，代价却是 PATH 依赖、用户配置（如 `log.showSignature`）可污染输出、作者名编码假设；而 hok 全链路本就走 libgit2（bucket 也是 libgit2 clone）。「是否全面改用外部 git / 放弃 libgit2」留作未来专项评估，上表即其对照输入。

**验证**：`cargo test -p libscoop --lib internal::git` 10 passed（含 treesame merge、未跟踪路径、仓库外路径）；`rustfmt --edition 2024 --check` 干净；clippy 本文件 0 警告；`cargo test --workspace` 除 2 个环境性用例外全绿——`permission::tests::grant_users_write_inherit_*` 在 stash 掉本次改动后**同样失败**（本沙箱对临时目录 grant Users 写权限报 error 5），非本次回归。

**坑/纪律**
1. 定位性能问题先量「两个模式的差」与「两个模式共用的部分」：只盯 `-v` 独有路径会完全修不到共用慢点（本次差点）。
2. 本沙箱 schannel 不可用（curl 报 `SEC_E_NO_CREDENTIALS`），而 hok 的 reqwest 用 native-tls = schannel → 会话内任何 HTTPS 都秒失败，「未安装包 HEAD 探测」这类网络路径在此测不出来；node 用自带 OpenSSL 正常（github 200 / 707ms），可作链路对照。
3. 性能数据必须 release + 区分「构建后首次冷跑」（曾测得 2.98s）与热跑，否则会把 Defender/冷缓存当成代码问题。

---

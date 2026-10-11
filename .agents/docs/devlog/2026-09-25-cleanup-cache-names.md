# `cleanup -k` 缓存面三修：不可见的名字 / 坏安装 / 零输出

> 2026-09-25 ｜ 2 提交 `ff05b80` `cb75ba4`，worktree `.worktrees/s-cleanup-cache`，分支 `fix/cleanup-cache-names`

**起因**：用户实机反馈「`hok cleanup * -k` 貌似没同步删缓存」。先做对照实验：把真机 `D:\App\Scoop` 的 74 个 app + 25 个缓存文件按**结构**（目录 + junction + install/manifest.json + 缓存文件名）镜像到临时根，用真机同款二进制跑 `cleanup * -k` —— 旧版本目录与过期缓存**都**被正确删除；`-k *` / `* -k` 两种参数顺序、经 `shims\hok.exe` 调用、persist junction 三类用例全部通过。⇒ 主路径无 bug，但沿"漏删"线索找到三个真缺口（用户拍板三项全修）。

**改动**（`ff05b80` cache 模块 / `cb75ba4` cleanup 命令）：
- ① `REGEX_CACHE_FILE` 的 name/version 字符类是 `[0-9a-zA-Z._-]+` ⇒ 版本号含 `+`/`_`（`1.9.11_2007`、`1.4.3-canary.1+29d9638da`；buckets 里 250+ 个 manifest 如此，CJK 包名同理）的缓存文件**对 hok 完全不可见**：`hok cache` 不列（总账也少算）、`cleanup -k` 不删、`cache rm <app>` 不删，只有 `cache rm *`（直接遍历目录的分支）能清掉。改为 `^[^#]+#[^#]+#` —— 对齐官方 `Remove-Item "$cachedir\$app#*"` 的 glob 语义，同时用整名锚定收紧。**这条正是 `pending-tasks` 里挂账的「CJK 包名缓存文件被拒」**。
- ② `cleanup -k` 对缓存**零输出**（官方也静默）⇒「只删了缓存」的运行与「什么都没做」在屏幕上无法区分。`remove_outdated` / 新 `remove_all_for_app` 改为返回被删条目，`cleanup()` 返回 `CleanupOutcome { apps, cache_removed }`，CLI 收尾打一行 `cmd.cleanup_cache_removed`（`过期下载缓存已删除：共 N 个文件，X`）—— **零删除也打**，于是"静默"只可能意味着没加 `-k`（这是 hok 扩展，官方无此行，已登记）。
- ③ current 版本解析不出的 app 原先**整包跳过**（缓存也不删）。官方此时仍执行同一行 `Remove-Item`，只是 `-Exclude "$app#$null#*"` 匹配不到任何东西 ⇒ 该 app 缓存**全删**（`scoop-cleanup.ps1:32-33`）。hok 对齐缓存部分；**版本目录仍不动**（"判不出当前版本就不删目录"是 hok 有意偏离，审计报告已登记）。

**验证**：worktree `cargo test -p libscoop` 786 passed / 0 failed、`cargo test -p hok` 全绿、`cargo clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings` 干净、改动文件 `rustfmt --edition 2024 --check` 干净；隔离 `SCOOP` 根端到端（按真机布局造的夹具）：`hok cache` 由「共 2 个文件」变为正确列出 6 个（含 `_`/`+` 两个拼写）、`cleanup * -k` 删 3 个过期条目并打汇总行、`broken` app 的缓存被清、当前版本缓存与 aria2 sidecar 保留、二次运行打「共 0 个文件」。另在**真机** `D:\App\Scoop` 上跑过一次 `cleanup * -k`（用户原本要执行的命令）：删掉 16 个旧版本目录 + 6 个过期缓存（≈156 MiB），当前版本缓存（19 个文件 2 GB）按设计保留。落地 `refactor/edition-2024`（`cb75ba4`，脚本漂移校验：2 提交 / 主题 / 6 文件集守恒 + 树逐字节相同，无 merge commit），落地树门禁复跑全绿。

**坑/教训**：① **"看起来没发生"要先分清是没删还是没说**——主路径实测正确，用户观感来自 `-k` 完全没有输出、且留下的都是当前版本缓存（设计保留）；修法是给可见性，不是加开关。② **过窄的字符类会把文件变成"不存在"**：同一个正则既是"这是不是一个缓存文件"的判据，又被列表/删除/`cleanup -k` 三处共用 ⇒ 一个字符类让这些文件在 hok 眼里彻底消失（挂账已久，只能靠 `cache rm *` 绕）。放宽一处要收紧另一处（加 `^…$`）以补偿。③ **broken app 的语义分两层**：上游 `$current_version = $null` 让 `-Exclude` 失效（缓存全删）**且** `Where-Object { $current_version -ne $_ }` 恒真（版本目录也全删）；hok 只对齐前者——删缓存可逆（重下），删版本目录不可逆。④ 沙箱里 `cargo test -p libscoop <filter>` 会踩既有 flaky 用例 `manifest_cache::tests::serving_a_stale_cache_warns_once`（进程级 warn-once + 并行）：**在 clean HEAD 上用同一 filter 复现同样失败**才判定与本次无关，验收一律以全量 `cargo test -p libscoop` 为准。

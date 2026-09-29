# hok 未完成任务总表（2026-09-13 整理）

> **这是什么**：把散落在 `reports/command-deviation-fix-plan-2026-09.md` §8、`.agents/skills/*`、
> `.agents/docs/reports/*`、代码内 TODO 里的**未完成项**汇到一张表。
> **权威性**：本文件是**索引**；每条的主题细节、决策记录、验收仍以「来源」列指的文件为准。
> 任何一项做完后，**同时**回填本表与该来源；**技能只装可复用知识，落地记录进 devlog**（判据见
> `skills/AGENTS.md`，触发与格式见 [`docs/AGENTS.md` R7](../../AGENTS.md)）。
> **基线**：主线分支名取自**机器本地** git 配置（键名、本机当前值与重设办法见
> [`hok-worktree-parallel`](../../../skills/hok-worktree-parallel.md) → 本仓事实）；
> `main` 是最终目标线，本开发线的成果**尚未并入**（合并 / push / 版本号是独立主题）。
> **派活时以实测 tip 为准**：`git rev-parse --short $(git config worktree-merge.mainbranch)`；下表里的数字都是当时的快照。
> **开工纪律**：先 `investigate-first`（出报告 + 选项 + 等确认），再 `worktree-parallel`
> （一会话一 worktree、`--ff-only` 线性落地）→ `hok-rust-gate-practice` → `proactive-review`。

---

## 1. 高优先：明确缺陷 / 已有复现（建议先做）

| # | 主题 | 状态 | 证据 | 建议 |
|---|---|---|---|---|
| **H1** | **`hok checkver` 挂起** | ✅ **已修复并落地**（= §8 R1，2026-09-13） | 根因（S1 定位 + 父会话独立核验）：`package/checkver.rs:383` 的 `.buffer_unordered(pending.len())` 在 `pending` 为空时容量 **0**；vendored `futures-util-0.3.34/.../buffer_unordered.rs:62-86` 在 `max=0` 时永不 poll 源流 ⇒ 返回 `Pending` 且无 waker ⇒ `collect().await` **永久挂起**。**触发面比原判断宽**：空目录 / 全部无 checkver / app glob 不匹配 / **任何 `--version`**（`:241-269` 全 `continue`，实测先打印版本行再挂死） | 已按用户 4 项决策实现（A2 `.max(1)` + `--version` 回归 + TODO 清理 + 同类站点加固），5 提交落地主线 `43795484`；改动面 `checkver.rs`/`download.rs`/`download_fragmented.rs` |
| **H2** | **缺环境变量 panic** | ✅ **已修复并落地**（= §8 R2，2026-09-13） | 2026-09-13 复现：清空 `USERPROFILE`/`HOME` 后跑 `hok list` → `panicked at crates\libscoop\src\internal\fs.rs:63:30`（`Option::unwrap()` on `None`），debug 回溯链：`fs::write_json:63` ← `config::write:790` ← `config::init_default:780` ← `Session::new:182`。**归因更正（S2 实测，2026-09-13）**：被写的是 **`hok_config_path()`**（`config.rs:889-891`，经 `Config::default` 的 `path:` 字段 `config.rs:758`），**不是** `scoop_config_path()`——后者只喂 `ConfigBuilder::scoop_path`，是**只读**回落（`config.rs:47-49` 注释 "Never written to"）。两者都因 `home_join` 为空而为空路径，但**只有 `hok_config_path` 进入写链**；`hok_config_path` 走 `home_dir()`（仅 `USERPROFILE`/`HOME`），**不走** `config_dir()`（APPDATA）⇒ 这就是"APPDATA 存在照样炸"的原因（实测：保留 `HOME`、去掉 `APPDATA` 不炸）。父会话的"反直觉点"**被证实**：读链确实回落默认，但"回落成默认"这一步本身要写盘 | **两层修（用户已拍板并落地）**：① `fs.rs:63` 与同型 `fs.rs:115`（`symlink_dir` 的 `lnk.parent().unwrap()`）改返回 `Error::Io(InvalidInput, …)` 而非 panic（`Error` 已有 `Io(#[from] io::Error)`，i18n 键 `error.io` 已存在，无需新增键）；② `config::init_default`（`config.rs:780`）对**空 `config.path`** 跳过写入（它本就是 `let _ = write(...)` best-effort）。**反对**把 `scoop_config_path` 改回退 `%APPDATA%`：修不到真正的写入源，且会改变 Windows 配置落点（语义级变更）。**另发现同型可达点**：`config.rs:110` 迁移写 `write_json(&self.path)`。**影响面**：panic 发生在 `Session::new` ⇒ **所有命令**都炸（`hok --version` 不炸）|
| **H3** | **清单解析失败静默消失**：解析失败的清单在查询/安装路径变成「找不到软件包」 | **未开工（设计题，必须先过决策门）**（= §8 R3） | `package/query.rs` 的 `filter_map`（现约 60 / 270 / 275 / 379 行；报告记的是 120/285） | P0-4（`autoupdate.hash.mode: github`）难定位的**根因**。要定「搜索/安装遇到不可解析清单：报错 / 警告 / 计数」，会改单入口的错误传播策略，**风险最高，单开** |
| **H4** | **`ci-auto-pr` 对齐**（S11 余项） | **未开工**（= §8 R4） | 用户在 CI 里用它更新清单；审计 §7.7 差异点：变更判定（字节快照 vs `hub diff`） | **要做**。注意它**写盘并提交**，受 `-d` 写盘护栏约束——当前 `-d` 默认 `bucket`，应改为「写盘时要求显式目标」 |

---

## 2. 已立项余项（fix plan §8 剩余）

| # | 主题 | 状态 | 证据 | 建议 |
|---|---|---|---|---|
| **R5** | **恢复 `-c/--cascade`**：`SyncOption::Cascade`/`NoDependentCheck` **无任何构造者** ⇒ 级联卸载在生产**不可达**，`resolve_cascade` 只被 4 个单测调用 | **未开工** | 已核实：`package/resolve.rs:219` 定义枚举，`sync_remove.rs:33/99` 读取，全仓无 `SyncOption::Cascade` 构造点。成因：`5f2532e`（eventloop 统一）连同 `options.push(SyncOption::Cascade)` 一起删掉，提交信息未提 | **单开**（属新功能 + 有意偏离官方）。恢复后 held 依赖按新政策**恒报错**（要求先 `hok unhold`，无 bypass）。**别顺手删这条已测通的逻辑** |
| **R6** | **`PRIVATE_HOSTS` 注入语义 = 替换（官方）而 hok 是 append**：同名头（Referer / User-Agent / Authorization）会发两条 | **未开工** | 官方 `$wreq.Headers[$key] = $value` 是替换；`internal/network.rs:188-192` 用 reqwest `.header()`（append）。reqwest 已有替换入口 `RequestBuilder::headers(map)` → `util::replace_headers` | 改法 ≈ 12 行。三个坑：① 别改成**静默跳过**（无效头名/值现在会让请求报错，保持 loud）；② 官方 checkver 自己也会出重复头（`checkver.ps1:240-246`），**别照抄这个 bug**；③ 官方只在 download/checkurls/checkver 注入，hok 全 profile 注入是既有有意偏离，保持 |
| — | **`HeaderProfile::CheckUrls` 不发 Referer**（追加待办） | 未开工（随 checkver/checkurls 专题） | 现状按官方 `bin/checkurls.ps1:60` 发 ⇒ 反盗链 CDN（腾讯/阿里云同款）上出现**假坏链** | 跟随下载路径的既有决定；`Checkver`/`Autoupdate` 保持官方口径不动 |
| — | `checkurls`（S11 余项） | **暂缓**（用户用得少） | 只读扫描，审计差异多为输出/边界 | 与上一条合并成一个专题 |
| — | `virustotal`（S11 余项，含退出位掩码） | **不做**（用户基本不用） | 审计 §7.5 | 登记为不修 |
| — | **`cleanup` 失败汇总文案与实现语义冲突**（「重启后可重试」暗示存在延迟删除，实际没有）→ **已并入 cleanup 并行化专题**（规格定稿 `.agents/docs/active/spec/cleanup-parallel-spec.md`，K1=C1 / K2=0 已拍板）：文案随该专题一起改，不再单列 | ✅ **已落地**（2026-09-20，随 cleanup 并行化专题；提交 `3bca476` `54a87d0` `2ad6e9d` `e8c09f1`，主线 tip `e8c09f1`） | 文案：`locales/zh.yml:164`「已清理 %{count} 个旧版本，%{failed} 个失败（重启后可重试）」/ `locales/en.yml:164`「… (retry after restart)」；全仓**无** `MoveFileEx(..., MOVEFILE_DELAY_UNTIL_REBOOT)` ⇒ 不存在任何重启后自动删除。2026-09-20 实测（release，旧版本目录 200 文件 + 1 占用）：`ERROR 无法移除 app1 v0.9.0：另一个程序正在使用此文件…(os error 32)` + 汇总行同上 + **exit 0**；目录被**部分删除**（剩 1 个锁定文件），释放占用后再跑一次即清干净。另测：非提升进程 `MOVEFILE_DELAY_UNTIL_REBOOT` = `err 5`（需管理员），真做延迟删除也不可行 | **改文案**（如「关闭占用程序后重试」）：多数占用（程序仍在运行）关掉程序即可，不必重启；「真做延迟删除」应否决（需管理员 + 空间延后释放）。证据与复跑脚本见 `.agents/docs/research/cleanup-delete-perf.md` §4 |

---

## 3. 对标差距里的暂缓 / 有意未做

| 项 | 状态 | 说明 / 来源 |
|---|---|---|
| **P1-9 `update <app>` 先同步 buckets** | ⏸ 用户暂缓 | 官方 `Sync-Bucket` 在 app 更新前；`scoop-gap-analysis-2026-08.md` §P1-9 |
| `aria2-min-split-size` 20M→5M | ⏸ 暂缓（S4） | 见 `reports/s4-config-investigation-2026-09.md` §5 |
| `proxy` 三态（`none` 存字面值 + 直连；`default`/`currentuser` 不打挂下载；未设≠系统代理） | ⏸ 暂缓（S4） | 同上 |
| 官方独有 config 键（15 个，**真缺口 3**：`use_external_7zip` / `show_manifest` / `autostash_on_conflict`） | 未实现，已分流为 `ConfigKeyUnsupported` | 其余为架构差异或 N/A；修入口 = `ConfigInner` 加字段 + `set()` match + `list_all`，单文件闭环 |
| **S8 多 bucket 命中保留 hok 交互选择** | ⏸ 暂缓/待定（非"已定偏离"） | 已知限制：**非 tty（脚本/CI）下交互失败 exit 1** ⇒ 有重名包时该命令不可用。`manifest_source::get_manifest` |
| S14 有意偏离/遗留 | 登记不修 | clone 失败仍打 libgit2 原文（官方 `Failed to clone …` 未对齐）；`bucket list -k` / `bucket rm` 多名称；`shim` 无 `alter` |
| `status -l/--local` + Scoop/bucket 新鲜度检查 | ⏸ 暂缓（属新功能） | 删冷却后 `last_update` 仍在写，其 reader `parse_last_update` 现为 dead code（`#[allow(dead_code)]`）就是为它留的 |
| checkver 有意未做 | 登记 | xpath 命名空间、`checkver.url` 变量替换、输出文案；Digest base64 解码失败回落 `.meta4`（有意偏离） |
| 其他登记有意偏离（`list` 升级箭头/计数页脚/多查询 OR、`search` 输出形态、`bucket known` 裸名称…） | 登记不改 | 除非用户明确要求，见 `reports/command-deviation-audit-2026-09.md` §11 状态表 |

---

## 4. 架构 / 重构 backlog

| 项 | 状态 | 说明 / 来源 |
|---|---|---|
| **framework P1-5：`config.rs` 混入命令操作** | **仍未做** | `list_all_rows`（`config.rs:1189`）、`edit`（`:1387`）仍在 lib 层；devlog 2026-09-12 仍记为"分层违反"。命令逻辑应上移 CLI |
| ~~P1-4 可见性落差~~ / ~~P1-6 checkver Args 镜像~~ | **已解决（更正）** | `lib.rs:66` 已是 `mod internal`（私有）+ `pub use` 门面；checkver 侧改为 `CheckverOptions`（`package/checkver.rs:69`，注释明示"deliberately not a clap struct"） |
| `innospect` 上游化（12.8k 行 fork ≈ 生产代码 21%） | ✅ 已完成（2026-09-19，`f5922b3`） | 上游 0.1.4 收录 PR #3（并额外改成从 Inno 属性字节读 LZMA2 字典）⇒ registry `innospect = "0.1.4"`、删除 `crates/innospect`；A/B 解码真实 Inno 安装器（zed 6.4.3）逐字节一致，见 devlog 2026-09-19 |
| 4.1 超长函数拆分（36 个 ≥120 行；`checkver.rs::execute` + `apply_autoupdate` 占 713 行收益最高） | ⏸ 未动 | 需先有明确问题陈述；`commit_one_install`/`commit_one_remove` 结构对称，**先核对是否真能抽公共管线**（别用开关掩盖 install/remove 语义差异） |
| 4.2 profile 评估（`opt-level = "s"` / `lto = "thin"` / `[profile.dev-fast]`） | ✅ 已评估（2026-09-24）→ **不改** | 见 `reports/binary-size-analysis-2026-09.md`：发布形态实测 `hok.exe` **7,084,544 B**（6.76 MiB，winsqlite3 形态；8.9 MB 是旧数）；实测 fat LTO 比 thin **省 506 KB**、`opt-level="z"` 已最优 ⇒ profile 无空间。体积是结构性的（一方 1.93 MB / regex 栈 1.06 MB / libgit2+ssh2 0.88 MB / 解压栈 0.86 MB）；杠杆在**分发压缩**（7z 2.26 MB、zip 3.12 MB）。`cargo bloat` 那条 40 分钟的路子已废弃，改用 PDB section-contributions + thin-LTO 探针归因 |
| P2-6 clap 参数组 flatten 备选 | ⏸（首选方案已否决，净 +54 行） | 仅在未来出现漂移证据时再动 |
| output 系统阶段 **③b 源头发射门控 + 进度节流** | ⏸ 评估后推迟 | 基准实测 220–260 ns/事件，可忽略；见 `output-system-design.md` §十、devlog `2026-08-29-phase3b-emit-gating-deferred.md` |
| `git2` ssh feature 去留 | ⏸ 已分析未做 | 见 §5（与私有 bucket SSH 同一主题，一起决策） |
| **libscoop 能否独立给第三方用** | ⏸ **已实测：不能，且能构建也不划算**；路线待定（用户 2026-09-29 决定"先不动，登记待后续会话"） | **构建面三条阻断（互不重叠）**：① `cargo package -p libscoop` exit 101（`hok-i18n`/`hok-shim` 是无版本 path 依赖，且 `crates/hok-i18n/Cargo.toml:11` `publish = false`）；② `cargo package -p hok-i18n --list` 只有 6 个文件、无 `locales/`（`crates/hok-i18n/build.rs:33` 读 `../../locales`，缺失即 panic）⇒ 发布件不可构建；③ `target/` 移走后 `cargo build -p libscoop` panic 在 `crates/libscoop/build.rs:204`（`[build-dependencies] hok-shim`（`Cargo.toml:94-95`）是空壳，`build.rs` 零引用，实际靠探测 `target/`）。**能力面（用户追问，实测同样成立）**：`Session::emitter()` 是 `pub(crate)`（`session.rs:538`），公开面只有裸 `EventBus`；第三方要么完全不接总线（自动批准/自动选首个候选），要么"接了不驱动"⇒ `resolve.rs:276`、`sync.rs:395` 的 `receive_response().await` **永久挂死**，而 `EVENT_BUS_CAPACITY = 20` + 全程 `try_send` 会让"部分实现"静默降级。要复刻的应用外壳（`eventloop.rs` 348 + `scoop_handler.rs` 345 + `output.rs` 484 + `cui.rs` 452 = **1,629 行生产代码**）全在 CLI crate，libscoop 只给 `EventHandler` trait。**另有半执行的决定**：`c99f07e` 在 hok-i18n 写下"libscoop is no longer treated as a publishable crate"，但 libscoop 无 `publish = false`，两处 README 仍挂 crates.io/docs.rs 徽章（crates.io 最新仍是上游 `0.1.0-beta.7`/2024-12-10）。**两条互斥路线**：A 登记为"不可独立使用"（当前架构下唯一自洽，工作量小）；B 真做成可独立引用（构建架构 + `locales/` 搬进 crate + 先发布 i18n 两 crate + **库内建默认事件消费者**，否则第三方仍用不了；`git2`+`rusqlite-bundled`+仅 Windows ⇒ docs.rs 永远构建不了）。来源：[`systemic-optimization-audit-2026-09.md`](../audit/systemic-optimization-audit-2026-09.md) §1.5 + §1.6 |
| 清单缓存：本地桶 mtime/size 行级记账 | 未做（收益为 0） | 唯一收益场景是无 git 的本地目录桶；本机 4 个桶全是 git 仓库。真要做按"目录 stat 差分 → 只重读变化文件"落地 |
| 表格：CJK 包名的缓存文件被拒 | ✅ 已修（2026-09-25，`ff05b80` + `cb75ba4`） | 根因比 CJK 更宽：`REGEX_CACHE_FILE` 的 name/version 字符类 `[0-9a-zA-Z._-]+` 让**任何**含 `+`/`_` 的版本号（`1.9.11_2007`、`1.4.3-canary.1+29d9638da`）与 CJK 包名一样不可见 ⇒ `cache` 列表、`cleanup -k`、`cache rm <app>` 三处都跳过它，只有 `cache rm *` 能删；已改 `^[^#]+#[^#]+#`。同批：`cleanup -k` 补删除汇总行、坏安装的缓存也删（详见 devlog 2026-09-25） |
| 测试 flaky：`manifest_cache::tests::serving_a_stale_cache_warns_once` | ✅ 已修（2026-09-26，`9af599a`，worktree `.worktrees/s-warn-latch`，分支 `fix/manifest-warn-latch`） | **修法（用户拍板 A+B）**：两处闩锁从进程级 static 搬进 `Session`（`stale_warned: AtomicBool` + `mismatch_warned: Mutex<HashSet<String>>`，`mark_stale_warned` / `mark_mismatch_warned`），并把 stale 判据提到 `swap(true)` **之前**（新鲜查询不再吃掉会话唯一的一次告警）；两个 `#[cfg(test)]` `reset_*_warning()` 随进程级状态一并删除，i18n 文案零改动。**根因 = 进程级 once 闩锁 + 每会话 sink 错位**：`manifest_cache.rs:163-184` 的 `static STALE_WARNED: AtomicBool` 在 `warn_if_cache_stale` 里**先 `swap(true)` 再判断是否 stale**（新鲜缓存也会吃掉闩锁），而 `session.output()` 是每会话的 ⇒ 并行测试互抢。`MISMATCH_WARNED: Mutex<Vec<String>>`（`:220-260`，key=`bucket/name`）同型；调用点 `query.rs:341-354`。**实测**（主线 `cb75ba4` clean 树）：`cargo test -p libscoop cache::` 默认并行 **2/25 失败**、同一命令 `-- --test-threads=1` **0/12 失败**，失败点恒为 `manifest_cache.rs:1075` "a stale cache must warn"。**实测**：改前同机 25 次并行该 filter **15/25 失败**（失败点与挂账逐字相同），改后 **0/20 失败**；`cargo test -p libscoop` 789 passed / 0 failed、`cargo test -p hok` 全绿、clippy + `rustfmt --edition 2024 --check` 干净。新增 3 条测试钉住语义（新鲜不吃闩锁 / 每会话各警告一次 / 会话内不重复），并**反向验证**：临时把闩锁改回进程级 static 或把 `swap` 提到 stale 判断之前 ⇒ 对应用例确定性失败（已回退） |
| `scripts/*.py` 与 `.agents/tmp` 审计脚本重叠 | 待收敛 | 建议并入 `scripts/` 一处 |

---

## 5. 专题计划挂起（方案已定稿，实现未开工）

**私有 Scoop bucket 支持** —— `.agents/docs/active/plan/private-bucket-plan-2026-09.md`（状态：方案定稿，实现挂起）：

1. **P1a SSH 凭据**（`internal/git.rs` 凭据回调加 `SSH_KEY` 分支：agent → `id_ed25519`/`id_ecdsa`/`id_rsa`，回调计数推进来源）；
2. **P1b SSH hostkey TOFU**（新模块 `internal/ssh_known_hosts.rs`，`accept-new` 语义，挂 `certificate_check`）；
3. **P2 凭据接入 git 传输**（`Git` trait 加 headers → `FetchOptions::custom_headers`；优先级 `private_hosts` > `SCOOP_GIT_TOKEN_<HOST>` > `SCOOP_GH_TOKEN` > config `gh_token`；只对 `https://` 注入）；
4. **P3 README 文档**（私有 bucket 与 proxy 小节）。

现状：**私有 HTTPS 今天就能用**（libgit2 → 系统凭据助手，非沙箱实测通过）；**私有 SSH 完全不可用**（缺凭据分支 + 缺 hostkey 校验）。前置 skill：`investigate-first`、`hok-code-organization`、`no-patchwork-fixes`。
验收要求**非沙箱手工三验**（沙箱内 Schannel 不可用）。若只做「关 ssh feature」则不满足官方 SSH 支持，需与 P1a/P1b 一起定夺。

---

## 6. 代码内 TODO（4 处，已核实）

| 位置 | 内容 | 处置 |
|---|---|---|
| `package/resolve.rs:291` | `TODO: handle this case smartly using pre-defined bucket priority` | 与 R6/多 bucket 主题相关，暂留 |
| `package/checkhashes.rs:97` | 注释说明 `"hash": "TODO"`/未知算法**显式报解析失败**（S10 决定） | **保留**（是说明，不是待办） |
| `package/checkver.rs:17,47-60` | `TODO(scoop-alignment)` 块声明 `ThrowError (--throw)` 未实现、UA/Referer/PRIVATE_HOSTS 为 gap | ✅ **已清理**（`b339afc`，随 S1 落地）：该块已陈旧（S9 已落 `--throw`；D4 已把手头收敛到 `ScoopRequest`/`HeaderProfile`） |
| `.agents/skills/scoop-gap-audit.md`「未做待办（归 S9）」 | 记「`mode: github` 缺 digest 为 null → 回落下载 + 本地 SHA-256」 | ✅ **已更正**（2026-09-13 父会话主树）：该行改为已解决——`checkver_hash.rs:172-187` 现已在无 digest 时回落 `download_and_compute_sha256` |

---

## 7. 文档 / 收尾卫生

| 项 | 说明 |
|---|---|
| **待归档候选**（任务已完成使命，按 `hok-report-archive` 判定 #3） | `.agents/docs/active/spec/task-help-i18n.md`（由 `c4082b6` + `d29297f` 完成：`--help` 结构头统一 + 全命令帮助面测试）；`.agents/docs/reports/prompts/clap-arg-parsing-corruption-prompt.md`（同一对提交闭环）；`prompts/{current-version-resolution,tolerant-json-single-entry,i18n-inhouse-replacement,base64-single-codec}-prompt.md`（均已落地并被 `reports/optimization-audit-2026-09.md` §9–§11 吸收） |
| `reports/README.md` 索引 | `optimization-audit-2026-09.md` 仍标"待执行"，但 P1 已全部执行完毕（§8 执行结果）⇒ 措辞需更新；§11 之后新增的报告需补进索引 |
| 未跟踪杂项决断 | ✅ **已决断（2026-09-26）**：`scripts/analyze_cohesion.py`、`scripts/analyze_rust_project.py` **已删除**（只扫 `src/`、不覆盖 `crates/`；在 GBK 控制台 `UnicodeEncodeError` 崩溃；职责已被 cargo-modules / clippy 取代。备份在项目外 `_backups/`）；仓库根 `AGENTS.md`、`CLAUDE.md` 与 `.agents/**` **已入库** |
| NSIS 上游提案 | `.agents/docs/legacy/nsis-issue.md` 的 2 个 PR（目录跟踪 API、solid 静默失败修复）**仍值得提交上游**——hok 侧已缓解（`max_decompressed_size(512 MiB)`）≠ 上游根治；PR 3（zstd）已实证撤销 |

---

## 8. 开工计划（分波 + 每会话卡片）

> 依据 `.agents/skills/hok-agent-orchestration`（重叠矩阵 → 并发封顶 2–3 → prompt 契约 → 父会话收口）
> 与 `.agents/skills/hok-worktree-parallel`（一会话一 worktree、`--ff-only` 线性落地、共享文档只在主树改）。

### 8.1 父会话开工前检查（每次派活前 5 分钟）

1. `git rev-parse --short $(git config worktree-merge.mainbranch)` = 主线当前 tip（**派活时以实际 tip 为准并写进 prompt**）；
2. `git status --short` 主树是否干净（`.agents/**` 与 `AGENTS.md` 已入库；未跟踪项只应是 `.agents/tmp/` 与本机 `*.local.md`）；有 ` M` 跟踪文件改动要先 stash 记录；
3. `git worktree list` 只应有主树 + 本轮要开的；
4. 合并脚本在位：`~/.dsh/skills/worktree-parallel/scripts/worktree-merge.ps1`（自测 `test-worktree-merge.ps1` 须在仓库根跑）；
5. 本机环境坑提醒（写进每条 prompt）：**出站 HTTPS 不可用**（真实源冒烟不可行，只能本地夹具/回环）、`gh` 可用、沙箱 Schannel 不可用。

### 8.2 重叠矩阵（派活前必做）

**文件重叠**（热点：`locales/{en,zh}.yml`、`README*.md`、`src/cmd/mod.rs`、`tests/exit_codes.rs`）：

| 会话 | 主要改动面 | 与谁重叠 |
|---|---|---|
| S1 `H1` | `crates/libscoop/src/package/checkver.rs`（+ 可能 `manifest_walker.rs`） | **与「§6 陈旧 TODO 注释清理」同文件** ⇒ 注释清理并入 S1（独立 commit） |
| S2 `H2` | `internal/fs.rs`、`config.rs`、`session.rs` | 与 S1 零交集 ✅ |
| S3 `H3` | `package/query.rs`（+ 可能 `manifest.rs`、`locales/*`） | 与 S1/S2 零交集；若引入用户可见输出会碰 `locales/*` ⇒ 与 S4 需错开 |
| S4 `H4` | `src/cmd/auto_pr.rs`、`package/auto_pr.rs`、`src/cmd/shared_args.rs` | 与 S3 可能同碰 `locales/*` ⇒ **不要与 S3 同日开** |
| S5 `R5` | `package/resolve.rs`、`sync_remove.rs`、`src/cmd/uninstall.rs`、`locales/*` | 与 S4 同碰 `locales/*`、`src/cmd/` ⇒ 分批 |
| S6 `R6` | `internal/network.rs`（+ 测试夹具） | **与「CheckUrls Referer」同文件同语义 ⇒ 合并成一个会话** |
| S7 私有 bucket SSH | `internal/git.rs`、新 `internal/ssh_known_hosts.rs`、`bucket.rs`、`src/cmd/bucket.rs`、`README*` | 与 S6 零交集；README 与文档类改动串行 |

**语义重叠**（比文件重叠更容易漏）：本轮**没有**跨会话共享同一谓词/单入口的组合——
S1 只动 checkver 执行流，S2 只动路径/写盘，S3 只动查询层错误传播，S6 只动请求头。
唯一要记住的既有约束：请求头**唯一入口**是 `ScoopRequest`/`HeaderProfile`（S6 只改注入语义，别新开第二条注入路径）。

### 8.3 分波与并行度

| 波次 | 会话 | 并行度 | 依赖 |
|---|---|---|---|
| **Wave 1** | S1 `H1` ∥ S2 `H2` | **2 条**（零文件交集） | 无 |
| **Wave 2** | S3 `H3`（设计题，单开）∥ S4 `H4` | **2 条**（错开 `locales/*`：S3 先落地再开 S4，或反之） | Wave 1 落地后 |
| **Wave 3** | S5 `R5` ∥ S6 `R6+CheckUrls` | **2 条** | Wave 2 落地后；与 S7 串行（README 冲突） |
| **Wave 4** | S7 私有 bucket SSH | **1 条**（四步四提交，本身够长） | 用户要 SSH bucket 时再开 |
| 随手项 | 父会话主树直接改（不占 worktree） | — | 任意时刻 |

**并发上限 2–3 条**：超过就要预期父会话手工重放（沙箱 `rebase --continue` 不通，只能 `cherry-pick -n` + `commit -C`）。

---

### 8.4 每会话卡片

#### S1 — `H1` `checkver` 挂起（Wave 1）

- **worktree / 分支**：`.worktrees/r1-checkver-hang` / `fix/r1-checkver-hang`（base = 主线当时 tip）
- **复现（已确认）**：`hok checkver -d <空目录>` → 无输出、30s 不退出（`target/release/hok.exe`，主线 `4748ae1`）
- **目标**：无任何清单声明 `checkver`（含空目录）时**有限时间内正常结束**
- **结论（S1 已交回；机制经父会话独立核验）**：`checkver.rs:383` 的 `.buffer_unordered(pending.len())` 在 `pending` 为空时容量为 **0**；vendored `futures-util-0.3.34/src/stream/stream/buffer_unordered.rs:62-86` 在 `max=0` 时永不 poll 源流 ⇒ `is_done()==false` ⇒ 返回 `Pending` 且**不注册任何 waker** ⇒ `collect().await` **确定性永久挂起**（无输出、无 CPU 占用，`-t/--timeout` 是单请求超时、到不了）。**触发面比原判断更宽**：①空目录 ②全部无 checkver ③app glob 不匹配 ④**任何 `--version` 调用**（`:241-269` 整支 `continue`、pending 恒空；实测先打印版本行再挂死）⇒ `--version` 与"零 checkver 清单"无关，必须一并覆盖
- **用户决策（2026-09-13）**：①修法取 **A2**（`.buffer_unordered(pending.len().max(1))`）；②回归测试**一并钉住 `--version` 不挂**；③**做**陈旧 TODO 清理（须连 `:17-19` 模块 doc 一起改），独立 commit；④**同类站点一并加固**（**用户覆盖 S1 的"不做"建议**：逐站点审计，可能为零容量的保证 ≥1，已结构性安全的用测试/`debug_assert!` 钉不变量、不堆冗余 `max(1)`），独立 commit。已转达、S1 已开工
- **最终交付 + 已收口（2026-09-13）**：S1 先重放到主线 `9b67220`（5/5 `cherry-pick -n` 零冲突），最终 tip **`437954842f108d42c824229464d51ea716dce3d5`** → 父会话 `worktree-merge.ps1 -VerifiedRef 4379548…` **`--ff-only` 落地**，主线 tip = **`43795484`**，无 merge commit，漂移校验"逐字节相同"，`git ls-files .dsh` = 0。提交：`a932bd4`（A2 修复）＋`224298c`（回归，含 `--version`）＋`b339afc`（清陈旧 TODO）＋`4a270fb`（同类站点加固）＋`4379548`（style/rustfmt）。
  - **④ 的实际形态（与计划里的示例标题不同，有意）**：全仓 `buffer_unordered` 仅 4 处——`checkver.rs` 已由 ① 修；`download.rs` 跨包 fan-out 上游已 `jobs.max(1)`、HEAD 探测已内联 `jobs.max(1)` ⇒ **不加恒等 `.max(1)`**，改为在容量**消费点**放 **debug-only `debug_assert!(jobs >= 1, …)`** + 注释；`download_fragmented.rs` 在函数入口加 `debug_assert!(chunk_count >= 1, …)`（同时覆盖 `remote_size / chunk_count` 除法）。**零行为变更、零 release 开销**，故用 `chore(download)` 而非 `fix(streams)` —— 诚实反映"无 live 缺陷"。
  - 回归测试**做过负对照**：把 `.max(1)` 临时改回后，新测试在 10s 预算上 `Elapsed(())` 失败而**不会挂住测试 runner**（随后已还原）。
  - 备份分支 `r1-checkver-hang-backup`（重放前 `9e1b76e`）保留，确认后可删。
- **范围**：`crates/libscoop/src/package/checkver.rs` 为主；`manifest_walker.rs` 按需
- **硬性禁区**：不改 checkver 的输出文案、`--throw` 语义、请求头注入（唯一入口 `ScoopRequest`）、hash 提取链
- **顺带项（独立 commit）**：清理 `checkver.rs:47-60` 已陈旧的 `TODO(scoop-alignment)` 块（`--throw` 已落、请求头已收敛）
- **验收**：①空目录 + ②有清单但全部无 checkver 的目录，均在数秒内退出且**不产生网络请求**（本地夹具/离线断言）；③有 checkver 的正常清单行为逐字节不变；④`cargo test -p libscoop -p hok`；⑤给可复跑的复现命令 + 前后对照
- **提交**：① `fix(checkver): stop hanging when no manifest declares checkver` ② `test(checkver): pin the no-checkver completion` ③ `chore(checkver): drop the stale alignment TODO block`
- **开场 prompt**：
  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 §8.4 S1（= fix plan §8 R1）执行：先读本表 §1 H1 与
  .agents/skills/hok-worktree-parallel，开 worktree .worktrees/r1-checkver-hang（分支 fix/r1-checkver-hang，
  base=主线当前 tip），复现 `hok checkver -d <空目录>` 挂起并定位挂起点，给出候选修法与影响面，
  等我确认后再改代码。
  ```

#### S2 — `H2` 缺环境变量 panic（Wave 1）

- **worktree / 分支**：`.worktrees/r2-env-panic` / `fix/r2-env-panic`
- **根因（S2 复核后更正，2026-09-13）**：清空 `USERPROFILE`/`HOME` → `Session::new:182` → `config::init_default:780` → `config::write:790` → `fs::write_json:63` `path.parent().unwrap()` on `None`。写链上的空路径来自 **`hok_config_path()`**（`config.rs:889-891`，经 `Config::default` 的 `path:` 字段 `config.rs:758`），**不是** `scoop_config_path()`（后者只喂 `ConfigBuilder::scoop_path`，只读、从不写）。`hok_config_path` 走 `home_dir()`（仅 `USERPROFILE`/`HOME`）⇒ 与 `APPDATA` 无关（实测：保留 `HOME`、去掉 `APPDATA` **不炸**）。**影响面**：panic 在 `Session::new` ⇒ 所有建 session 的命令都炸（`list`/`info`/`config -a`/`cache`/`status`/`health`），仅 `--version` 不炸。**第二个可达入口**：`config.rs:110`（迁移写）在 exe 旁有 portable `config.json` 且无 home 时同样 panic
- **S2 交付（第一版，2026-09-13）**：tip **`3bf7203`**（→ 完整 sha `3bf72035d50ab077008fe3b2e5be347090926cbb`），提交 ① `82969d7 fix(config,fs): degrade instead of panicking without a home directory` ② `3bf7203 test(fs): pin the empty-path write`；均签名；门禁 libscoop lib **745 passed / 2 failed**（= 基线 741 + 4 新测试，失败仍是那两条 ACL 噪声）、clippy/rfmt 干净；改前 panic → 改后 `list`/`info`/`config -a` 均不 panic，正常环境 4 命令**逐字节相同**
- **用户 2026-09-13 拍板（4 条全答）**：①**采用 (a)+(b)**；②`fs.rs:115` **一并纳入**；③**`root_path` 空 → CWD 相对路径的相邻坑「本主题一并处理」**（比 S2 建议范围更大，须"明确报错而非静默落 CWD"）；④错误文案**英文**、复用 `Error::Io`/`error.io`，不新增 i18n 键
- **S2 抛回的开放问题 + 用户追加决策**：官方 `lib/core.ps1:1291-1301` 把空串 `SCOOP`/`SCOOP_GLOBAL` 当**未设置**并回落默认（`$HOME\scoop` / `C:\ProgramData\scoop`），而 S2 首版把空串当"根=空"报错 ⇒ **用户选「方案 A：对齐官方」**，已转达 S2 在同一分支追加 commit（并**按证据**删除因此变多余的 `require_global_path`/`ensure_global` 守卫；(a)/(b) 必须保留）
- **最终交付 + 已收口（2026-09-13）**：最终 tip **`9b672207ae6fd160fc59bd6d446db0bbe9005799`**（三提交全签名）→ 父会话 `worktree-merge.ps1 -VerifiedRef 9b672207…` **`--ff-only` 落地**，主线 tip = **`9b672207`**，无 merge commit，漂移校验"逐字节相同"，`git ls-files .dsh` = 0。
  - 方案 A 实现：`config::default::env_path(name)` = `var_os(name).filter(|v| !v.is_empty())`（复用 `home_dir`/`config_dir` 既有约定），用于 `SCOOP`/`SCOOP_GLOBAL`/`SCOOP_CACHE`/`ProgramData`；实测 `SCOOP=""→<USERPROFILE>\scoop`、`SCOOP_GLOBAL=""→<ProgramData>\scoop`、`SCOOP_CACHE=""→<root>\cache`。
  - **按证据删掉**变不可达的守卫（`require_global_path`、`require_known_root`、`Session::require_effective_root_path`、`ensure_global` 守卫）；**保留** (a) `fs.rs:63/115` 降级、(b) `init_default` 跳过空路径、`require_root_path` + CLI 启动守卫（`SCOOP` 未设且无 home 时仍可达，且全局 `buckets` 也按用户级 root 解析）。
  - **撤销一条残留**：原报"`update`/`reset`/`import`/`shim` 直接 `set_global(true)` 未经守卫"——方案 A 后实测 `hok shim list -g` 在 `SCOOP_GLOBAL=""` + 隔离 ProgramData 下落 `<ProgramData>\scoop\shims`、CWD 零条目 ⇒ **该残留已消失**。
  - **仍未动（登记）**：`shortcut.rs:41`、`psmodule.rs:87` 的 CWD 相对路径潜在项；`SCOOP="."`（normalize 后空 root，已被启动守卫覆盖）与 `SCOOP_GLOBAL="."`（早返回不 normalize，既有行为）。
- **目标**：无 `USERPROFILE`/`HOME` 的环境**降级或明确报错**，绝不 panic
- **决策门**：改哪一层？候选——(a) `fs::write_json`（`fs.rs:63`）+ `fs.rs:115` 的 `parent().unwrap()` 改 `?`/降级；(b) `config::init_default` 对空路径**跳过写入**（它已是 best-effort，且注释承诺"reverts to defaults instead of panicking"，但只覆盖了读）；(c) 让 `scoop_config_path` 回退 `%APPDATA%`（注意：返回空路径是**有意**的读链回落设计，改动面更大）。**推荐 (a)+(b)**，等确认
- **硬性禁区**：不改配置键名/schema；不引入补丁式开关；不改读链的回落语义
- **验收**：①清空 `USERPROFILE`/`HOME` 隔离环境跑 `hok list` / `hok info` / `hok config -a` 均 **exit 0 或明确报错，无 panic**；②`write_json` 对空路径返回 `Err` 的单测；③正常环境行为不变；④`cargo test --workspace`（已知噪声：`permission::grant_users_write_inherit_*` 2 条 ACL 环境性失败）
- **提交**：① `fix(config,fs): degrade instead of panicking without a home directory` ② `test(fs): pin the empty-path write`
- **开场 prompt**：
  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 §8.4 S2（= fix plan §8 R2）执行：先读本表 §1 H2（含已确认的
  回溯链与文件名:行号），开 worktree .worktrees/r2-env-panic（分支 fix/r2-env-panic，base=主线当前 tip），
  先复现 panic，再给"改哪一层"的候选与影响面，等我确认后再改。
  ```

#### S3 — `H3` 清单解析失败静默消失（Wave 2，**设计题单开**）

- **worktree / 分支**：`.worktrees/r3-parse-visibility` / `fix/r3-parse-visibility`
- **目标**：定"搜索/安装遇到不可解析清单"的可见性策略并落地
- **决策门（必须先报告，禁先改）**：候选 = 报错 / 警告 / 计数 / 保持静默；影响面覆盖全部清单扫描路径（`search`/`list`/`install`/`info`/`checkver`/`checkurls`/missing-checkver）
- **量化要求**：用本机 4 个真实 bucket（约 21797 个 manifest）统计不可解析数量与受影响命令面（参照 P0-4 那次的量化做法）
- **硬性禁区**：不改 `internal::json` 的宽容度契约（11 条三方裁决）；不把"严格解析"改回"warn→null"
- **验收**：①决策门确认的策略在**所有**扫描路径一致；②隔离夹具（故意放坏清单）逐命令实测；③`cargo test --workspace`
- **提交**：① `fix(query): surface manifest parse failures instead of dropping them` ② `test(query): pin the visibility policy` ③（如需）`i18n`
- **开场 prompt**：
  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 §8.4 S3（= fix plan §8 R3）执行：这是设计题，先读本表 §1 H3 与
  .agents/skills/hok-agent-orchestration §3，开 worktree .worktrees/r3-parse-visibility（分支 fix/r3-parse-visibility），
  出"可见性策略候选 + 真实 bucket 量化影响面"报告，等我确认后再改代码。
  ```

#### S4 — `H4` `ci-auto-pr` 对齐（Wave 2）

- **worktree / 分支**：`.worktrees/r4-ci-auto-pr` / `fix/r4-ci-auto-pr`
- **目标**：对齐审计 §7.7 的差异点
- **决策门**：①`-d` 当前默认 `bucket`，按 `-d` 写盘护栏的统一规则应改为"**写盘时必须显式目标**"（破坏性，需确认）；②变更判定机制（hok 字节快照 vs 官方 `hub diff`）是否对齐
- **硬性禁区**：不改 `-d` 护栏工具函数 `write_dir`/`scan_dir` 的既有语义（复用即可）；不在 CI 环境做真实 push 测试
- **验收**：①隔离环境干跑（无变更 / 有变更两条路径）；②缺 `-d` 时 exit 1 + 诱饵树未动；③`cargo test --test exit_codes --test cli_help_i18n`
- **提交**：① `fix(ci-auto-pr): require an explicit target when writing` ②（如需）`align change detection` ③ `i18n/docs`
- **开场 prompt**：
  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 §8.4 S4（= fix plan §8 R4）执行：先读本表 §1 H4 与
  reports/command-deviation-audit-2026-09.md §7.7，开 worktree .worktrees/r4-ci-auto-pr（分支 fix/r4-ci-auto-pr），
  先出"`-d` 护栏落法 + 变更判定是否对齐"的候选，等我确认后再改。
  ```

#### S5 — `R5` 恢复 `-c/--cascade`（Wave 3）

- **worktree / 分支**：`.worktrees/r5-cascade` / `feat/cascade-uninstall`
- **目标**：恢复级联卸载（"删除不再被依赖的依赖"），让 `SyncOption::Cascade` 在生产可达
- **决策门**：①flag 形态（恢复 `-c/--cascade` 到 uninstall vs 保持删除）；②held 依赖**恒报错**（要求先 `hok unhold`，无 bypass）确认；③官方无此能力 ⇒ 登记为有意偏离
- **硬性禁区**：**不许顺手删掉** `resolve_cascade`/`NoDependentCheck`；`NoDependentCheck` 目前同样无构造者，一并决策
- **验收**：①三件套场景（依赖仍被需要 → 保留；不再被需要 → 级联删除；held → 报错）逐条实测；②恢复/新增单测；③`cargo test -p libscoop -p hok`
- **提交**：① `feat(uninstall): restore cascade removal` ② `test(resolve): pin the cascade decisions` ③ `i18n/docs`
- **开场 prompt**：
  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 §8.4 S5（= fix plan §8 R5）执行：先读本表 §2 R5 与
  crates/libscoop/src/package/resolve.rs:304-393，开 worktree .worktrees/r5-cascade（分支 feat/cascade-uninstall），
  先出 flag 形态与 held 政策的候选，等我确认后再改；不要删既有级联逻辑。
  ```

#### S6 — `R6` `PRIVATE_HOSTS` 替换语义 + `CheckUrls` Referer（Wave 3，**合并会话**）

- **worktree / 分支**：`.worktrees/r6-private-hosts` / `fix/private-hosts-headers`
- **目标**：①同名头改为**替换**（对齐官方 `$wreq.Headers[$key] = $value`）；②`HeaderProfile::CheckUrls` 不再发 Referer
- **决策门**：①hok 统一替换会比官方更干净（官方 checkver 自身也出重复头）——确认为**有意偏离**；②`checkurls` 结果会变（反盗链 CDN 上从"假坏链"变正常）——破坏性需确认
- **硬性禁区**：保持"无效头名/值会让请求报错"（**别改成静默跳过**）；请求头唯一入口仍是 `ScoopRequest`/`HeaderProfile`，**别新开第二条注入路径**；`Checkver`/`Autoupdate` 保持官方口径
- **验收**：①本地夹具断言同名头**只出现 1 条**；②`count == 1` 的新测试；③`checkurls` 对本地夹具的假坏链用例转正常；④`cargo test -p libscoop -p hok`
- **提交**：① `fix(network): replace private-host headers instead of appending` ② `fix(network): stop sending a Referer from the checkurls probe` ③ `test`
- **开场 prompt**：
  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 §8.4 S6（= fix plan §8 R6 + CheckUrls 追加待办）执行：
  先读本表 §2 R6，开 worktree .worktrees/r6-private-hosts（分支 fix/private-hosts-headers），
  先出"替换语义 + checkurls 不发 Referer"的候选与影响面，等我确认后再改。
  ```

#### S7 — 私有 bucket SSH（Wave 4，方案已定稿）

- **worktree / 分支**：`.worktrees/pb-ssh-buckets` / `feat/ssh-buckets`
- **依据**：`.agents/docs/active/plan/private-bucket-plan-2026-09.md`§4（P1a SSH 凭据 / P1b known_hosts TOFU / P2 token 注入 / P3 README），§5 已给 4 个提交拆分
- **决策门**：先确认"补齐 SSH"路线（vs 只关 `git2` ssh feature 并在 `bucket add` 显式报错登记 gap）
- **验收**：`cargo test -p libscoop`；**非沙箱手工三验**（清沙箱下 TLS 不可用）：GitHub 私有库 HTTPS+token、`private_hosts` 自定义头、SSH 首次写 `~/.ssh/known_hosts` + 篡改后报 host key changed

### 8.5 父会话收口检查单（每次 ff 前 30 秒）

1. 分支引用是否指向子代理**报的** tip（`git rev-parse <branch>`）——手工重放后常留在 detached HEAD；
2. worktree 是否**挂在分支上**（`git rev-parse --abbrev-ref HEAD` ≠ `HEAD`）；
3. `git merge-base --is-ancestor <mainline> <branch>` = true；
4. 主树未提交的跟踪文件改动是否会被本次 ff 覆盖（有则先 `git stash push -m "<会话>-merge-leftovers" -- <files>`，ff 后 `pop`）；
5. ff 后 **`git diff --cached --name-only` 必须为空**（收口脚本失败会把无关文档误暂存进索引，发现即 `git reset`）；
6. 落地后在新树上重跑门禁（树与受测树逐字节相同时可省）；
7. 回填：**本表**对应行 + 审计报告对应条 + 一条 devlog（触发与格式见 [docs/AGENTS.md R7](../../AGENTS.md)；**技能里不再追加落地记录**；汇总型热点文件只在主树改）。

### 8.6 随手项（父会话主树直接做，不需 worktree）

- ✅ **已完成**：`scoop-gap-audit` 里"`mode: github` 缺 digest null 回落"已更正为已解决（2026-09-13，主树改）；
- 待办：`checkver.rs:47-60` 的陈旧 TODO 块清理（**归 S1**，避免与 S1 抢同一文件）；
- §7 的归档与索引：`task-help-i18n.md`、`prompts/clap-arg-parsing-corruption-prompt.md` 及 4 份已落地 prompt → `.agents/docs/reports/`（或 `legacy/`），并更新 `reports/README.md` 索引；
- ✅ §7 的未跟踪杂项决断（2026-09-26）：`scripts/analyze_*.py` **已删除**；根 `AGENTS.md`、`CLAUDE.md` 与 `.agents/**` **已入库**（详见 §7 表格该行）。
- ✅ 已落地（2026-09-26，规则架构）：闸门锚点审计的 3 项——复述收敛（7 组）、根 `AGENTS.md` 的命令计数措辞
  （改指 `src/cmd/mod.rs` 的 `Command` 枚举）、主线名措辞（不再写死 `main`）。逐组本体与"去重门槛归属"的取舍见
  [`.agents/docs/active/audit/rule-gate-anchors-2026-09.md`](../audit/rule-gate-anchors-2026-09.md) §3；
  同轮早先处置的（脚本区清理、全局技能根边界）见该文件 §6。

> **证据纪律**：每条会话交回的是**可复跑的命令 + 前后对照**，不是结论；允许用证据反驳本表（本表的 H2 曾被初判为"疑似已修"，实测推翻了它——**先重跑再下结论**）。

### 8.7 Wave 1 进行中（2026-09-13 派活）

| 会话 | agent id | worktree / 分支 | base | 状态 |
|---|---|---|---|---|
| **S1** | `5fa53fcf-e5b4-41f8-81cf-63af2d8eb59d` | `.worktrees/r1-checkver-hang` / `fix/r1-checkver-hang` | `4748ae1` | **报告已交回、机制经父会话核验**（`buffer_unordered(0)` 确定性死锁；触发面含 `--version`）；**用户 4 条已转达 → 开工实现**（4 个 commit：A2 修复 / 回归含 `--version` / 陈旧 TODO / 同类站点加固） |
| **S2** | `d35260bb-752e-4e44-aa98-c836fef124ab` | `.worktrees/r2-env-panic` / `fix/r2-env-panic` | `4748ae1` | ✅ **已收口进主线**（`--ff-only`，3 提交，无 merge commit；漂移校验"逐字节相同"）→ 主线 tip **`9b672207`**。交付：`82969d7`（fs/config 降级不 panic）＋`3bf7203`（空路径测试）＋`9b67220`（空串 SCOOP/SCOOP_GLOBAL ≡ 未设置，对齐官方） |

- **决策门在子代理自己的会话里回答**（GUI 会话头的血脉目录进入）；**父会话必须保持存活**，否则子代理输入框变只读。
- **收口口径**：子代理报"可合并 tip" → 父会话
  `pwsh "$HOME/.dsh/skills/worktree-parallel/scripts/worktree-merge.ps1" -Worktree <wt> -VerifiedRef <在重放后的树上实测过的 ref>`；
  前提：worktree **非 detached HEAD**、落地后 `git diff --cached --name-only` 为空（脚本失败会把无关文档误暂存进索引，见 `hok-worktree-parallel`）。
- **已完成**的父会话随手项：`scoop-gap-audit` 的「`mode: github` 缺 digest null 回落」已更正为已解决。
- **暂缓**：§8.6 的归档类改动（`task-help-i18n.md`、已落地 prompts）按 `hok-report-archive` 在**会话收官时**统一做——脚本化的归档会牵动 `reports/optimization-audit-2026-09.md` 等处的路径引用，不宜在两条子代理在飞时改。

**Wave 1 现场快照（更新 2026-09-13 24:00：✅ S1 与 S2 均已收口落地）**
- **主线 `refactor/edition-2024`：`4748ae1` → `9b672207`（S2，3 提交）→ `43795484`（S1，5 提交）**，共 **8 个提交全部线性落地、零 merge commit**。
- **S2：✅ 已收口**。落地后门禁 **747 passed / 0 failed**；父会话独立行为复验：①无 `USERPROFILE`/`HOME` 且 `SCOOP` 已设 → `hok list` 打印「尚未安装任何应用。」exit 1、**无 panic**（改前 panic + `exit -1073740791`）；②无 `SCOOP` 且无 home → 明确英文报错 exit 1；③`SCOOP=""` + 有 home → 回落 `C:\Users\<user>\scoop`（对齐官方）。
- **S1：✅ 已收口**（重放到 `9b67220` 后 5/5 零冲突；漂移校验逐字节相同）。落地后门禁 **752 passed / 0 failed**（`cargo_test_exit=0`）。
  - **父会话独立复验**（新树二进制 + 15s 硬超时守卫，隔离 `SCOOP`）：空目录 exit 0（2.9s，含启动）/ 无 checkver 目录 exit 0（86ms）/ glob 不匹配 exit 0（77ms）/ `-V 2.0.0` exit 0 且打印 `app (1.0.0 -> 2.0.0)`（83ms）/ `-V 1.0.0 -s` exit 0（80ms）/ 有 checkver 且连接被拒 exit 0 且报错文案不变（`ERROR app: 下载失败: HTTP status server error (502 …)`，111ms）。**六项全部由"永不退出"变为秒级正常结束**。
- **收口工具与流程均已实跑验证**：`worktree-merge.ps1` 自测 15/15，两条分支各走一遍完整链路（漂移校验 + 落地不变量 + `git ls-files .dsh` = 0）。
- **已完成的收尾**：S1/S2 文档回填全部完成——本文件 §1/§6/§8.4/§8.7、`reports/command-deviation-fix-plan-2026-09.md`（§8 的 R1/R2 标记完成 + 主线 tip 更新）、`.agents/skills/scoop-gap-audit.md`（新增 R1/R2 两节）、`.agents/docs/devlog/README.md`（新增一节 + 7 条教训）。**Wave 1 的两个 worktree 已于 2026-09-13 删除**（删前逐个验证 `merge-base --is-ancestor <branch> HEAD` = true 且工作区干净；回收约 **14.9 GB**）。**保留的回滚点**：分支 `fix/r1-checkver-hang`、`fix/r2-env-panic`、`r1-checkver-hang-backup` 与 `refs/worktree-backup/*`。W4 剩余：归档 prompt 文档、决断未跟踪杂项、Hindsight 恢复后补录更正。

---

## 8.8 Wave 2+ 开工卡片（复制即用；2026-09-13 立）

### 共同前置（新会话必读）

- **主线 tip 自己取**：`git rev-parse --short $(git config worktree-merge.mainbranch)`（§8.4/§8.7 与下表里的数字都是快照、属历史记录，别当待办）。
- 先读本文件 §1/§2（选条目）与这些 skill：`investigate-first`、`hok-worktree-parallel`、`hok-agent-orchestration`、`hok-rust-gate-practice`、`git-commit-practice`、`proactive-review`、`no-patchwork-fixes`、`token-efficient-processing`。
- **worktree 纪律**：`.worktrees/<会话>-<slug>`，base = 主线**当前** tip；`--ff-only` 线性落地、禁 merge commit；**汇总型热点文件**（本文件、`active/audit/command-deviation-audit-2026-09.md`、`active/audit/scoop-gap-status.md`）**只在主树改**，子代理禁止写（`.agents/**` 其余部分已入库；`skills/scoop-gap-audit.md` 自 2026-09-26 起只是方法论，不再回填）；不合主线、不碰别会话 worktree、不派生下级子代理。
- **门禁基线（Wave 1 落地后）**：`cargo test --workspace` = **libscoop lib 752 passed / 0 failed**。⚠️ 该数字依赖沙箱为 `danger-full-access`；若退回 `workspace-write`，`permission::grant_users_write_inherit_{sets_dacl,is_idempotent}` 会因 ACL `error 5` 失败——**那是环境噪声，不是回归**。
- **门禁命令**：`cargo clippy --workspace --all-targets -- -D warnings`（2026-09-19 起 vendored `innospect` 已退役 ⇒ **不再需要** `--exclude innospect` / `--no-deps`；`--no-deps` 仍可留着提速）；rustfmt **只对改动文件** `rustfmt --edition 2024 --config skip_children=true --check`（**别整包 `cargo fmt`**）。
- **环境坑**：新 worktree 的 `target/` 是空的，`libscoop/build.rs` 要求 `target/{debug,release}/hok-shim.exe` ⇒ **先 `cargo build -p hok-shim`**；`git rebase --continue` 在本沙箱走不通（用 `cherry-pick -n` + `commit -C`）；提交信息**一律** `git commit -F <文件>`；**有未提交改动时绝不 `reset --hard`**；**出站 HTTPS 在 `workspace-write` 沙箱下不可用**（schannel 报 `SEC_E_NO_CREDENTIALS`；同一条 curl 在 `danger-full-access` 下返回 200，Node 自带 CA 不受影响）⇒ 真实源冒烟要么本地夹具/回环，要么放宽沙箱；**拉新的 crate 版本必须先放宽沙箱跑一次 `cargo fetch`**，缓存落盘后普通构建回到沙箱内即可。
- **门禁/决策**：任何"新功能/行为变更/重构/去重/选型"先出调研报告（证据 + 候选含"不做" + 影响面 + 推荐）→ **停下等用户确认** → 再动代码；结论要带**可复跑的命令 + 前后对照**，不是"已修复"。

### W2-A `H3` 清单解析失败静默消失（**设计题，单开，推荐先做**）

```
按 .agents/docs/active/plan/pending-tasks.md 的 §8.8 W2-A 执行（= §1 H3 / fix plan §8 R3）。
先读该文件 §1 H3、§8.8 共同前置，以及 .agents/skills/hok-agent-orchestration §3。
这是设计题：先开 worktree .worktrees/w2a-parse-visibility（分支 fix/parse-visibility，
base=主线当前 tip），量化"清单解析失败静默消失"的影响面（用本机真实 bucket 统计不可解析清单数量与
受影响的命令面），给出可见性策略候选（报错 / 警告 / 计数 / 保持），含代价与风险，等我确认后再改代码。
硬禁区：不改 internal::json 的宽容度契约、不把严格解析改回 warn→null。
```

### W2-B `H4` `ci-auto-pr` 对齐（与 W2-A **错开 `locales/*`**，勿同日开）

```
按 .agents/docs/active/plan/pending-tasks.md 的 §8.8 W2-B 执行（= §1 H4 / fix plan §8 R4）。
先读该文件 §1 H4、§8.8 共同前置，以及 .agents/docs/active/audit/command-deviation-audit-2026-09.md §7.7。
开 worktree .worktrees/w2b-ci-auto-pr（分支 fix/ci-auto-pr，base=主线当前 tip）。
先给两个决策点的候选与影响面，等我确认后再改：① `-d` 当前默认 bucket，按 -d 写盘护栏的统一规则
应改为"写盘时必须显式目标"（破坏性）；② 变更判定机制（hok 字节快照 vs 官方 hub diff）是否对齐。
注意它写盘并提交，测试用隔离环境干跑，不做真实 push。
```

### W2-P 父会话编排（想并行跑 A+B 时用这条）

```
按 .agents/docs/active/plan/pending-tasks.md 的 §8.8 执行：为我派两个子代理并行做 W2-A 与 W2-B
（各自 worktree 与分支，先出调研报告、等我确认后再改），你在父会话负责收口
（worktree-merge.ps1 -VerifiedRef <实测 tip> 串行 rebase + --ff-only + 落地树重跑门禁）与文档回填。
派活前先按 hok-agent-orchestration §1 做文件 + 语义重叠矩阵；并发上限 2；注意两条都可能碰 locales/*。
```

### W3-A `R5` 恢复 `-c/--cascade`（Wave 3）

```
按 .agents/docs/active/plan/pending-tasks.md 的 §8.8 W3-A 执行（= §2 R5 / fix plan §8 R5）。
先读该文件 §2 R5、§8.8 共同前置，以及 crates/libscoop/src/package/resolve.rs 的 resolve_cascade。
开 worktree .worktrees/w3a-cascade（分支 feat/cascade-uninstall，base=主线当前 tip）。
先给定 flag 形态与 held 政策的候选，等我确认后再改。硬禁区：不许顺手删掉既有级联逻辑
（SyncOption::Cascade / NoDependentCheck 目前无构造者，但 resolve_cascade 已被 4 个单测覆盖）。
```

### W3-B `R6` `PRIVATE_HOSTS` 替换语义 + `CheckUrls` Referer（Wave 3，**同一会话合并做**）

```
按 .agents/docs/active/plan/pending-tasks.md 的 §8.8 W3-B 执行（= §2 R6 + CheckUrls 追加待办）。
先读该文件 §2 R6 与 §8.8 共同前置。开 worktree .worktrees/w3b-private-hosts
（分支 fix/private-hosts-headers，base=主线当前 tip）。
先给两个候选与影响面，等我确认后再改：① 同名头由 append 改 replace（官方 $wreq.Headers[$key] = $value；
hok 统一替换会比官方更干净，属有意偏离）；② HeaderProfile::CheckUrls 跟随下载路径不再发 Referer
（反盗链 CDN 上的"假坏链"会变正常，属行为变化）。
硬禁区：别改成静默跳过无效头（现在是 loud）；请求头唯一入口仍是 ScoopRequest/HeaderProfile，
别新开第二条注入路径；Checkver/Autoupdate 保持官方口径。
```

### W3-C 私有 Scoop bucket SSH（Wave 4，方案已定稿）

```
按 .agents/docs/active/plan/private-bucket-plan-2026-09.md §4 执行（P1a SSH 凭据 / P1b known_hosts TOFU /
P2 token 注入 / P3 README），配合 .agents/docs/active/plan/pending-tasks.md §5 与 §8.8 共同前置。
开 worktree .worktrees/w4-ssh-buckets（分支 feat/ssh-buckets，base=主线当前 tip）。
先确认路线（补齐 SSH vs 只关 git2 的 ssh feature + 显式报错登记 gap），等我确认后再改。
验收要求非沙箱手工三验（沙箱内 TLS 不可用）。
```

### W4 收尾杂项（小会话，也可在主树直接做）

> ⚠️ 本卡片写于 2026-09-13：③ 依赖的 `hindsight_ingest_document` 已无法证实存在（仓库内只剩文档自引用），
> ④ 的未跟踪杂项已在 §7 / §8.6 决断——**这两项都别再执行**，其余照卡片。

```
按 .agents/docs/active/plan/pending-tasks.md 的 §7 与 §8.6 做收尾：
① 归档已完成使命的文档（task-help-i18n.md、prompts/clap-arg-parsing-corruption-prompt.md 及 4 份已落地
   prompt）到 .agents/docs/legacy/ 或 reports/，并更新 reports/README.md 索引、同步修复被改动的路径引用；
② 清理 Wave 1 的 worktree（git worktree remove .worktrees/r1-checkver-hang .worktrees/r2-env-panic）
   与备份分支（r1-checkver-hang-backup）——保留 refs/worktree-backup/* 回滚点，除非我另有指示；
③ 若 Hindsight 后端恢复（此前 502），补录 R2 根因更正（hok_config_path() 而非 scoop_config_path()）；
④ 决断 git status 里的未跟踪杂项（scripts/analyze_*.py、AGENTS.md：提交 or 忽略）。
```

**基线门禁（2026-09-13，主树 `4748ae1`，`cargo test --workspace`）**：
- `libscoop --lib`：**741 passed / 2 failed / 8 ignored**，2 条失败正是 `permission::tests::grant_users_write_inherit_sets_dacl`（`permission.rs:165`）与 `grant_users_write_inherit_is_idempotent`（`permission.rs:224`）——**既有 ACL 环境性失败，非回归**；
- 其余全部套件通过（47 / 3+9 / 6 / 3 / 6 / 10 / 36 / 79 / 6 …）；
- 唯一编译警告 `unused import: sha1::Digest` 位于 **vendored `crates/innospect/src/crypto/kdflegacy.rs:187`** ⇒ 这就是门禁必须 `--exclude innospect` / `--no-deps` 的原因，与我们无关。**（2026-09-19 起该 vendored crate 已退役，此限制不再存在。）**
- 结论：收口时只要数字仍是 **741 / 2（同样的两条 ACL）** 即视为无回归。
- **⚠️ 口径更新（2026-09-13 收口时实测）**：沙箱切到 `danger-full-access` 后，那两条 ACL 测试**真的通过了** —— 落地后新树 `cargo test --workspace` = libscoop lib **747 passed / 0 failed / 8 ignored**（= 741 + 4 条新测试 + 2 条 ACL 转绿），其余 target 全绿。**判据改为"0 failed"**；若将来再现这 2 条失败，先怀疑沙箱策略退回了 `workspace-write`，而不是当作回归。

**⚠️ 沙箱发现：签名提交在本会话被文件沙箱挡住（2026-09-13 实测）**
- 现象：`git commit`（本机 `commit.gpgsign=true`）失败 —— `gpg: can't create 'C:\Users\...\.gnupg\pubring.kbx.lock': 拒绝访问` + `can't connect to the gpg-agent: Operation not permitted`。
- 定性：**沙箱策略问题，不是 gpg/git 配置问题**。同一条 `git commit-tree -S` 命令改用 `danger-full-access` 重试即成功（`sig=G`，signer `maboloshi`）。根因是 file sandbox = `workspace-write`，而 gpg 需写 workspace 之外的 `~/.gnupg`。
- 已通告两条子代理：**不得**用 `--no-gpg-sign` / `-c commit.gpgsign=false` 绕过（破坏本仓"签名提交"不变量）。
- ✅ **已解除（2026-09-13，随后本次会话的策略被切到 `danger-full-access` 且审批关闭）**：复测 `git commit-tree -S` 直接成功（`exit=0`，`sig=G`，signer `maboloshi`），**无需任何提权**。给子代理的指令已同步更正为"直接正常提交；**不要**再设 `sandbox_permissions`（审批已关闭，会被自动拒绝）；若 gpg 仍失败则停下如实上报"。
- 连带影响：`.dsh` 收口工具自测 `test-worktree-merge.ps1` 在 `%TEMP%` 沙箱副本里因签名失败而 `selftest_exit=1`（**策略切换前**的观测）；策略放宽后重跑 → **15/15 checks passed，`selftest_exit=0`**（A 干净 rebase+ff 无 merge commit / B 冲突 exit 2 且不落地、`-Continue` 可收尾 / C `-VerifiedRef` 漂移 exit 3 + 生成 recovery ref / D 脏 worktree 拒绝 exit 1 / E 已落地 no-op exit 0 / F `-DryRun` 无副作用）⇒ **收口链路已预先验证可用**。

---

## 9. 解压模块（7z 引擎）待决策（2026-09-25 调研，来源见报告）

> 来源：`.agents/docs/active/audit/7zip-switch-investigation-2026-09.md`（含实测表、7z.dll 专项、完整选项）。
> 用户当日拍板：**只先落地 P0**，其余不动——「目前模式貌似没有太大问题」。

| # | 主题 | 状态 | 证据 | 建议 |
|---|---|---|---|---|
| **X1** | 7z 原生路径静默丢「无流条目」（空文件 + 全部目录条目） | ✅ **已修复并落地**（`4bf1397`，2026-09-25） | `extract_7z_entries_mt` 逐 block 解码绕过了 sevenz-rust2 `ArchiveReader::for_each_entries` 尾部的 "decode empty files" 兜底 ⇒ 空文件与所有目录条目（无流、不属于任何 block）永不落盘且不报错。真实包复现：nodejs 丢 4 个 0 字节文件 + 460 个目录条目 | 已加 stream-less 物化 pass + 2 条单测 + 206 B 夹具；实测 nodejs 1934→2398 条目与 7z.exe 逐条一致 |
| **X2** | 「原生失败 → `7z.exe` 单次兜底」+ 未知格式按 Scoop `Test-7zipRequirement` 兜底 | **未开工（待决策）** | 现状：zip/tar/rar/msi 路径解析失败即整包失败（只有 pseudo-7z 链有兜底）；`.7z.001` 分卷 / `.img` 类 URL 被**静默当普通文件拷走**（本机 4 bucket 实测 26 处 URL / ~13 个包） | 收益在兜底不在覆盖：原生成功路径保持纯 Rust（实测多数场景更快），失败才回退。范围与语义需先拍板 |
| **X3** | PS 兼容层 `Expand-7zipArchive` 偏离 Scoop 语义 | **未开工（待决策）** | `asset_scripts/decompress.ps1:15-20` 只写 `[[HOK_EXTRACT]]` 标记、由 Rust 在整段脚本跑完后解压：`-Switches` 被忽略、`-ExtractDir` 实现成 `extract_to`（非 Scoop 的"取子树并上提"）、`-Removal` 先删源文件使标记空转。本机 4 bucket：1295 个 manifest 调用它，535 处带 `-Removal`、154 处带 `-ExtractDir`（**影响面尚未端到端复现**） | 改法（方案 C④）：PS 层直接调 `7z.exe`/`lessmsi`，与 Scoop 同语义；动前必须先复现这 535/154 处的现状 |
| **X4** | `7z.dll` 作为解压后端（进程内、可协作取消、`mt` 可限线程） | **仅登记（未决策）** | 已核实 `7z.exe` 只是 `7z.dll` 的 CLI 前端（UTF-16 动态加载，DLL 导出 `CreateObject`/`GetHandlerProperty2`）；DLL **不能**接宿主线程池。约束：`libscoop` 是 `unsafe_code = "deny"` ⇒ FFI 需另开 crate；用已装 `7zip` 零体积，自带则 +2.45 MB + LGPL/unRAR 再分发义务 | 若目标只是"补兜底 + 修丢文件"= 过度工程；只有确定要收缩自研解码链（rar/nsis/inno/iso）才值得做 spike。**收益不含速度** |

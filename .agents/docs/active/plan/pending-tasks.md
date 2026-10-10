# hok 待办总表

Status: active
Updated: 2026-10-11
维护者：用户（拍板）/ 各会话（回填）

> **这是什么**：散在审计、专题计划与代码里的**未完成项**索引。已完成项不在本表——见 `devlog/` 与 `reports/`。
> **权威性**：本表只是索引；每条的主题细节、决策记录与验收以「来源」指的文件为准。出清时在**同一次改动**里
> 删掉本行并回填来源。
> **开工前置**：先过决策门（`investigate-first`：出报告 + 选项 + 等确认）再动手；worktree、门禁与落地纪律见
> `.agents/skills/hok-worktree-parallel.md`，派活纪律见 `.agents/skills/hok-agent-orchestration.md`。
> **基线**：主线分支名取自机器本地 git 配置（键名与取法见 `hok-worktree-parallel`），派活时以实测 tip 为准。

## 1. 待拍板（有方案，缺决定）

| ID | 主题 | 等谁拍板 | 待决问题 | 来源 |
|---|---|---|---|---|
| **H4** | `ci-auto-pr` 对齐（它写盘并提交，受 `-d` 写盘护栏约束） | 用户 | ①`-d` 是否改为"写盘时必须显式目标"（破坏性）②变更判定是否对齐官方 `hub diff` | [`command-deviation-audit-2026-09.md`](../audit/command-deviation-audit-2026-09.md) |
| **R5** | 恢复 `-c/--cascade`（`SyncOption::Cascade` 无构造者 ⇒ 级联卸载在生产不可达） | 用户 | ①flag 形态 ②held 依赖恒报错政策（要求先 `hok unhold`，无 bypass）③登记为有意偏离 | [`command-deviation-fix-plan-2026-09.md`](../../reports/command-deviation-fix-plan-2026-09.md) |
| **R6** | `PRIVATE_HOSTS` 同名头由 append 改 replace（官方 `$wreq.Headers[$key] = $value`）；**同题合并**：`HeaderProfile::CheckUrls` 不再发 Referer | 用户 | ①统一替换比官方更干净，确认为有意偏离 ②`checkurls` 在反盗链 CDN 上由"假坏链"转正常属行为变化 ③**硬禁区**：请求头唯一入口仍是 `ScoopRequest`/`HeaderProfile`，别新开第二条注入路径；`Checkver`/`Autoupdate` 保持官方口径 | [`command-deviation-audit-2026-09.md`](../audit/command-deviation-audit-2026-09.md) |
| **X2** | 7z：原生解析失败时单次 `7z.exe` 兜底；未知格式按官方 `Test-7zipRequirement` 兜底 | 用户 | 兜底范围与语义（现状：zip/tar/rar/msi 解析失败即整包失败；`.7z.001` 分卷与 `.img` 类 URL 被静默当普通文件拷走） | [`7zip-switch-investigation-2026-09.md`](../audit/7zip-switch-investigation-2026-09.md) |

## 2. 未开工（无待决问题，随时可开工）

| ID | 主题 | 优先级 | 做什么（触发 + 验收要点） | 来源 |
|---|---|---|---|---|
| **RF1** | 超长函数拆分（`checkver.rs` 的 `execute` + `apply_autoupdate` 收益最高） | 有条件 | **前置**：先给出具体问题陈述（性能或维护痛点）；`commit_one_install` / `commit_one_remove` 结构对称，先核对是否真能抽公共管线，别用开关掩盖 install/remove 语义差异 | 同上 |
| **CA1** | `--dry-run`（uninstall → cleanup → install） | 下一批 | 验收 = 零副作用 | [`cli-additions-plan.md`](cli-additions-plan.md) |
| **C1** | 官方独有 config 键的**剩余**真缺口 2 个：`use_external_7zip` / `show_manifest`（`autostash_on_conflict` 已于 2026-10-09 单独落地，键一旦拆出即按用户拍板只做这一键） | 以后 | 各需单独立案：`use_external_7zip` 与 X3/X5 的 7z 面重叠，`show_manifest` 属安装前交互（新增交互面）。入口仍是 `ConfigInner` 加字段 + `set()` match + `list_all` | [`scoop-gap-status.md`](../audit/scoop-gap-status.md) |
| **B1** | `resolve.rs` 的 bucket 优先级 TODO（"smartly using pre-defined bucket priority"） | 以后 | 与"多 bucket 命中"主题合并处理 | 代码内 TODO：`crates/libscoop/src/package/resolve.rs` |
| **X6** | 清单 hash 大小写：官方 `get_hash` 用 `-notcontains`（不区分大小写）接受 `SHA256:xxx`，hok 的 `REGEX_HASH` 大小写敏感 ⇒ 判无效 | 待拍板 | 对齐会扩大对既有清单的接受面，属行为变更；先判是否列为有意偏离 | [`systemic-optimization-audit-2026-09.md`](../audit/systemic-optimization-audit-2026-09.md) + D1 会话记录 |
| **C2** | `Package::is_nightly` 大小写：官方三处 nightly 判定均不区分大小写（`-eq`），hok 精确比较而 `checkhashes` 走 `eq_ignore_ascii_case` ⇒ 同一语义两种口径 | 待拍板 | 对齐会让 download/verify 对 `Nightly` 变体从"不跳过校验"变"跳过校验"；决定后单独提交并更新偏离登记 | 同上 |
| **MS1** | 安装态解析失败仍静默（H3 未覆盖的三个点）：`resolve_current_version` 坏清单**静默回落**到 `newest_version_dir`（可能给出**另一个版本**）、`load_install_state` 把坏清单当"未安装"、`maybe_fill_upgradable` 把坏清单当"不可升级" | 以后 | 需要给共享解析器加失败通道并贯穿 `Session`，调用点跨 7+ 模块（cleanup/health/info/list/session/manifest_source/sync_remove/query）⇒ 单独一条工作流；判据与已覆盖部分见 H3 devlog | H3 会话记录（2026-10-11） |

## 3. 进行中

无。开工中的条目在此登记 owner（worktree + 分支），同一时刻只应有一个 owner。

> 编号是**按落地顺序发号**：`X6` 曾被 X5 会话用于「marker 的 `format` 未被当作格式判定」，该缺陷已于
> 2026-10-11 修掉（`9f59e58`），编号归还待办池后由「清单 hash 大小写」接管。历史记录里出现的 X6 指前者。

## 4. 暂缓（必须写理由与重启条件）

| ID | 主题 | 为什么暂缓 | 什么条件下重启 | 来源 |
|---|---|---|---|---|
| **P1-9** | `update <app>` 先同步 buckets（官方 `Sync-Bucket` 在 app 更新前） | 用户暂缓 | 要求对齐官方语义时 | [`upstream-0.6.0-impact-2026-10.md`](../audit/upstream-0.6.0-impact-2026-10.md) |
| **S4-A** | `aria2-min-split-size` 20M → 5M | 用户在 S4 调研后暂缓 | 出现 aria2 分片相关的下载问题时 | [`s4-config-investigation-2026-09.md`](../../reports/s4-config-investigation-2026-09.md) |
| **S4-P** | `proxy` 三态语义（`none` 存字面值并直连；`default`／`currentuser` 不打挂下载；未设 ≠ 系统代理） | 同上 | 用户要求修 proxy 语义时 | 同上 |
| **CA4** | `edit <app>` | 会修改他人 bucket 仓库，价值未评估 | 用户提出时 | [`cli-additions-plan.md`](cli-additions-plan.md) |
| **CA5** | 原生 `service` 子命令 | 含新增 FFI，需登记 unsafe 审计 | 用户提出时 | 同上 |
| **CA6** | `update -q/--quiet` | 只是默认输出的简洁版，低价值 | 用户提出时 | 同上 |
| **ST1** | `status -l/--local` + Scoop/bucket 新鲜度检查 | 属新功能，用户未排期 | 用户要求 status 本地／新鲜度检查时；`parse_last_update` 的 `dead_code` 允许就是为它留的 | [`status-gap-analysis.md`](../../reports/status-gap-analysis.md) |
| **S8** | 多 bucket 命中保留 hok 交互选择（已知限制：非 tty 下交互失败 exit 1） | 待定，非"已定偏离" | 脚本／CI 需要安装重名包时 | [`command-deviation-audit-2026-09.md`](../audit/command-deviation-audit-2026-09.md) |
| **CU1** | `checkurls` 的 S11 余项（输出与边界差异） | 用户用得少 | 与 R6 同一专题一起做时 | [`command-deviation-fix-plan-2026-09.md`](../../reports/command-deviation-fix-plan-2026-09.md) |
| **RB1** | 桶扫描类兄弟命令的输出统一（`checkhashes` / `checkurls` / `formatjson` / `missing-checkver`） | 用户定为后期专项、不排期 | 用户提出时 | [`manifest-scan-output-2026-10.md`](../../research/manifest-scan-output-2026-10.md) |
| **P2-6** | clap 参数组 flatten 备选 | 首选方案已否决（净 +54 行） | 出现参数漂移证据时 | [`hok-code-organization.md`](../../../skills/hok-code-organization.md) |
| **O3B** | output 阶段 ③b 源头发射门控 + 进度节流 | 基准实测可忽略 | 画像显示 emit 成为热点时 | [`output-system-design.md`](../spec/output-system-design.md) |
| **X4** | `7z.dll` 作为解压后端（进程内、可协作取消） | 仅登记：收益不含速度，且需另开 crate（`libscoop` 是 `unsafe_code = "deny"`） | 确定要收缩自研解码链时 | [`7zip-switch-investigation-2026-09.md`](../audit/7zip-switch-investigation-2026-09.md) |
| **PB1** | 私有 bucket SSH（SSH 凭据 / hostkey TOFU / token 注入 / README；含 `git2` 的 ssh feature 去留） | 私有 HTTPS 已可用、SSH 完全不可用；方案已定稿 | 需要 SSH bucket 时（先定路线：补齐 SSH vs 关 ssh feature 并登记 gap） | [`private-bucket-plan-2026-09.md`](private-bucket-plan-2026-09.md) |
| **L1** | libscoop 独立给第三方用（路线 A 登记为不可独立使用 / 路线 B 真做成可发布） | 已实测不能独立使用，用户"先不动" | 要对外发布 libscoop 时 | [`systemic-optimization-audit-2026-09.md`](../audit/systemic-optimization-audit-2026-09.md) |
| **M1** | 清单缓存按本地桶 mtime/size 行级记账 | 唯一收益场景是无 git 的本地目录桶，本机没有 | 出现无 git 的本地目录桶且成为瓶颈时 | 同上 |
| **N2** | `shortcut.rs` / `psmodule.rs` 的 CWD 相对路径潜在项（缺 home 时的落点） | 只观察到潜在项，尚未复现 | 出现 CWD 相对路径相关的安装／卸载异常时 | 代码：`crates/libscoop/src/shortcut.rs`、`crates/libscoop/src/psmodule.rs` |
| **RF2** | 投机性重构组（事件模型折叠 / 库内默认事件消费者 / install-remove 公共提交管线 / 公共 API 收窄 / 依赖替换） | 属"该等"的一类：需需求稳定或性能证据才能判对错 | 各自的触发条件见 [`systemic-optimization-audit-2026-09.md`](../audit/systemic-optimization-audit-2026-09.md) 的「使用规则」小节 | 同上 |

## 5. 已定不做 / 有意偏离（长期登记，不计入"还剩什么"）

| 项 | 决定 | 理由 / 出处 |
|---|---|---|
| **V1** `virustotal` 对齐（含退出位掩码） | 不做 | 用户基本不用；[`command-deviation-audit-2026-09.md`](../audit/command-deviation-audit-2026-09.md) |
| **EN1** `list` 升级箭头／计数页脚／多查询 OR、`search` 输出形态、`bucket known` 裸名称 | 不改 | 除非用户明确要求；同上（状态表） |
| **EN2** clone 失败打 libgit2 原文、`bucket list -k` 与 `bucket rm` 的多名称、`shim` 无 `alter` | 登记不修 | S14 有意偏离；同上 |
| **EN3** checkver：xpath 命名空间、`checkver.url` 变量替换；Digest base64 失败回落 `.meta4` | 登记不动 | 有意偏离；[`command-deviation-audit-2026-09.md`](../audit/command-deviation-audit-2026-09.md) |
| **EN4** upstream 0.6.0 的两条行为差：`git pull --tags --force`（只作用于桶同步）、`update -f` 丢 pin 重解析到桶 HEAD（hok 是原地重装） | 只记录差距 | 用户 2026-10-03 拍板；[`upstream-0.6.0-impact-2026-10.md`](../audit/upstream-0.6.0-impact-2026-10.md) |
| **EN5** 二进制体积的 profile 调优 | 不改 | 实测体积是结构性的，杠杆在分发压缩；[`binary-size-analysis-2026-09.md`](../audit/binary-size-analysis-2026-09.md) |
| **EN6** 清单历史索引的已知取舍：只走第一父链、不跟随重命名（`--follow`）、`gc --prune` 后按行失效回落 autoupdate | 登记不动 | 有意取舍；[`manifest-history-index-plan.md`](../../reports/manifest-history-index-plan.md) |

## 6. 专题计划（细节在各自文档，本表只留指针）

- [`cli-additions-plan.md`](cli-additions-plan.md) —— 从 hyperscoop 借鉴的 CLI 增补（来源调研
  [`hyperscoop-feature-survey-2026-10.md`](../../research/hyperscoop-feature-survey-2026-10.md)）；未落地项见
  CA1–CA3，已暂缓项见 CA4–CA6。
- [`private-bucket-plan-2026-09.md`](private-bucket-plan-2026-09.md) —— 私有 bucket（对应 PB1）。
- [`cleanup-parallel-spec.md`](../spec/cleanup-parallel-spec.md) —— cleanup 并行化（已落地；失败汇总文案随该专题改）。

# hok 框架评估报告（2026-08 重构后）

> 评估时间：2026-08（重构后现状）。方法：3 个子代理对当前代码实读审计（大文件结构 / 五层架构合规 / unwrap 风险）+ 独立基线验证（cargo check / test / clippy / 依赖树 / 违规扫描）。**历史报告（analysis-report.md、FINAL_AUDIT.md）仅作线索，结论以当前代码为准。**

## 执行状态（按 framework-fix-plan.md）

| 项 | 状态 | 说明 |
|---|---|---|
| 阶段 0（P0×2） | ✅ | eventloop.rs:164 `unwrap_or(p.version())`；download.rs:1185 删恒真断言 → clippy 转绿 |
| 阶段 1（panic 容错×6） | ✅ | home_dir/config_dir `unwrap_or_default`；bucket/query/cache 非 UTF-8 `.ok()?` 跳过；shim.rs:44 file_stem fallback |
| 阶段 2（可见性收口） | ✅ | `Session::set_default_architecture`；internal 全 `pub(crate)`；`Arch` 经门面公开；doc 示例与集成测试适配；**T2.3 收缩**：package/ 保持公开（CLI 是主要消费者）、string 门面保留（非死门面） |
| 阶段 3（config 拆分） | ⚠️ 部分 | 仅 `list_all` 保留库层（移动需公开 ~20 accessor，收益低）；`edit` 移动后**按用户要求恢复库层**（未来 `config edit` 复用）；set/alias/list 本就是领域 API |
| 阶段 4（checkver Args 镜像） | ✅ | package 层 `Args`→`CheckverOptions`（纯数据）；CLI clap Args + 显式构造（编译期双向保护）；auto_pr 调用同步 |
| 阶段 5（P2） | ✅ | T5.4 clippy **全零警告** ✅（含删死代码 empty_dir/Checksum::check）；T5.5 shim.rs 分区注释 ✅；T5.1 archive→event 定论（保留）已写入技能；**T5.2/T5.3 已执行**（见下） |
| T5.2 query 拆分 | ✅ | query.rs 936 行 → `query.rs`（核心+命令操作 ~580）+ `query_matcher.rs`（匹配引擎+12 测试）+ `install_state.rs`（状态解析，含 select_current_version）；InstallState 类型/逻辑分离缺口修复；5 处跨模块引用路径更新 |
| T5.3 download 拆分 | ✅ | download.rs 2004 行 → `download.rs`（核心 608）+ `download_fragmented.rs`（分片+测试桩 926）+ `download_verify.rs`（校验 176）+ `download_apps.rs`（独立命令+测试 355）；`test_server` 共享测试桩模块；成功文案仍留库层（`download_apps` 内联，未上移 CLI——留待后续） |
| 阶段 6（收尾） | ✅ | 技能更新（internal 收口/已接受设计）；本报告状态标记 |

**遗留（backlog）**：P3 低优先项（cmd 令牌 env 直读、checkver --throw TODO、resolve bucket 优先级）、download_apps 成功文案上移 CLI（T5.3 顺带项，未做）。

## 0. 基线状态

| 检查 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | ✅ 通过（1 警告：bucket.rs:543 测试模块 unused import） |
| `cargo test --workspace` | ✅ 全绿（489 单元 + 11 doc-tests，0 失败） |
| `cargo clippy --workspace --all-targets` | ❌ **失败**（1 error + 26 警告） |
| 依赖树 | ✅ 健康（无异常重复版本） |
| package 层 stdout 违规 | ✅ 已清零（重构前 9 处） |
| dead_code allow | ✅ 4 处（constant.rs×2、shim.rs、dag.rs） |
| 重复代码组 | ✅ 版本比较/glob_to_regex/checkver_hash 已合并 |
| 门面 lib.rs | ✅ 纯 re-export，零函数实现 |

---

## 1. 问题点总表（按严重度）

| # | 严重度 | 问题 | 位置 | 性质 |
|---|---|---|---|---|
| P0-1 | **必修** | `upgradable_version().unwrap()` —— `update -f` 且存在已最新包时必 panic（注释与 unwrap 矛盾） | `src/eventloop.rs:164` | **新引入**（2026-08-01） |
| P0-2 | **必修** | clippy error：`assert_eq!(0 * chunk_size, 0)` 触发 `erasing_op`（deny 默认 lint，CI 直接红） | `package/download.rs:1185` | **重构未解决**（历史报告已点名） |
| P1-1 | 高 | `dirs::home_dir().unwrap()` —— 无 USERPROFILE/HOME 时 Session 创建即崩，几乎所有命令启动即 panic | `config.rs:813` | 重构未解决 |
| P1-2 | 高 | `dirs::config_dir().unwrap()`（同类，psmodule.rs:87 已用 unwrap_or_default 但此处未对齐） | `shortcut.rs:38` | 重构未解决 |
| P1-3 | 高 | 非 UTF-8 文件名/目录扫描 panic（`to_str().unwrap()`/`file_type().unwrap()`） | `bucket.rs:79/212/260/261`、`query.rs:333/447`、`cache.rs:20` | 既有 |
| P1-4 | 高 | 可见性落差：`pub mod internal`/`pub mod package` 全公开，与宣称的 pub(crate)/选择性暴露不符，门面无编译器强制力 | `lib.rs:67/70`、`internal/mod.rs` | **新引入**（架构声明 vs 现实） |
| P1-5 | 高 | config.rs 混入命令操作（`list_all` 三列表格、`edit` 编辑器进程、alias_add/remove）——最值得拆 | `config.rs:895+` | 新引入（重构合并导致） |
| P1-6 | 中高 | checkver `Args` 与 CLI clap `Args` 双份镜像结构（逐字段手工拷贝，可漂移重复） | `package/checkver.rs:57` vs `src/cmd/checkver.rs:46-54` | 新引入 |
| P2-1 | 中 | internal/ 唯一真实反向依赖：`use crate::event::Event` 发进度事件 | `internal/archive.rs:24` | 新引入 |
| P2-2 | 中 | CLI 直用 `internal::arch`（install/download 的 --arch）—— internal 无法收口 pub(crate) 的根因 | `src/cmd/install.rs:100-101`、`download.rs:37-38` | 新引入 |
| P2-3 | 中 | query.rs 四关注点挤一文件 + `InstallState` 类型（mod.rs）与读取逻辑（query.rs）分离缺口 | `package/query.rs` | 新引入 |
| P2-4 | 中 | download.rs 2004 行（全仓最大）：分片机制/独立下载命令/共享校验三簇可拆（分区注释已画缝） | `package/download.rs` | 新引入 |
| P2-5 | 中 | 死门面 `libscoop::string` 全仓库 0 引用；fs/os 门面与 internal 双路径并存 | `lib.rs:99` | 新引入 |
| P2-6 | 中 | 26 个 clippy 警告（config 4、env/archive/script/reinstall 等，6 个可 --fix 自动修） | 多处 | 既有+新 |
| P3-1 | 低 | 领域服务↔package 双向依赖（无环）未写入规范，"单向"表述名不副实 | env/persist/psmodule/shim/shortcut/event | 规范缺口 |
| P3-2 | 低 | shim 命名重叠：顶层 shim.rs（生成）vs package/shim.rs（检查）vs cmd/shim.rs | 三层 | 维护噪音 |
| P3-3 | 低 | cmd 层直读环境变量令牌（GITHUB_TOKEN/VT_API_KEY/HOK_LOG_LEVEL） | auto_pr/virustotal/mod.rs | 既有 |
| P3-4 | 低 | 分区注释不一致（8 大文件 4 有 4 无，`// ───` 与 `// ----` 混用） | 多处 | 风格 |
| P3-5 | 低 | TODO 残留：checkver `--throw`（ThrowError）、resolve.rs bucket 优先级 | checkver.rs:40、resolve.rs:263 | 功能缺口 |

---

## 2. 详细分析

### 2.1 必修项（P0）

**P0-1 eventloop.rs:164 —— 确定可复现的框架级 panic**
`sync_install.rs:214-219` 在 `Force` 模式下**故意保留** `upgradable_version()==None` 的包进入 upgrade_view（注释明说 "With Force every strictly-installed package is reinstalled"），而事件循环渲染 "Upgraded" 列表时 `p.upgradable_version().unwrap()` 必炸。事件循环线程 panic → `update.rs:206 handle.join().unwrap()` 二次 panic → 整个命令崩溃。触发：`hok update -f` 且任一已装包已是最新版。
建议：`unwrap_or(p.version())` 或 `if let Some(v)` 跳过渲染；或 sync_install 在 Force 下也填充 upgradable 引用。

**P0-2 download.rs:1185 —— clippy 失败**
`assert_eq!(0 * chunk_size, 0)` 触发 `clippy::erasing_op`（默认 deny），`cargo clippy` 直接失败。历史报告点名过（erasing_op/identity_op 断言），重构后未修。该断言本身无意义（0×n 恒为 0），改为对 chunk_size 边界的真实断言即可。

### 2.2 高优先级（P1）

**P1-1/P1-2 环境依赖 panic**：`dirs::home_dir()`/`dirs::config_dir()` 返回 None 时 unwrap 崩。包管理器常见于服务账户/精简环境。config.rs:813 影响面最大（Session 创建即崩）。psmodule.rs 已有 `unwrap_or_default()` 先例，照抄即可。

**P1-3 非 UTF-8 容错**：Windows 允许非 UTF-8 文件名（非法代理项），bucket/apps/cache 扫描路径 `to_str().unwrap()` 即炸 `hok list/search/status`。query.rs 的 `filename.to_str().unwrap()` 与 bucket.rs:212 的 `file_type().unwrap()` 是高频路径。建议 filter_map 内 `.ok()?` 跳过条目（manifests() 已有此风格先例）。

**P1-4 可见性落差**：`pub mod internal` + internal/mod.rs 全 `pub mod`（注释自承 "pub 仅为 re-export 所需"，但 Rust 允许 pub(crate) + lib.rs pub use 转发）；package/ 树 25+ 子模块全 pub。真实原因是 CLI 直用 `internal::arch`（P2-2）——修掉 P2-2 后即可真正收口。当前"门面"只是文档性约束。

**P1-5 config.rs 命令操作混入**：`// ─── Session operations` 之后（895 行起）是 config 命令实现：`list_all` 拼三列表格（纯呈现）、`edit` 拉 $EDITOR 子进程、`alias_add/remove`。命令逻辑应在 CLI 层。拆分低风险、直接改善边界，是**最值得拆**的文件。

**P1-6 checkver Args 镜像**：`package/checkver.rs::Args` 与 `src/cmd/checkver.rs` 的 clap Args 逐字段手工拷贝——正是架构原则要消除的"可漂移重复"。checkver 的 Args 应并入 CLI。

### 2.3 中优先级（P2）

- **P2-1** archive.rs 依赖 Event：功能合理（提取进度），但把工具层耦合领域事件。改 `on_progress` 回调或独立进度类型；至少写入规范定论。
- **P2-2** CLI 直用 internal::arch：给 Session 加 `set_default_architecture` 方法或新增 `libscoop::arch` 薄门面，之后 internal 可收口 pub(crate)（与 P1-4 联动）。
- **P2-3** query.rs：匹配引擎（~160 行）/安装状态读取（~110 行）/查询核心/命令操作（suggest/prune）四关注点；`InstallState` 类型在 package/mod.rs 而读取逻辑在 query.rs，结构缺口。拆 query_matcher.rs / install_state.rs / query_ops.rs。
- **P2-4** download.rs：非测试 1167 行中分片机制（~330）、独立下载命令 download_apps（~130，含命令级成功文案应上移 CLI）、共享校验（~170）三块可独立成模块，测试齐全（RangeServer 桩）使拆分有保障。
- **P2-5** 死门面 string（0 引用）；fs/os 与 internal 双路径。
- **P2-6** clippy 警告：6 个可 `--fix` 自动修，其余手动（items after test module、too many arguments 9/7 等）。

### 2.4 明确不拆（子代理 1 结论）

- `shim.rs`（1058 行）：单一职责，620 非测试行全是真实复杂度（双变体嵌入/归属探测/冲突语义），只建议补分区注释 + 处理文件级 `#![allow(dead_code)]`
- `sync_install.rs`（964 行）：已是一次拆分产物，剩余体积是流水线本身
- `internal/archive.rs`（985 行）：分区最规整（10 处注释），internal/ 扁平结构既定

---

## 3. 与历史报告对照（重构验收）

| 历史报告条目 | 现状 |
|---|---|
| package 层 9 处 println/eprintln | ✅ 已清零 |
| dead_code 6 处 allow | ✅ 4 处 |
| 版本比较双实现 | ✅ 已统一 |
| glob_to_regex 位置 | ✅ 已移 internal/string.rs |
| checkver.rs 1716 行 | ⚠️ 已拆 checkver_hash/checkver_url，主体 1468 行 |
| sync.rs 1637 行 | ⚠️ 拆出 sync_install/sync_remove，但 manifest.rs 1612→**1801**、config.rs 862→**1462** 不降反升 |
| clippy erasing_op 断言 | ❌ **仍未修**（download.rs:1185） |
| "items after test module" | ⚠️ 仍在（clippy 警告） |
| cmd 层令牌 env 直读 | ⚠️ 仍在 |

---

## 4. 推荐行动顺序

1. **P0 立即修**（2 项，均为小改动）：eventloop.rs:164、download.rs:1185 → clippy/CI 转绿
2. **P1-1/2/3 panic 容错**（1 批，模式统一：unwrap_or_default / .ok()?）：config.rs:813、shortcut.rs:38、bucket.rs/query.rs/cache.rs 非 UTF-8
3. **P1-4+P2-2 可见性收口**（联动）：Session::set_default_architecture → internal 收口 pub(crate) → 删死门面 string
4. **P1-5 config.rs 拆分**（最值得拆）：命令操作上移 CLI
5. **P2 其余**：checkver Args 镜像消除、query.rs 拆分、download.rs 拆分、clippy 警告清零
6. **规范更新**：领域服务↔package 双向依赖、archive→event 定论写入 AGENTS/技能

> 原则提醒（技能规范）：每项修复先问"解决什么真实问题"——P0/P1 满足（行为不一致/正确性 bug/崩溃），P2 拆分类满足（维护成本/导航成本），避免为拆而拆。

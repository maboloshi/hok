# hok 框架修复执行计划（基于 framework-review-2026-08.md）

> 原则（来自项目技能）：每个修复解决真实问题（崩溃/行为不一致/维护成本）；可漂移的重复才合并；提交必须分组（一主题一 commit）；git 加 `-c core.autocrlf=false`；新代码一次满足 cargo fmt。
> 基线：cargo check ✅ / test ✅（489+11）/ clippy ❌（1 error + 26 警告）。

## 前置：git 工作树现状

- 当前分支 `main`；`Cargo.toml` 有未提交修改（profile.dev 优化 2 行）
- 未跟踪：`.agents/`、`AGENTS.md`、`framework-review-2026-08.md` 及历史报告/脚本（多为工具产物）
- **建议**：开工前先确认这些未跟踪文件的去留（提交/忽略），避免污染后续分组合并的 hunk

---

## 阶段 0：基线恢复（P0，2 项，工作量小）— 让 clippy/CI 转绿

### T0.1 修复 eventloop.rs:164 panic（P0-1）
- **问题**：`update -f` 且存在已最新包时 `p.upgradable_version().unwrap()` 必 panic；`sync_install.rs:214-219` 在 Force 下故意保留 `upgradable_version()==None` 的包（注释与代码矛盾）
- **方案**：渲染时 `p.upgradable_version().unwrap_or(p.version())`；或 `if let Some(v) = ...` 跳过该项。优先前者（保持列表完整）
- **验证**：新增/调整测试覆盖 Force + 已最新包场景；`cargo test -p libscoop` + `cargo build`
- **提交**：`fix(cli): update --force with up-to-date apps no longer panics`

### T0.2 修复 download.rs:1185 clippy error（P0-2）
- **问题**：`assert_eq!(0 * chunk_size, 0)` 触发 `clippy::erasing_op`（默认 deny），clippy 失败
- **方案**：该断言无意义（0×n 恒 0），改为对 chunk_size 边界/分片参数的真实断言，或删除
- **验证**：`cargo clippy --workspace --all-targets` 无 error
- **提交**：`fix(libscoop): clippy erasing_op in download test`

**阶段 0 验收**：clippy 全绿（error 清零）、test 全绿。

---

## 阶段 1：panic 容错（P1-1/2/3，6 项，工作量小-中）

### T1.1 config.rs:813 home_dir 缺失（P1-1）
- `dirs::home_dir().map(|p| p.join(path)).unwrap()` → 对齐 psmodule.rs:87-95 的 `unwrap_or_default()` 先例；无 HOME 时回退相对路径
- **验证**：模拟无 USERPROFILE/HOME 环境（PowerShell 临时删变量）跑 `hok config list` 不崩
- 提交：`fix(libscoop): config default path no longer panics without HOME`

### T1.2 shortcut.rs:38 config_dir 缺失（P1-2）
- `dirs::config_dir().unwrap()` → 与 T1.1 同模式回退
- 提交：`fix(libscoop): shortcut_dir no longer panics without APPDATA`

### T1.3 bucket.rs 非 UTF-8/file_type 容错（P1-3a）
- 位置：bucket.rs:79（to_str）、212/260/261（file_type/file_name）
- 方案：filter_map 内 `.ok()?` / `to_str().ok()?` 跳过脏条目（对齐 manifests() 既有风格）
- 提交：`fix(libscoop): bucket scan tolerates non-UTF-8 entries`

### T1.4 query.rs 非 UTF-8 容错（P1-3b）
- 位置：query.rs:333/447（filename.to_str）
- 方案：同上 `.ok()?` 跳过
- 提交：`fix(libscoop): query/installed scan tolerates non-UTF-8 names`

### T1.5 cache.rs:20 容错（P1-3c）
- `path.file_name().unwrap().to_str().unwrap()` → `.ok()?` 跳过
- 提交：`fix(libscoop): cache listing tolerates odd file names`

### T1.6 shim.rs:44 file_stem fallback（P1-3d）
- manifest 怪 bin 项（如 `["bin/"]`）→ file_stem None → unwrap 崩 uninstall
- 方案：fallback 到 `to_string_lossy()` 全名（alt_filename 只是冲突备份命名）
- 提交：`fix(libscoop): shim removal tolerates path-like bin entries`

**阶段 1 验收**：test 全绿；手动/脚本模拟脏环境（非 UTF-8 目录、无 HOME）不 panic。

---

## 阶段 2：可见性收口（P1-4 + P2-2 联动，3 步，工作量中）— 最高"重构性"步骤

### T2.1 消除 CLI 直用 internal::arch（P2-2）
- 现状：install.rs:100-101、download.rs:37-38 直用 `libscoop::internal::arch`
- 方案：给 Session 加 `set_default_architecture(&self, arch)` 方法（内部委托 internal::arch），CLI 改走 Session；session.rs:434-437 的 apply_default_architecture 顺带归口
- 提交：`feat(libscoop): Session::set_default_architecture; CLI stops using internal::arch`

### T2.2 internal/ 收口 pub(crate)（P1-4a）
- internal/mod.rs 全 `pub mod` → `pub(crate) mod`；lib.rs 仅对 CLI 实际使用的符号做 `pub use` 转发
- **前置**：先收集 CLI 对 `libscoop::internal::*` 的全部引用（grep），确认收口后哪些符号需 pub use（预期仅 arch 相关，T2.1 后可能归零）
- 提交：`refactor(libscoop): internal modules are pub(crate); facade re-exports`

### T2.3 package/ 收口 + 删死门面（P1-4b + P2-5）
- package/mod.rs 25+ 子模块全 pub → 评估哪些需对外（CLI 直用 package::* 70+ 处）——**这一步依赖 T2.2 的引用收集结果**，若 CLI 深路径过多，可先只收 internal（收益大头），package 收口列为后续
- 删死门面 `libscoop::string`（0 引用）
- 提交：`refactor(libscoop): drop dead string facade; narrow package surface`

**阶段 2 验收**：cargo check/test 全绿；CLI 无 `libscoop::internal::` 直用；internal 模块 pub(crate) 生效。

---

## 阶段 3：config.rs 拆分（P1-5，工作量中）— 最值得拆

### T3.1 命令操作上移 CLI
- 从 config.rs:895+ 移出：`list_all`（三列表格）、`edit`（$EDITOR 进程）、`list`/`set` 命令入口、`alias_add`/`alias_remove`
- 归属：`src/cmd/config.rs`（呈现 + 编辑交互）、`src/cmd/alias.rs`（alias 操作）
- 领域保留：ConfigBuilder/Config/ConfigInner/accessors/set/commit/`mod default`
- **验证**：config 10 个测试 + alias/list/edit 命令手动验证
- 提交：`refactor(libscoop): move config command ops to CLI layer`（若 alias 逻辑量大可拆 2 个 commit：config / alias）

**阶段 3 验收**：config.rs 回到领域职责（<1000 行）；test 全绿。

---

## 阶段 4：checkver Args 镜像消除（P1-6，工作量小-中）

### T4.1 checkver Args 并入 CLI
- `package/checkver.rs::Args`（57）与 `src/cmd/checkver.rs` clap Args 逐字段拷贝 → 删除 package 层 Args，CLI 直接定义；package 层保留 `execute`（改签名接收 CLI 解析结果）或领域函数 `check_*`
- 提交：`refactor(libscoop): checkver Args lives in CLI; package exposes domain fn`

**阶段 4 验收**：checkver 15 个测试 + `hok checkver` 命令验证。

---

## 阶段 5：P2 改进（5 项，工作量中-大，可分批）

### T5.1 archive.rs event 依赖定论（P2-1）
- 方案 A（重构）：`extract` 族改 `on_progress` 回调参数（~7 个提取函数签名变更，测试需同步）
- 方案 B（定论）：保留 + 写入规范（internal 允许向 event 单向发进度）
- **建议先定论（B），重构（A）列入 backlog**——符合"不为重构而重构"
- 提交：规范更新入 T6；代码不动

### T5.2 query.rs 拆分（P2-3）
- 拆 `query_matcher.rs`（Matcher 引擎 ~160 行）/ `install_state.rs`（安装状态读取，与 mod.rs 的 InstallState 合并）/ 保留 query.rs 核心；`suggest`/`prune_installed` 评估移 CLI
- 提交：`refactor(libscoop): split query into matcher/install_state/query`

### T5.3 download.rs 拆分（P2-4）
- 拆 `download_fragmented.rs`（分片机制 + RangeServer 桩）/ `download_verify.rs`（verify 族）/ `download_apps.rs`（独立命令 + 成功文案上移 CLI）
- **风险**：2004 行含 19 测试 + RangeServer 桩，移动量大；建议拆前先跑 `cargo test -p libscoop` 锁定基线
- 提交：2-3 个 commit（每模块一个）

### T5.4 clippy 警告清零（P2-6）
- `cargo clippy --fix` 自动修 6 个 → 手动修剩余（items after test module、too many arguments 9/7、struct update no effect 等）
- 提交：`chore: clippy warnings cleanup`（或按文件拆分）

### T5.5 分区注释补齐（P3-4）
- shim.rs / manifest.rs / sync_install.rs 补 `// ───` 分区注释（与既有风格统一）
- 提交：`docs(libscoop): section comments for shim/manifest/sync_install`

---

## 阶段 6：规范与收尾（工作量小）

### T6.1 技能/规范更新
- 记录：领域服务↔package 双向依赖（无环）为已接受设计；archive→event 定论（T5.1 结果）；shim 三处命名重叠说明
- 更新技能：`hok-code-organization`（目录结构若因 T3/T5 变化）、`scoop-gap-audit`（若 T4 顺带修了 --throw）
- 提交：`docs: update project conventions`

### T6.2 更新 framework-review-2026-08.md 状态
- 逐项标记 ✅/⏸（与技能 gap-audit 同风格）

---

## 执行顺序与依赖

```
阶段0 ──▶ 阶段1 ──▶ 阶段2 ──▶ 阶段3 ──▶ 阶段4 ──▶ 阶段5 ──▶ 阶段6
  P0       P1-1/2/3    T2.1→T2.2→T2.3   T3.1     T4.1      T5.x     收尾
  (2项)     (6项)      (3步联动)        (1项)     (1项)     (5项)
```

- **强依赖**：T2.2/T2.3 依赖 T2.1；T5.3 建议在 T3/T4 之后（避免同期大改 download + config + checkver）
- **可并行**：阶段 1 各 T 独立；T5.4/T5.5 随时可插
- **风险点**：阶段 2（可见性收口）改动面最大——若 CLI 对 package::* 深路径过多，T2.3 收 package 可能牵涉 70+ 处，届时收缩范围（只收 internal）或降级为文档性约束

## 每阶段验收标准（通用）

1. `cargo build --workspace` ✅
2. `cargo test --workspace` ✅ 全绿
3. `cargo clippy --workspace --all-targets` ✅ 零 error（阶段 0 起）；警告只减不增
4. `cargo fmt --all -- --check` ✅（新代码一次到位，不动整包既有 diff）
5. 提交分组：一主题一 commit；`git -c core.autocrlf=false`；`git diff -w` 确认无行尾噪音

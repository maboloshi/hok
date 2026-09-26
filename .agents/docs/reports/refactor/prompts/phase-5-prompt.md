# hok 重构阶段 5 开工提示词（新会话）

> 用法：新开对话后，把本文件整段贴给 agent。本文件由上一会话（2026-08-22）生成，配合 `.agents/docs/reports/refactor/plan.md` 使用。
> 分支：`refactor/edition-2024`（阶段 0-4 已完成，阶段 4 的 5 个 commit 均已 GPG 签名 G）。
> 阶段 4 收官现状：libscoop 已全面 async（Network/Git/Archive trait async + reqwest + buffer_unordered/tokio::spawn 并发），`Session` 变 `Sync`，tokio runtime 已接入，本地快操作保持同步。

## 任务
继续 hok 重构计划的**阶段 5：护栏**——三块：
1. **`unsafe_code = deny`**（除 hok-shim / 必要 FFI）：把能 deny 的 crate 全部 deny，必要 FFI（Win32）集中到少数模块局部豁免
2. **criterion 基准**：给关键纯函数加基准护栏，防重构回归
3. **vs Scoop 脚本基准**：对同一批真实命令分别用 hok 和官方 Scoop 跑，验证行为等价 + 性能不劣化
每步保持 `cargo test --workspace` 全绿，按分组提交规范逐主题提交。

## 背景与现状（阶段 0-4 已完成）
- 阶段 0/1：edition 2024 全量升级（已完成）
- 阶段 2：domain 纯化（Package/Manifest/Config/Version 零 I/O 零 async 零框架依赖）
- 阶段 3：infra trait 化（network/git/archive 抽 trait + 内存 fake，单测离线化）
- 阶段 4：operations 异步化（已完成，5 个 commit）：Session Sync（`3a8f08c`）、bin tokio runtime（`94713b3`）、Network trait async + reqwest（`931b627`）、Git trait async + bucket（`e87211a`）、Archive trait async + install/remove 管线（`6744129`）；checkver 用 `buffer_unordered`、download 分片并行、bucket update 用 `tokio::task::spawn` 并发；checkup/search/list/status/which/config/cat 等本地快操作保持同步
- 工作树干净（仅 `.agents/docs/`、`AGENTS.md`、`scripts/` 等未跟踪文件，不入库）；阶段 4 的 5 个 commit 均为 G 签名

## 开工必读
1. `.agents/docs/reports/refactor/plan.md`（阶段路线/风险保障/执行日志；完成后更新日志）——阶段 5 状态 ☐
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`、`local-scoop-source-path`、`no-patchwork-fixes`、`hok-report-archive`（新会话先查根目录报告类文件）
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`，需要核对语义/做脚本基准时直接读
4. gpg 签名：提交前先 `$env:HOME=$env:USERPROFILE`，正常 `git commit`（commit.gpgsign 全局已 true）；不要用 `-c commit.gpgsign=false` 绕过

## 阶段 5 目标与验收
- [ ] `unsafe_code = deny` 落地：能 deny 的 crate 全部 deny；必要 FFI（Win32）集中在少数模块局部豁免；审计清单记录每处 unsafe 的用途与分类
- [ ] criterion 基准覆盖关键纯函数（compare_versions / glob_to_regex / hash / manifest parse / url 工具等），有 `cargo bench` 可跑
- [ ] vs Scoop 脚本基准：对同一批命令（install/checkver/download/status/list 等）分别用 hok 与官方 Scoop 跑，行为等价 + 性能不劣化，有可复现的基准脚本
- [ ] 每步 `cargo test --workspace` 全绿；最终全量 build+test 全绿 checkpoint
- [ ] 行为等价：护栏不改变任何命令语义/输出/错误/幂等

## 现状盘点（已扫描，新会话可直接引用）
### unsafe 分布（grep `unsafe` 实测）
| 文件 | unsafe 处数 | 性质 |
|---|---|---|
| `crates/hok-shim/src/lib.rs` | 47 | no_std shim，Win32 FFI（本质是 shim 生成器） |
| `crates/hok-shim/src/bin/hok-shim.rs` | 3 | console shim，FFI |
| `crates/hok-shim/src/bin/hok-shim-gui.rs` | 3 | GUI shim，FFI |
| `crates/libscoop/src/internal/os.rs` | 15（含 4 个 extern block） | Win32 API（is_admin/ps_command/CreateProcess 等） |
| `crates/libscoop/src/permission.rs` | 3 | Windows ACL（GetNamedSecurityInfoW/SetEntriesInAclW/SetNamedSecurityInfoW） |
| `crates/libscoop/src/internal/env.rs` | 1 | 环境/进程 FFI |
| `crates/innospect/src/lib.rs` | 1 | 已被 `unsafe_code = deny` 覆盖（见下） |
**注**：innospect 的 `Cargo.toml` 已有 `[lints.rust] unsafe_code = "deny"`（连同 missing_docs、clippy unwrap/panic 等强 lint），其 `src/lib.rs` 的 1 处 `unsafe` 是**注释/文档字样**或 `cfg_attr(test, allow(...))` 逃逸——需在阶段 5 复核确认是 deny 生效的（grep 匹配到 `unsafe` 字样不一定是真 unsafe 代码）。

### 基准设施现状
- **无 criterion**：workspace 与各 crate 的 Cargo.toml 均无 `criterion` 依赖，无 `[[bench]]` 配置，无 `benches/` 目录
- libscoop 已有 2 个 **ignored 的临时 bench 测试**：`internal::archive::tests::bench_git_7z_extract`、`bench_progress_overhead`（`#[ignore]`，可参考其思路或迁到 criterion）
- 编译/发布 profile：root `[profile.release]` 用 `lto="fat"`、`opt-level="z"`、`panic="abort"`、`strip="symbols"`、`codegen-units=1`；基准要关注这个 profile 下的表现

## 阶段 5 关键决策（先出计划再动手）
### 决策点 1：unsafe_code = deny 的粒度（本阶段核心架构决策）
`unsafe_code = deny` 分两种落地方式，二选一并写进计划：
- **方案 A（推荐）**：能 deny 的 crate 在 `Cargo.toml` 设 `[lints.rust] unsafe_code = "deny"`，对必要 FFI 的模块用 `#[allow(unsafe_code)]` **模块级局部豁免**（库层零全局 allow，FFI 集中在 os/permission/env 少数模块）。适用：root hok、libscoop（os/permission/env 豁免）、hok-i18n-derive（proc-macro，零 unsafe）。
- **方案 B**：hok-shim 因 no_std shim 本质就是 FFI（47 处），在 `src/lib.rs` 顶部 `#![allow(unsafe_code)]`（crate 级）并注明理由；其余 crate 同方案 A。
**无论如何都必须先做**：逐一审计每处 unsafe 的用途，分类为「可消除（用安全 API 替代）」「必要 FFI（保留 + 局部豁免）」「误报（注释/文档）」——这是 deny 的前提。

### 决策点 2：criterion 基准的 async 处理
阶段 4 后 libscoop 已 async，基准两类函数分开处理：
- **纯函数**（compare_versions / glob_to_regex / hash / manifest parse / url 工具）：直接 criterion `bench_function`，无 runtime
- **async 函数**（checkver / download 等，若要基准）：在 bench 内用 `tokio::runtime::Runtime::block_on` 包一层，或用 `#[tokio::main]` 的 bench；只测纯逻辑部分（如 `extract_version`、manifest 解析），网络 I/O 用 fake 或只测 CPU 部分

## 建议实施步骤（每步一个 commit，先出计划再动手）
1. **unsafe 审计**：grep 所有 `unsafe`，逐处确认用途并分类（可消除/必要 FFI/误报）→ 产出审计清单（可放 .agents/docs/ 或代码注释）
2. **可消除的 unsafe 消除**：能用安全 API/`safe` 包装替代的优先消除（若有无谓 unsafe）→ 全绿
3. **unsafe_code = deny 落地**：按决策点 1 对能 deny 的 crate 设 deny + 必要 FFI 模块局部豁免；hok-shim 按方案 A/B 处理 → 全绿（一个 crate 一个 commit，或一个主题）
4. **criterion 基准**：root 或 libscoop 加 `criterion` dev-dependency + `[[bench]]` + `benches/*.rs`，覆盖关键纯函数（compare_versions / glob_to_regex / hash / manifest parse 等），`cargo bench` 可跑 → 全绿
5. **vs Scoop 脚本基准**：写基准脚本（PowerShell），对同一批命令分别用 hok 与官方 Scoop 跑（行为等价对比 + 耗时），输出可复现的报告 → 全绿
6. **收尾**：更新 plan 文档阶段 5 执行日志（含 ✅）；全量 `cargo build`+`cargo test --workspace` checkpoint

## 约束与纪律
- **行为等价**：护栏（deny/基准）不许改变任何命令语义/输出/进度/错误/幂等；发现问题记 gap 清单，别悄悄带进
- 提交规范：一个主题一个 commit（见 grouped-commits-preference）；git 一律 `-c core.autocrlf=false`，eol=lf；提交前设 `$env:HOME=$env:USERPROFILE`（gpg 签名）
- fmt：只修本次改动 hunk（仓库大量文件有既有 fmt diff，**禁整包 fmt**）；clippy 无新增 warning
- **提交前 review**（hok-proactive-review）：官方行为对齐/边界/架构合规/测试/提交内容完整性，逐项过完才 commit
- criterion 是 dev-dependency，不影响发布构建；`unsafe_code = deny` 加在 `[lints.rust]`（非 `[lints.clippy]`），`cargo build`/`test` 都会触发
- 行为锚官方行为：本地 Scoop 源码在 `D:\App\github项目\Scoop\Scoop`

## 已知坑（直接避）
- **innospect 已 deny**：`crates/innospect/Cargo.toml` 已有 `unsafe_code = "deny"`，别重复加；其 `src/lib.rs` 的 1 处 `unsafe` 字样需复核是真代码还是注释（若真代码但 deny 生效会编译失败——现状能编译说明是被覆盖/注释，别误删）
- **libscoop 的 FFI 分布**：os.rs（15 处，含 4 个 extern block）/ permission.rs（3 处 ACL）/ env.rs（1 处）是 Win32 必要 FFI，deny 时用 `#[allow(unsafe_code)]` **模块级**豁免，别在 lib.rs 顶部全局 allow（会让 deny 失去意义）
- **hok-shim 是 no_std**：47 处 unsafe 是 shim 的本质，deny 不现实；方案 B 的 crate 级 allow 要带理由注释；`bin/hok-shim.rs`/`bin/hok-shim-gui.rs` 各 3 处一并处理
- **async 基准**：libscoop 已 async，criterion 基准 async 函数要 `Runtime::block_on` 包一层；纯函数（compare_versions 等）直接测
- **基准环境**：vs Scoop 脚本基准需要真实 Scoop 环境（本地源码路径是只读行为锚，不是可运行环境；如需运行官方 Scoop 得用实际安装或 `scoop` 命令），基准脚本要能跳过不可用环境而不报错
- **criterion 的 bench profile**：root `[profile.release]` 是 `panic="abort"`，criterion bench 默认用 `[profile.bench]`（继承 release 需注意 panic=abort 对 criterion 的影响）；必要时为 bench 单独配 profile
- **gpg 签名**：本沙箱 gpg 需 `$env:HOME=$env:USERPROFILE`；校验 `git log --format="%h %G? %s"` 全 G
- 已知 flake：`internal::git::tests::pull_fast_forward` / `pull_merge_commit` 在并行跑时偶发失败（libgit2/环境抖动，isolated 重跑即绿）；不要把它当回归去修

## 完成时汇报
- 提交清单（commit hash + 主题）+ 每步全绿证据
- `unsafe_code = deny` 落地：哪些 crate 已 deny、哪些模块局部豁免（含豁免清单）、审计发现的「可消除」unsafe 是否消除
- criterion 基准：覆盖了哪些函数、`cargo bench` 实测数字（对比基准前/后如有）
- vs Scoop 脚本基准：基准脚本位置、对比结果（行为等价 + 性能差异）、不可用环境的跳过说明
- 遗留/已知差异（gap 清单，不悄悄放过）
- 更新 `.agents/docs/reports/refactor/plan.md` 阶段 5 执行日志（含状态 ✅）
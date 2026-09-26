# 提示词：hok 输出系统重设计 — 阶段 3b（源头发射门控 + 进度节流）

> 本提示词自包含，新会话无需上一个会话上下文。目标：**实施并提交 hok 输出系统重设计的阶段 3b**，或先给出带证据的方向推荐（视下述开放问题而定）。仓库：`D:\App\github项目\Scoop\hok`（Rust 2021，五层单向依赖，渲染只在 CLI 层；libscoop 只发 Event/Output、不写 stdout）。

---

## 一、项目背景与已完成阶段

「hok 输出系统重设计」目标是重构 CLI 输出为「单渲染器、单风格方案」。总方案见 `.agents/docs/active/spec/output-system-design.md`。已完成三块：

1. **阶段① 渲染层**（`b532f4d`）：单渲染器 + 彩色语义表 + 八个输出原语（section/plan/gate/action/row/done/fail/notes）+ 计数行 + -v/-vv 单轴；砍掉 output-style 双风格、--detail、DETAIL_MODE、output::detail()。
2. **阶段② 确认闸门**（`8076f49`/`4f58c0d`）：三层可关闸门（--confirm 强制开 / -y--yes 强制关 / config confirm 全局默认 / TTY 内置），计划块永远打印，只关「等 Y/n」。
3. **阶段③a 逐包计数与收尾**（`921b74b`）：PackageCommitStart/Done 事件携带 (k/N) 的 name+index+total；新增 PackageCommitFailed(name,index,total,error)；sync_install/sync_remove 的 commit_one_* 线程化 index/total 并在提交失败时发 Failed；output.rs 新增 closure_done/closure_fail 原语（(k/N) ✓ pkg / ✗ pkg (err)）；scoop_handler 渲染逐包收尾。

当前分支：`refactor/edition-2024`（注意：skill 记载仓库是 edition 2021，但当前分支名如此，以实际 `rustfmt` 版本为准）。

## 二、阶段 3b 目标（来自设计文档 §7、§10 阶段③）

3b = **源头发射门控与进度节流**：

- 源头发射门控：低 verbosity 不构造/发送细粒度事件（粒度事件默认不发，-v/-vv 才注入）。
- 进度事件节流：高频进度事件在源头限频，避免打满 bounded 事件总线。
- 惰性格式化：事件构造时不做昂贵的格式化，渲染时才格式化。

设计文档 §7 verbosity 与颗粒度表：
| 级别 | 漂亮输出 | 日志 |
|---|---|---|
| 默认 | 最小 | ERROR |
| -v | 不变 | INFO |
| -vv | 不变 | DEBUG（排错） |
| -vvv | 不变 | TRACE |

## 三、当前技术现状（已调研，新会话直接采信）

1. **下载进度节流已有**：`crates/libscoop/src/package/download.rs` 的 `ProgressThrottle`（100ms，`PROGRESS_THROTTLE`）+ `send().await` 背压 + `try_send` 丢满即丢。分片/单流两条路径都已接。
2. **细粒度事件目前无条件发射**：extract（`internal/archive.rs` 及 `package/operations/extract.rs`）、shim（`shim.rs` 的 PackageShimAdd/RemoveProgress）、shortcut（`shortcut.rs` 的 PackageShortcutAdd/RemoveProgress）等逐文件 Progress 事件**无条件 try_send**，未做 verbosity 门控。
3. **CLI 端已路由**：`src/scoop_handler.rs` 里这些细粒度 Progress 事件（shim/shortcut/env/persist/psmodule 等）已被路由到 `tracing::debug!`，非 TTY 不显示。但**发射端仍无条件构造+发送**，是浪费点。
4. **verbosity 尚未下沉到 libscoop**：当前 verbosity 是 CLI 层概念（`-v/-vv/-vvv` 走 tracing 过滤），libscoop 的 `Session`/事件发射点不知道 verbosity。3b 若要做「源头发射门控」需先把 verbosity（或一个 emit 开关）暴露给 libscoop。
5. **Session 已 Sync**（阶段④重构 `3a8f08c`），emitter 用 `session.emitter()`（`pub(crate)` 返回 `Option<mpsc::Sender<Event>>`）。

## 四、待决策的开放问题

1. **verbosity 下沉方式**：给 `Session` 加 verbosity 状态（类似已有的 `force_confirm` 模式），还是加一个 `EventEmit` 开关？或维持 CLI 端过滤、只做发射端惰性格式化？各权衡（架构合规：五层单向依赖，libscoop 不能依赖 CLI 的 verbosity 类型）。
2. **门控粒度**：只门控 extract/shim/shortcut 逐文件 Progress（高频、已被 CLI 路由到 tracing 的），还是所有细粒度事件？低频事件（Start/Done/Commit）不受影响。
3. **惰性格式化**：现在 `try_send(Event::Xxx(format!(...)))` 在发射点就地格式化。是否改成传结构化数据、渲染时才 format？这涉及事件变体签名改动（3a 刚把 PackageCommit* 改成 struct 携带 name/index/total 即结构化，可参考）。
4. **是否先做性能基准**：在没有真实性能数据前，「门控所有细粒度事件」是否值得？还是只对已被 CLI 丢弃（非 TTY）的事件做最小门控？
5. **3b 与 3a 的事件演进**：3b 是否会改 PackageCommit* 等已定型事件的签名？尽量不破坏 3a 刚确立的结构化形态。

## 五、约束（必须遵守）

- 五层单向依赖：CLI 层做参数/事件循环/输出渲染；libscoop 只发 Event/Output；内部 `internal/` 只 pub(crate)，禁依赖 package/或领域服务类型（archive→event 是既有唯一例外）。
- 单渲染器、单一风格；颜色沿 `force_color_output`（尊重 --no-color / NO_COLOR）；i18n 沿 rust-i18n（`locales/en.yml`、`locales/zh.yml`）。
- 输出原语收敛至八个；每行输出符合「包名 + 动作 + 结果」语义行规范，禁纯状态行。
- `cargo build` + `cargo clippy --workspace --all-targets`（0 新 warn；archive.rs/manifest.rs 有 3 条既有债务不动）+ `cargo test --workspace` 全绿。
- 改动文件 `rustfmt --edition 2021 --check` 自检（仓库有既有 fmt diff，**禁整包 fmt**，只修自己改的文件；sync_install.rs/sync_remove.rs 有既有 debt 不要顺带改）。
- 提交前做整体 review（`hok-proactive-review`）：官方行为对齐、边界/失败路径/幂等、架构合规、测试覆盖、提交内容完整，发现问题先修再提交，一次成型。
- 提交必须分组签名（`git -c core.autocrlf=false`，`commit.gpgsign=true`，GPG key maboloshi）；一个主题一个 commit，用 `git apply --cached`/`git add` 拆 hunk。
- 不为重构而重构、不为去重而去重：先问「解决什么真实问题」，答不上就不做。

## 六、交付物

1. 对第四节开放问题给出带证据的方向推荐（引用代码现状与设计文档）。
2. 若确认要落地：给出改动方案 → 实现 → 验证（build/clippy/test/fmt）→ 整体 review → 分组签名提交。
3. 更新 `.agents/docs/active/spec/output-system-design.md` 的进度标记（§10 阶段③ 勾掉 3b）与 `.agents/docs/devlog/README.md`。
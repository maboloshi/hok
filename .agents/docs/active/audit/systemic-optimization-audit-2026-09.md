# hok 系统性优化点审计（结构 / 代码冗余 / 逻辑冗余 / crates / feature）

Status: active
Updated: 2026-09-29

> **这是什么**：对 hok 五个维度（系统性结构、代码冗余、逻辑冗余、外部 crate 替代与剪裁、feature 裁剪）的
> 现状盘点与优化点清单。**只做盘点与建议，未改任何生产代码**（`investigate-first`：先报告、等拍板）。
> **取代**：`hok-overall-framework-analysis.md`（同一目录，结论已被本文取代，见 §0）。
> **基线**：分支 `refactor/edition-2024`，tip `b632371`（2026-09-29）。
> **复跑口径**：§10 给出每条结论的可复跑命令。

---

## 0. 为什么重做：上一版框架报告的失真清单

`hok-overall-framework-analysis.md`（2026-09-26）广度与深度都不足，且部分内容已过期。逐条更正：

| # | 上一版说法 | 实测（本次） |
|---|---|---|
| 1 | 工具层 `internal/` 含 `network`、`Archive`、`Git`；「核心纯类型」在 `package/` 底层 | 五层描述含混：把 `internal/` 说成「工具层」却又把 `package/` 同时列为「领域服务」和「核心纯类型」两层。实际分层与目录见 §1 |
| 2 | 双入口 = `scripts/build-hok.ps1` + 根 `build.rs`，两处各自检查 winsqlite3，引用「`build-hok.ps1` 线 286-304 / `build.rs` 线 88-117」 | **行号全部失效**：winsqlite3 断言在 `crates/libscoop/build.rs:25-118`；根 `build.rs`（75 行）只做 `BUILD_DATE`、`/STACK:` 与 cmd 模块注册，与 sqlite 无关。`build-hok.ps1` 是**构建编排脚本**（staging + 环境变量 + `-Verify`），不是「重复入口」——两者职责不同，F1 的收益被高估 |
| 3 | 「已固化：`crates/innospect` 内联 fork 已删除（2026-09-19）」 | 正确，但同一份报告 §1 表格仍把 `crates/innospect` 当作现存单元描述，**内部自相矛盾** |
| 4 | F2 声称「代码 11 处 `.buffer_unordered()`」 | **实测 4 文件 8 处 `futures::` 引用**（`network.rs`、`checkver.rs`、`download.rs`、`download_fragmented.rs`），其中 `buffer_unordered` 仅 4 处。数量是拍的不是数的 |
| 5 | F3「任务批处理泛化」、F4「事件/进度框架重构」只给了「未做」，没有给出**问题陈述与影响面**，也没有量出事件系统实际规模 | §1.3 量出：`Event` **87 个变体**、`scoop_handler.rs` 265 行穷举匹配、约 28 个变体的唯一消费者是 `tracing::debug!` |
| 6 | 完全未覆盖 | 代码冗余（精确/近似克隆）、逻辑冗余（重复决策与单入口旁路）、`feature` 裁剪、五层架构的**违规点**、公共 API 面与 CLI 用量的比例——即用户点名的三个维度全部缺失 |

**结论**：上一版是「框架名词复述 + 5 条未执行建议」，本文以实测替换之；上一版应归档。

---

## 1. 系统性结构

### 1.1 基线与规模（实测）

| 指标 | 值 | 来源 |
|---|---|---|
| 生产行（首处 `#[cfg(test)]` 之前） | **41,032** | `loc.mjs`（TEMP 脚本，§10） |
| 测试行 | **22,880**（占比 35.8%） | 同上 |
| `.rs` 文件数 | 136（`src` 48 / `libscoop` 97 / `hok-shim` 1 / i18n 2） | `Get-ChildItem -Recurse -Filter *.rs` |
| workspace 依赖闭包（Windows, normal+build） | **255** 个 crate | `cargo tree -e normal,build --target x86_64-pc-windows-msvc` |
| `cargo metadata --all-features` 节点 | 330 节点 / 313 名 | `cargo metadata` |
| 多版本同名 crate | 8 组（`bitflags`/`hashbrown`/`hashlink`/`linux-raw-sys`/`openssl-probe`/`rustix`/`syn`/`windows-sys`） | 同上 |
| `cfg(feature = …)` 门控 | **0 处**（全仓源码） | §5 |
| 二进制体积 | `winsqlite3` 形态 7,084,544 B（6.76 MiB）；bundled 8,124,928 B | [`binary-size-analysis-2026-09.md`](binary-size-analysis-2026-09.md) |

### 1.2 结构问题 S1：外部 4 crate 划分是必要的，但 `hok` 自身是 lib+bin 双态

四个 crate 的**存在理由均成立**，不建议合并：

| crate | 存在理由（实测） |
|---|---|
| `crates/hok-shim` | `no_std` + Win32 FFI；两个 bin 只差 `#![windows_subsystem]`（`hok-shim.rs:16-34` ↔ `hok-shim-gui.rs:14-32`，16 行完全相同）——**这是 PE subsystem 的必然**，不能合并成一个 bin |
| `crates/hok-i18n` | 依赖图里的**单点**：`build.rs`（140 行）把 `locales/*.yml` 展平成查找表，CLI 与 libscoop 共用同一份内嵌表。若并回任一消费方就会出现第二份内嵌 |
| `crates/hok-i18n-derive` | proc-macro，只能独立 crate |
| `crates/libscoop` | 领域库；`package/` 有 4.6 万行 |

**真正的结构问题在根 crate 的双态**：`Cargo.toml:42-43` 声明 `[lib] src/lib.rs`，而 `src/lib.rs`（221 行）装的是 CLI 侧管道（`create_app`/语言探测/日志初始化），不是可复用 API。它的唯一实际消费者是
`tests/*.rs`（8 个集成测试）与 `cmd/` 自身。这本身不是缺陷，但它使「CLI 层」在目录上无法一眼区分（`src/cmd/` 是命令，`src/{output,cui,eventloop,i18n,format,scoop_handler}.rs` 是同层管线）。**判定：可不动，但需要在 `ARCHITECTURE.md` 里把这一层写清楚，否则每次新增 CLI 侧文件都要重新判断归属。**

### 1.3 结构问题 S2（最重）：事件模型 87 变体 / 两处分发 / 约 1/3 变体只喂 `tracing::debug!`

实测：

| 量 | 值 | 证据 |
|---|---|---|
| `Event` 枚举变体 | **87** | `crates/libscoop/src/event.rs`（519 行） |
| `Start` / `Done` / `Progress` 命名对 | 18 / 23 / 10 | 同上 |
| CLI 侧分发点 | **2 处，风格不同**：`src/eventloop.rs:102-244`（10 臂 + `_ => {}`）、`src/scoop_handler.rs:21-280`（穷举 265 行） | 两文件 |
| `scoop_handler` 中的空臂（`=> {}`） | **12 处**（如 `:27`、`:33`、`:39`、`:103`、`:111`、`:125`、`:133`、`:155`…） | 同上 |
| 只被转发到 `tracing::debug!` 的变体 | **约 28 个**（shim/shortcut/env/persist/psmodule/symlink 的 Start/Done/Progress 全部） | `src/scoop_handler.rs:104-200,257-262` |

**为什么这是结构问题而不是风格问题**：

1. **同一事件总线两种消费者协议**：`eventloop.rs` 用 `recv().await` 的 `match`，`scoop_handler.rs` 用 `handle(session, event)` 的穷举 `match`；同一个 `Event::PackageDownloadProgress` 在两个文件里各有一条臂（`eventloop.rs:127`、`scoop_handler.rs:280` 只 `{}`）。新增事件的"该加哪"没有规则。
2. **信息含量与管道成本倒挂**：约 1/3 变体的载荷只是为了拼一行 debug 文本，而生产侧要为它分配 `String`、`try_send` 进容量 20 的 channel（`constant.rs:22` `EVENT_BUS_CAPACITY = 20`）。仓库自己已经为这个成本写了基准：`crates/libscoop/benches/event_emission.rs:1-13` 的用意即「判这笔浪费是否 material」，但**从未据此做决定**。
3. **配对无不变式**：`Start`/`Done` 是 18/23 个独立变体，没有任何机制保证配对；`PackageShortcutAddStart` 等 12 个空臂说明配对本身已无意义。

**候选方向（互斥，需拍板）**：
- (a) 保留 `Event` 语义变体，但把 `Start`/`Done`/`Progress` 三件套折叠成 `Event::Step { kind: StepKind, phase: Phase, ctx }`，变体数从 87 降到约 25，空臂与配对问题一起消失；代价是 `StepKind` 需要覆盖现有语义（枚举改名，CLI 与 libscoop 同步改）。
- (b) 把「只喂 debug 的 28 个变体」改为 `tracing::debug!` 直接在 libscoop 发射（`HOK_LOG_LEVEL` 已存在，`src/cmd/mod.rs:751-753`），事件总线只保留有 UX 语义的事件。代价：库层直接依赖 tracing 输出（当前 libscoop 生产代码只有 5 处 `tracing::warn!`，其余 debug 全在 CLI），需确认这是否破坏「库不直接写输出」的设计线。
- (c) 不动。

**推荐 (a)**，理由：一次消除「变体数量 × 两处分发 × 配对不变式」三个问题，且不改变 UX 语义；(b) 动摇了库/CLI 的输出边界（`libscoop/src/lib.rs:16-38` 明确写了库不直接写 stdout）。

### 1.4 结构问题 S3：公共 API 面 627 项，CLI 只用到 45 条路径 / 10 个 `Session` 方法

| 量 | 值 | 证据 |
|---|---|---|
| libscoop 公开项（`pub fn/struct/enum/trait/const/static`） | **627**（其中 `pub fn` 474） | `Select-String '^\s*pub (async )?fn \|…'` |
| `src/**` 实际引用的 `libscoop::` 路径 | **45**（去重） | `libscoop::[A-Za-z_:]+` |
| `src/**` 实际调用的 `session.xxx` | **10** | `session\.\w+` |
| 最"胖"的公开模块 | `config.rs` 46 pub / `package/manifest.rs` 78 pub / `session.rs` 37 pub | 逐文件计数 |

`crates/libscoop/README.md` 与 crate 文档（`lib.rs:1-13`）把 libscoop 定位为「可供他人实现 Scoop 前端」的库，
所以公开面**有其存在理由**；但当前状态是「所有领域服务模块都 `pub`」，而 CLI 只从中取了极小一片。
可以收窄的明显候选（**先问是否愿意保留对外兼容承诺**）：

- `package/mod.rs:36-71` 把 34 个子模块中 24 个声明为 `pub`，其中 `formatjson`/`export`/`import`/`virustotal`/`missing_checkver` 等是**纯 CLI 功能**（无第三方复用场景），可降为 `pub(crate)` + 由 `Session` 暴露。
- `internal/` 已全部 `pub(crate)`（`internal/mod.rs`），收口已完成——这一层不是问题。
- 反例（不要动）：`output::Output`、`Event`、`Session`、`QueryOption`/`SyncOption`、`hash`/`url`/`os`/`fs`/`string` 薄门面 —— CLI 与 `tests/` 都在用。

### 1.5 结构问题 S3b（致命）：libscoop 无法脱离本仓独立使用

**结论先说：`libscoop` 今天只在"本仓 + 已构建过"的条件下可用。** 第三方以任何方式（crates.io / git / 裸 path）
引用它，都拿不到一个能构建的 crate；而**全新 clone 里连 `cargo build -p libscoop` 都失败**。
三个阻断点全部实测，且互不重叠——修掉一个还有下一个。

**B1 —— 路径依赖没有版本号，`cargo package` 连清单校验都过不去（实测）**

```
$ cargo package -p libscoop
error: failed to verify manifest at `crates/libscoop/Cargo.toml`
  all dependencies must have a version requirement specified when packaging.
  dependency `hok-i18n` does not specify a version
```
- 站点：`crates/libscoop/Cargo.toml:48`（`hok-i18n = { workspace = true }`，而 workspace 声明是
  `Cargo.toml:26` 的纯 path）、`:95`（`hok-shim = { path = "../hok-shim" }`）。`crates/hok-i18n/Cargo.toml:11`
  还有 `publish = false` —— 意味着即使给 libscoop 补上版本号，依赖也拿不到。
- 性质：Cargo **不支持**发布带无版本 path 依赖的包（crates.io 上会解析成"同名任意版本"，必须显式约束）。

**B2 —— locales 在 crate 之外，`hok-i18n` 自己就不可打包（实测 + 代码）**

```
$ cargo package -p hok-i18n --list
.cargo_vcs_info.json  Cargo.lock  Cargo.toml  Cargo.toml.orig  build.rs  src/lib.rs
```
- 打包产物**只有 6 个文件**，没有 `locales/`——因为 `locales/{en,zh}.yml` 在仓库根，
  而 `crates/hok-i18n/build.rs:33` 写的是 `manifest_dir.join("../../locales")`。
- 推论（**由两个已实测事实推出，非直接实测**）：该 build script 对缺失目录会
  `panic!("hok-i18n: cannot read …")`（`build.rs:37-38`）⇒ 发布出去的 `hok-i18n` 无法构建。
- 同一个 `../../locales` 也是**仓库内**的隐含约束：把 `crates/hok-i18n` 单独拷出仓库即失效。

**B3 —— 嵌入式 shim 依赖"本仓 target/ 里已有的产物"，干净克隆直接 panic（实测）**

```
$ cargo build -p libscoop          # 把 target/ 移走，模拟全新 clone
thread 'main' panicked at crates\libscoop\build.rs:204:
  hok-shim binary not found at <repo>\target\release\hok-shim.exe
    Run `cargo build -p hok-shim` first.
exit=101
```
- 根因：`crates/libscoop/build.rs:157-229` 需要 `hok-shim.exe` / `hok-shim-gui.exe` 的**二进制**来
  `include_bytes!`，而 `candidate_target_dirs`（`:239-250`）的候选里**包含 workspace `target/`**。
- 关键佐证（反证）：把 `CARGO_TARGET_DIR` 指向一个空目录再构建**会成功**（实测 2m24s）——
  因为它在最后回退到本仓既存的 `target/release/hok-shim.exe`。也就是说"构建成功"这件事
  完全依赖本仓曾经构过 shim，与目标目录无关。
- `crates/libscoop/Cargo.toml:94-95` 的 `[build-dependencies] hok-shim` **是空壳**：`build.rs` 里
  没有任何 `hok_shim::` 引用（grep 仅命中注释与 `"hok-shim"` 字符串字面量）。它大概被写来
  "让 cargo 先构建 shim"，但**构建脚本之间没有顺序保证**，实际拿到二进制靠的是 `target/` 探测。
  这使真正的依赖是隐式的文件系统探测，而不是 manifest 里的声明。

**B4 —— 文档仍在承诺第三方可用（与实现矛盾）**

| 位置 | 承诺 | 与实现的关系 |
|---|---|---|
| `crates/libscoop/Cargo.toml:5` | `documentation = "https://docs.rs/libscoop"` | 指向上游旧版（crates.io 上 libscoop 最新 **0.1.0-beta.7 / 2024-12-10**，`repository` 仍是 `chawyehsu/hok`） |
| `crates/libscoop/README.md:5-15,27-30` | 4 个 crates.io/docs.rs 徽章 + `libscoop = "0.1"` 安装示例 | 本仓版本是 **0.2.0**，从未发布 |
| `README.md:7,10,305-309` | 同样的 crates.io / docs.rs 徽章 | 同上 |

**唯一的"决定痕迹"**是 `crates/hok-i18n/Cargo.toml:9-11`（`c99f07e`，2026-09-11）的一句注释：
> `# Internal to the workspace: hok is the only consumer and libscoop is no longer`
> `# treated as a publishable crate (Windows-only, off upstream).`

但这条决定**只落在了 hok-i18n 上**：`libscoop` 自己没有 `publish = false`，README 与 crate 文档也没有改。
所以现状既不是"完全没考虑"，也不是"已收口"，而是**决定做了、只执行了一半**——这正是它危险的地方：
下一个人读到 README 会以为能用。

**两条出路（互斥，需拍板；按 `investigate-first` 不自行选型）**

- **路线 A｜承认定位：明确"不可独立使用"**（工作量小）
  `libscoop`/`hok` 加 `publish = false`；删掉两处 README 的 crates.io/docs.rs 徽章与
  `libscoop = "0.1"` 安装示例，改为"内部 crate，随 hok 一同构建"；`ARCHITECTURE.md` 写明
  「必须先 `cargo build -p hok-shim`」是**设计约束**而非临时坑。收益：文档与现实一致，且把 B3 登记为
  显式约束，不再被当成 bug 反复踩。
- **路线 B｜真的做成可独立引用**（工作量大，且要动构建架构）
  三件事必须一起做：① `hok-shim` 从 `[build-dependencies]` 变成**真依赖**（在 `build.rs` 里调用它导出的
  函数/常量生成 shim，而不是探测 `target/` 里的二进制），或改为在 build.rs 里现场编译 shim 源码；
  ② `locales/` 搬进 `crates/hok-i18n/locales/` 并把 `build.rs:33` 改成 `manifest_dir.join("locales")`，
  同时给 `hok-i18n` 去掉 `publish = false`；③ 给 `hok-i18n`/`hok-i18n-derive` 补版本号，二者先发布。
  额外代价：`git2`/`rusqlite-bundled`/Windows-only 意味着**只能在 Windows 上构建**，crates.io 的
  docs.rs 也构建不了（docs.rs 是 Linux）——所以"docs.rs 徽章"这条路本身就不通，最多做 git 依赖。

**判定**：路线 A 是**当前架构下唯一自洽**的选择；路线 B 需要一个明确的第三方消费场景作为前提，
且必须连 §1.6 的能力面一起解决——只解决"能构建"而没有默认事件消费者，第三方仍然用不了。

### 1.6 结构问题 S3c（同属致命）：能构建也没用——能力面停在内核，缺"应用外壳"

用户追问："第三方可能拿到很低级 API 需要自己造轮子，可能还没直接调用 hok 方便。"实测成立，
且它比 B3 更难绕：**libscoop 暴露的是内核（操作 + 事件协议），应用外壳全在 CLI 里。**

**证据 1 —— `emitter` 是 `pub(crate)`，第三方只能拿裸事件总线**

`Session::emitter()`（`session.rs:538`）是 `pub(crate)`；库内 28 个调用点全靠它发事件。
第三方能用的公开面是 `EventBus`（`event.rs:71-125`）+ `session.event_bus().sender()`。
也就是说：库把"发事件"这半边对外开放，却把"该不该发"的开关（`emitter()` 的 `Option` 语义）留在内部。

**证据 2 —— 自己驱动事件循环是硬要求，否则会**挂死**

`resolve::select_candidate`（`resolve.rs:269-288`）：先 `try_send(PromptPackageCandidate)`，
`try_send` 成功就 `while let Some(answer) = session.receive_response().await` —— 等回包。
`sync::confirm_transaction`（`sync.rs:380-401`）同构，等不到就
`Err(Error::Custom("event bus closed unexpectedly"))`。
- 若第三方**完全不碰事件总线**：`emitter()` 为 `None` ⇒ 自动批准 / 自动选首个候选（有安全兜底）。
- 若第三方"注册了总线但没驱动"（最容易犯的错）：`try_send` 成功 ⇒ 走进 `receive_response().await`
  **永久等待**。这是可复现的挂死，不是理论风险。

**证据 3 —— 出站 channel 只有 20 格，不消费就静默降级**

`EVENT_BUS_CAPACITY = 20`（`constant.rs:22`）。所有发射点几乎都用 `try_send`（如 `sync.rs:385`、
`resolve.rs:272`、`download.rs:199`），**满了就 `Err` ⇒ 被当作"没有前端"⇒ 静默跳过事件甚至自动批准**。
所以"部分实现事件循环"比"完全不实现"更危险：行为不一致且无报错。

**证据 4 —— 应用外壳在 CLI 里，且规模不小**

| 模块 | 生产行 | 职责（第三方要自己写） |
|---|---:|---|
| `src/eventloop.rs` | 348 | 取 `receiver()`、起 tokio 任务、进度条/光标 RAII、交互提示、`reset_outbound` 二次事务 |
| `src/scoop_handler.rs` | 345 | 穷举 87 个变体，映射到输出 |
| `src/output.rs` | 484 | `Output` sink 与样式渲染 |
| `src/cui.rs` | 452 | 确认框 / 候选选择 / 表格 |
| **合计** | **1,629** | 纯"把内核接成人能用的东西" |

**证据 5 —— 现有的"高层"入口是 CLI 编排，不是库 API**

`install.rs:90-149` 是**薄**的（这一点仓库做得好），但薄在有 1,629 行外壳垫底的前提下：

```rust
let handle = crate::eventloop::run_event_loop_default(session);   // ← 外壳，且不在 libscoop 里
let sync_result = package::sync::sync(session, to_install, options).await;
handle.await.unwrap();
```
`run_event_loop_default` / `run_event_loop` 都在 **CLI crate**（`src/eventloop.rs:76,346`），
libscoop 只提供 `EventHandler` trait（`lib.rs:82`）。第三方要复刻的正是这段编排。

**所以真实的"能力面"边界是**：libscoop 给的是 `query` / `sync` / `status` / `config` / `bucket` 等
**领域操作**（`list.rs:85-92` 那三行就是例子：`query` → `source::classify` → `AppStatus::from_package`
已经是像样的高层返回值）；但**从"调用它"到"跑完不挂死"之间那 1.6k 行外壳没有打包**。

**结论（对"该不该做路线 B"的影响）**

- 若第三方只是要**读状态**（装了什么、能不能升级、bucket 里有什么）：今天的 API 够用，且比调 `hok` 解析文本更靠谱。
- 若第三方要**执行生命周期**（装/卸/update）：必须自己接事件协议，风险是上述两条（挂死 / 静默降级），
  成本约 1.6k 行的等价物。**"直接 shell out 调 hok 并解析输出"在这个场景下确实更省事**——用户这个判断对。
- 要让路线 B 真正成立，缺的不是更多 `pub fn`，而是**库内建一个默认事件消费者**：
  例如 `libscoop::headless`（内部驱动总线、非交互默许、把事件转成回调或 `Output`），
  让"不接前端也能跑完且行为可预期"成为默认而非陷阱。这才是能砍掉那 1.6k 行的东西；
  再加便利入口（如 `install(&session, pkgs, opts)` 一次调用）才有意义。

### 1.7 结构问题 S4：命令注册是双份真相

| 机制 | 位置 | 性质 |
|---|---|---|
| `build.rs` 扫 `src/cmd/*.rs` → 生成 `src/cmd/__cmd_reg__.rs`（`build.rs:41-74`） | 自动 | 只生成 `mod` 声明 |
| `src/cmd/mod.rs` 的 `enum Command`（clap derive 变体） | 手写 | 真正的注册表 |

仓库文档已自觉记录了这点（`AGENTS.md`：「add a cmd = drop a `.rs` with `execute()` **and** add a variant to the `Command` enum」）。
**判定：有意接受，不建议改**——clap derive 的命令表无法从目录扫描生成（每个变体要写参数类型/帮助文案），
把 `enum Command` 换成宏只会把编译期错误换成宏展开难读。

### 1.8 结构问题 S5：CLI 层内嵌 clap 本地化机制（约 130 行）

`src/cmd/mod.rs:454-571` 是一次**针对 clap 内部行为的绕行**：为让中文帮助里的 `Arguments:`/`Options:`
标题本地化，对整棵命令树做两阶段改写，且带硬约束注释「`mut_arg` 只能在 `build()` 之前」。
这段代码已经修过 `checkver`/`reset` 两个命令被 `mut_arg` 破坏的真 bug（注释 `:503-510`、`:534-538`）。

**判定：保留，但把它登记为"高脆弱面"**：任何 clap 升级都要重跑全部帮助面逐字节比对（`.agents/scripts/dump-help.js`、`help-audit.js` 已具备）。这不是可优化点，是**必须持续付费的约束**——值得写进 `ARCHITECTURE.md`，否则下次升级 clap 会重踩。

### 1.9 结构问题 S6：超长文件与超长函数（含测试）

生产行最大的文件（剔除测试块）：

| 文件 | 生产行 | 测试行 | 说明 |
|---|---:|---:|---|
| `package/checkver.rs` | 1,720 | 1,625 | 测试超过生产 |
| `internal/archive.rs` | 1,720 | 1,210 | 格式分派 + 测试嵌进模块尾部 |
| `package/manifest.rs` | 1,644 | 810 | 78 个 pub 项 |
| `config.rs` | 1,485 | 463 | 46 个 pub 项 |
| `package/download.rs` | 1,140 | — | |
| `shim.rs` | 987 | 622 | |

最长函数体（子代理机械测量，`:file:line` 为函数起点）：`sync_install.rs:607 commit_one_install` 369 行、
`auto_pr.rs:85 run_auto_pr` 351、`download.rs:355 download_one_package` 340、`sync_install.rs:88 install` 340、
`checkver.rs:161 execute` 334、`sync_remove.rs:168 commit_one_remove` 323、`scoop_handler.rs:21 handle` 265、
`hok-i18n-derive/src/lib.rs:10 derive_i18n_help` 261、`info.rs:334 build` 231、`bucket.rs:657 update` 198。

**判定**：`commit_one_install`(369) / `commit_one_remove`(323) 是 §2/§3 里多个冗余点的**共同成因**，
拆分收益最高；`internal/archive.rs` 是唯一能按格式（7z/zip/tar/gz/xz/zst/bz2/rar/nsis/inno）清楚切分的模块。
两者都需要先有具体问题陈述（`hok-code-organization`：不为重构而重构）。

### 1.10 构建期结构：embedded-shim 的前置依赖（已并入 §1.5）

`crates/libscoop/build.rs:157-229` 按 `target/{release,debug}` × 候选 target 目录查找 `hok-shim.exe`，找不到就
`panic!("hok-shim binary not found … Run `cargo build -p hok-shim` first.")`。这使**任何新鲜 worktree 的第一次
`cargo build` 必须分两步**（`AGENTS.md` 已记录此坑）。属于已知约束，非本次新增问题；若要消除，选项是
`schema = "cargo:rerun-if-changed"` 之外引入构建顺序（Cargo 不支持 crate 间构建依赖），或让 shim 源码在
build.rs 里现场编译（引入 cc/rustc 调用，代价更大）。**判定：保持现状**。

---

## 2. 代码冗余（机械实测）

**方法**：两个自研检测器（`$TEMP\hokaudit`，非仓库内）：① 规范化后**精确重复连续段**（≥ 8 行，左极大匹配）；
② **近似重复函数**（标识符/字面量占位化 + 4-gram Jaccard ≥ 0.70 且 ≥ 50 token）。两条都按 `#[cfg(test)]`
边界分类，命中的每一处均被人工打开复核（含已剔除的假阳性）。检测器是启发式的，**结论为结构性判断，不是观察到的运行失败**。

### 2.1 总量

- 精确重复段 **141** 处：生产类 42 / 测试类 99。
- 近似重复函数簇：**35** 对（Jaccard ≥ 0.70），几乎全部落在测试。
- `hok-i18n` / `hok-i18n-derive` 源码：**零**精确重复段、零近似重复函数对。
- 测试 22,880 行（占比 35.8%）——**这是测试，不是重复**，但其中确实藏着下面 P7/P8 两类真实的复制粘贴。

### 2.2 生产代码冗余点（按可漂移度排序）

| id | 内容 | 位置 | 可漂移 | 真实问题 |
|---|---|---|---|---|
| **P2/N1** | 架构固定顺序 `64bit → 32bit → arm64` 在**一个文件里手写 4 遍** | `package/manifest.rs:1165`、`:1195`、`:1229-1233`、`:1296-1300` | **是** | `checkurls` 与 `checkhashes` 各自手写遍历顺序；改一处不会编译失败，两边校验的架构集合/顺序会不一致 |
| **P2** | 4 份手写 `SyncArgs` 投影 | `install.rs:73-88`、`update.rs:69-85`、`upgrade.rs:47-62`、`reinstall.rs:38-55` | 是 | 每命令默认值写在字面量里（`upgrade` 的 `no_replace/no_upgrade=false`、`reinstall` 的 `assume_yes=true`）；加字段会逼你改 4 处，但**不逼你改对值** |
| **P3** | `SyncArgs`/`QueryArgs` 携带 clap `#[arg]` 属性，但**从未被 `flatten`** | `shared_args.rs:24-66`、`:188-201`；全仓 `#[command(flatten)]` 仅 1 处（`cmd/mod.rs:254` `Verbosity`） | 是 | 仓库里存在**第二份未被 clap 验证的 flag 拼写表**；读者会把 `shared_args.rs` 当权威 |
| P1 | `Session { … }` 十二字段字面量 3 份 | `session.rs:177-191`、`:198-212`、`:241-255` | 弱 | 新字段会编译失败，但**初始化值**可以三处不同 |
| N2 | `commit_install` ↔ `commit_remove`（J=0.912） | `sync_install.rs:574-600` ↔ `sync_remove.rs:131-160` | 中 | 逐项失败循环 + `Event::PackageCommitFailed{…}` 完全相同，尾部已按需分叉（`sync_remove.rs:154-155` 有注释）——正是未来一次编辑会抹掉的那种差异 |
| N3 | `manifest_matches(…)` 7 参数调用 3 份，其中两个相邻 `bool` | `package/query.rs:129-137`、`:296-304`、`:390-398` | **是** | 交换两个 bool 能编译通过且**静默改变查询语义** |
| N5 | push 模式 / PR 模式各自重写 progress + `get_ref_sha` + 失败上报 | `package/auto_pr.rs:262-278` ↔ `:327-343` | 中 | 修一条分支不会到另一条 |
| P6 | `CreateFileW → 句柄校验 → ReadFile → CloseHandle` 前缀读 2 份（`unsafe`） | `hok-shim/src/lib.rs:348-369` ↔ `:642-658` | 中 | 短读/句柄泄漏的修复只会落在其中一份 |
| N4 | `download_and_hash_multi(…)` 7 参数调用 2 份 | `checkver.rs:1532-1541` ↔ `:1574-1583` | 弱 | 类型不同，交换会编译失败 |

### 2.3 测试侧冗余（不构成生产风险，但成本真实）

| id | 内容 | 规模 | 位置 |
|---|---|---|---|
| **P8/N6** | CLI 集成测试的「spawn `CARGO_BIN_EXE_hok` + 固化 SCOOP/USERPROFILE/HOME/APPDATA/XDG_CONFIG_HOME + `env_remove("HOK_LOG_LEVEL")`」被复制 **7 份，且已经漂移**（`cat_style_bat_cli::run` 返回三元组，其余两元） | 10-20 行 ×7 + 12 处 `env_remove("HOK_LOG_LEVEL")` | `tests/{formatjson_cli,download_stack_overflow,exit_codes,missing_checkver_cli,cat_style_bat_cli,root_env_fallback,cli_help_i18n}.rs` |
| **P7** | `fn plain()`（ANSI SGR 剥离测试 oracle）复制 **9 份** | 10-17 行 ×9 | `src/cmd/{alias:228,bucket:304,cache:234,cleanup:464,config:364,info:334,list:362,shim:306,status:244}.rs` |
| P10 | `session_with_no_junction` 重实现 `test_utils::test_session_with_junctions` | 13 行 | `package/health.rs:145-158` ↔ `test_utils.rs:64-77` |
| P12 | `operations/script.rs` 测试模块内部重复最密（76 段，最长 19 行） | ~792 行 | `script.rs:781+` |

**失败模式（P8 尤其是真问题）**：影响 root/config 解析的新环境变量（如曾经的 `HOK_LOG_LEVEL`）必须
在 7 处同时加；漏一处时，测试会**静默读取开发机真实的 `~/scoop`**，而不是失败。

### 2.4 明确不建议动（已排除）

- `manifest.rs` 的 `ManifestSpec`/`Autoupdate`/`AutoupdateArchSpec` serde 字段镜像（7 段）：声明式 schema 镜像，
  文件内已注明「Same as `ManifestSpec::…`」，且 `hash` 类型**故意**不同。
- `hok-shim` 两个 bin 的 16 行重复：只差 `#![windows_subsystem]`，**不能合并**。
- `src/cmd/{home,prefix}.rs` 头部 8 行、`tests/*` 的模块文档 + `use` 块：零逻辑。
- 测试类近似克隆中的「同骨架改名」型（35 对里的大多数）：风险是"共享 fixture 的 bug 被复制"，不是生产行为，不值得动。

---

## 3. 逻辑冗余（重复决策与单入口旁路）

**方法**：按「一个决定被做了几遍 / designated 单入口是否被绕过」扫描，每处都 grep 全部调用点并按当前
checkout 打开复核；官方语义锚点取自 `../Scoop/lib/install.ps1:235-279`。

### 3.1 重复决策表（14 项，节选并按风险排序）

| id | 决策 | 全部站点 | 状态 | 风险 | 真实问题 |
|---|---|---|---|---|---|
| **R8** | 「是不是 nightly」 | 指定 `Package::is_nightly`（`package/mod.rs:383`，**生产零调用**）vs 裸比较 `download.rs:253`、`download.rs:285`、`download_verify.rs:138`、`checkhashes.rs:112` | 等价重复 | 是 | 四个校验/跳过点各自手写字面量；某一处改了 nightly 的 hash 策略，其余三处不会跟着变 |
| **R1** | 「元数据在哪个目录」 | 指定 `install_state.rs:166 current_meta_dir`（生产仅 1 个调用点 `:188`）vs `query.rs:111-117` 手写同款 if/else | 等价重复 | 是 | 解析器改了分支，`list`/`update` 读的目录与 `load_install_state` 悄悄分叉——这正是 `install_state.rs:99-102` 记载过「cleanup 删掉活版本目录」那一类 bug |
| **R3** | `<root>/apps`、`<root>/shims` 的 global/root 选择 | 指定 `session.rs:373-381,458-460` vs 5 个私有孪生：`cleanup.rs:107-115`、`manifest_source.rs:93-105`、`package/shim.rs:57-65`、`info.rs:579-581`、`shim.rs:476,579-583` | 等价重复（3 个名字同义） | 是 | 布局再变一次（第三种 root 族）要改 6 处；漏改即「删错树」——仓库自己的布局笔记里已记过两次同类事故 |
| **R5** | hash 算法名表 | `internal/hash.rs:54-57,117-120`、`manifest.rs:1444-1450`、`manifest.rs:1471-1475`、`checkver_hash.rs:607-613`、`constant.rs:108`（正则） | 4 份 + 1 正则 | 是 | 校验用算法与 manifest 声明算法必须一致，无任何机制保证 |
| **R6** | hash 前缀格式化 | 指定 `internal/hash.rs:210 format_hash_value`（文档自称唯一来源）vs 重实现 `checkhashes.rs:65-73` | 等价重复 | 是 | 前缀规则改动后 `checkhashes --update` 写出的 manifest hok 能验、Scoop 验不过 |
| **R9** | 架构字段名映射 | 指定 `internal/arch.rs:123 Arch::name()` vs 硬编码 `manifest.rs:1229-1233`、`import.rs:314-319`；另有**死代码** `Package::supported_arch`（`package/mod.rs:482-496`）返回枚举名（`ia32`）而非 manifest 名（`32bit`） | 等价重复 + 死码 | 是 | 死码一旦被启用会打印错值 |
| **R10** | 无扩展名工具解析（`<shims>/tool.exe` → 裸名） | `internal/archive.rs:1072-1078`（7z）、`:1160-1166`（lessmsi）、`internal/readme.rs:24-29` | 3 份 | 是 | 都从环境变量重建 `<SCOOP>/shims`，**都看不到 global shims 目录** → 全局安装时可能报「找不到 7z.exe」 |
| R2 | 「安装是否损坏」 | `cleanup.rs:125-129` vs `query.rs:96-106,159-179` | 等价重复 | 是 | 同一个损坏判据（junction 模式下走了 mtime 兜底）写了两遍 |
| R4 | JSON 文本 → 值 | 单入口 `internal/json.rs:98,116`；裸 `serde_json` 16 处 | 混合 | 低-中 | `internal/json.rs:57-61` **有意**为远程载荷/机器状态保留裸解析（`github.rs`/`checkver*`/`install_state.rs` 属此类）；真正越界的是 `config.rs:1077`、`export.rs:104,307`——它们重新解析 hok 自己刚序列化出来的 JSON |
| R7 | 版本目录排除（`current`、`_*.old*`） | `install_state.rs:150` ≡ `info.rs:601`；`cleanup.rs:233,424`、`sync_remove.rs:424` 范围更窄（有意） | 2 份相同 + 2 份有意不同 | 中 | 权威谓词无共享 helper |
| R11 | 符号链接删除 | 指定 `internal/fs.rs:99-120` vs `link.rs:25-28`、`sync_remove.rs:464-467` | 等价重复 | 低 | `sync_remove.rs:464` 绕过 `operations::unlink_current` 这个 funnel |
| R12 | shim/shortcut 的 owner 探测（token 化 + `apps/<name>/<版本目录>`） | `shim.rs:82-108` ↔ `shortcut.rs:216-245` | 等价重复 | 低 | 各约 25 行同算法，`shim.rs:83` 自己写了「镜像」 |
| R13 | installer/uninstaller 的 `file + args` 展示串 | `sync_install.rs:664-690` ↔ `sync_remove.rs:243-268` | 逐字相同 | 是 | 第三处 `operations/script.rs:700-707` 的判定**不同**（args-only 也会走 file 分支并回退到 URL 文件名）⇒ 展示的步骤类型与实际执行不一致 |
| R14 | cookie map 转换 | `download.rs:356-367` ↔ `:998-1008` | 逐字相同 | 低 | 同文件内两份 |

### 3.2 单入口旁路盘点（12 个 designated 入口）

| 单入口 | 旁路 | 证据 |
|---|---|---|
| `install_state::resolve_current_version` | **无**（8 个生产调用点，无遗留 `read_link("current")`） | 收口完成，是本仓做得最好的一处 |
| `operations::script::run_hook` | **无**（2 个调用点） | 同上 |
| `internal::path::bucket_dir` | **无**（`join("buckets")` 命中全在测试） | 同上 |
| `internal::string::strip_bom` | **无**（2 个生产调用点） | 同上 |
| `os::ps_command` | **无**（7 个生产调用点） | 同上 |
| `calculate_download_size` 必须先于 `download_and_verify` | **无**（2 个调用点都已遵守） | 同上 |
| `internal::json::parse_str` | 4 处真越界（`config.rs:1077`、`export.rs:104,307`）+ 8 处已登记的例外 | `serde_json::from_str|from_slice` 共 23 命中 |
| `install_state::current_meta_dir` | `query.rs:111-117` | 生产仅 1 调用点 |
| `Package::is_nightly` | 3 处裸比较 | 见 R8 |
| `hash::format_hash_value` | `checkhashes.rs:65-73` | 见 R6 |
| `Arch::name` | `manifest.rs:1229-1233`、`import.rs:314-319` | 见 R9 |
| `operations::unlink_current` | `sync_remove.rs:464-467` | 见 R11 |

**读法**：12 个单入口里 **6 个完全干净**、6 个有旁路。干净的那 6 个说明"单入口 + 调用点收敛"这套做法在本仓是**可复制的**，
不是理论；有旁路的 6 个正好是 §3.1 里风险最高的几项。

### 3.3 已分叉孪生（具体差异，不是形状）

| id | 一对孪生 | 具体差异 | 判定 |
|---|---|---|---|
| T1 | `sync_install.rs:664-690` ↔ `sync_remove.rs:243-268` | 两者逐字相同，但都与 `script.rs:700-707` 的判定不同：前者只看 `file`，后者 `file.is_some() \|\| !args.is_empty()` | 展示/步骤类型漂移（非执行错误） |
| T2 | `persist::link`（`persist.rs:41-44`）vs `persist::unlink`（`persist.rs:173`） | link 自己重算版本目录，unlink 由调用方传入（`sync_remove.rs:377,437`） | 入口目录有两个权威——正是该模块头部警告的形状 |
| T3 | `link_current`（`link.rs:20-31`）vs `unlink_current`（`link.rs:38-48`） | 上游 `install.ps1:249-253`/`:269-278` **两边都** `Remove-Item -Recurse -Force`；hok 的 link 多了 `remove_dir_all` 兜底，unlink 没有 | 今天等价（`fs.rs:99-120` 已清只读位并分派目录/文件），但遇到"非链接目录占位"时只有 link 能恢复 |
| T4 | `install_state.rs:145-162 newest_version_dir` ↔ `info.rs:590-613 installed_version_dirs` | 同过滤（`install_state.rs:150` ≡ `info.rs:601`）、同 `sort_by_key(mtime)`，一个返回最后一个、一个返回整列 | 谓词无共享 helper |

### 3.4 必须保留的"重复"（语义不同，不得合并）

- `query.rs:111-117` 与 `current_meta_dir`：前者还需要 `CurrentVersionSource` 来判 broken——合并必须**保留 source 返回值**，不能只消掉分支。
- `cleanup.rs:233`（排除当前版本，保留 `_*.old*` 供显式删除）vs `cleanup.rs:424`/`sync_remove.rs:424`（只排除 `current`）：扫描范围不同。
- `download_apps.rs:92` 传 `should_offline=false` vs `sync_install.rs:377` 算 `total == 0 && !estimated`：`hok download` 有意不启用离线缓存校验。
- `manifest.rs:736-765 arch_specific_field!`（宏，first-present-wins）vs `[&arch.amd64, …]` 数组（固定顺序）：机制与语义都不同。

---

## 4. 外部 crate 的替代与剪裁

### 4.1 直接依赖用量实测（crate 路径命中数，不是标识符命中）

| crate | 使用点 | 文件 | 结论 |
|---|---:|---:|---|
| `reqwest` | 32 | 10 | keep |
| `git2` | 38 | 3 | keep（但 **ssh feature 可关**，见下） |
| `rusqlite` | 9 | 2 | keep |
| `sxd-document` / `sxd-xpath` | 3 / **1** | 2 / 1 | **候选替换**（见下） |
| `jsonpath-rust` | 2 | 2 | keep（但与 sxd 双重 XML/JSONPath 提取并存，见下） |
| `json5` | **1** | 1 | 候选裁掉（`internal/json.rs:103`） |
| `indexmap` | 3 | 1 | 候选裁掉（`manifest.rs:28,194,1069`；`serde_json` 的 `preserve_order` 已把它带进树） |
| `junction` | **2** | 2 | 候选替换（`internal/fs.rs:149` 生产 + `info.rs:1213` 测试） |
| `sys-locale` | **1** | 1 | 候选替换（`src/i18n.rs:126`） |
| `zstd` | **1** | 1 | keep（已实测否决 `ruzstd`，见 §4.4） |
| `lzma-rust2` | **1** | 1 | keep |
| `flate2` / `tar` / `bzip2` | 4 / 3 / 3 | 2 / 2 / 2 | keep |
| `base64` | 23 | 3 | keep（`internal/base64.rs:34-40` 用 `GeneralPurposeConfig` 精确对齐 `Convert.FromBase64String`，自研需复刻这些语义） |
| `rayon` | 2 | 2 | keep（`bucket.rs` 扫描 + `query.rs`） |
| `futures` | 8 | 4 | 候选裁剪（见下） |
| `async-trait` | 8 | 4 | **keep**：`Session` 持有 `Arc<dyn Network/Git/Archive + Send + Sync>`（`session.rs:109-115`），原生 async-fn-in-trait 目前仍非 dyn-safe ⇒ 不能去 |
| `windows-sys` | 13 | 2 | keep（但 features 可裁，见 §5.3） |
| `innospect` / `nsis` / `shortcuts-rs` / `rars` / `zstd` | 2 / 5 / 1 / 9 / 1 | 1 / 1 / 1 / 2 / 1 | keep（已上游化，见 `libscoop/Cargo.toml:83-92` 注释） |

### 4.2 真的有杠杆的 4 条

**(A) `git2` 的 `ssh` feature → 关掉（去掉 `libssh2-sys` 的 C 构建 + `openssl-sys` 分支）**
- 事实：`libscoop/Cargo.toml:24` `git2 = "0.19.0"` 用默认 features，实测解析出 `default, https, openssl-probe, openssl-sys, ssh, ssh_key_from_memory`；
  `cargo tree` 反查：`libssh2-sys 0.3.2 ← libgit2-sys`、`openssl-sys ← git2/libgit2-sys/libssh2-sys/native-tls`。
- 已在 `optimization-audit-2026-09.md` §8 记录过：hok 的 libgit2 凭据回调**没有 `SSH_KEY` 分支与 hostkey 校验**，
  实测报 `invalid or unknown remote ssh hostkey` ⇒ **ssh 从来不可用**。
- 收益：release 图减 `libssh2-sys` + `libz-sys` 的 C 构建路径（openssl-sys 在 Windows 上由 `native-tls→schannel` 之外仍被 libssh2 拉入）。
- 代价：`bucket add` 需对 `ssh://`/`git@` 形态**显式报错**并登记 gap（否则会退化成 libgit2 的晦涩错误）。
- **需要用户拍板**：这是「砍掉一个官方支持但 hok 从未实现的能力」还是「补齐 ssh」——属范围决策（`investigate-first`）。

**(B) `criterion` 的 `plotters` feature（dev-only）**
- 事实：`criterion = "0.8"`（`libscoop/Cargo.toml:110`）默认 features 解析出 `cargo_bench_support, default, plotters, rayon`。
  两个 bench（`pure_functions.rs`、`event_emission.rs`）只用到 `Criterion`/`criterion_group!`/`criterion_main!`/`black_box`，**不生成图表**。
- 收益：仅影响 `cargo bench`/`cargo test --all-targets` 的编译时间（不进 release 图，不影响二进制）。建议 `default-features = false, features = ["cargo_bench_support"]`（需实测确认 group/main 宏仍可用）。

**(C) `rusqlite` `bundled`（C SQLite 编进二进制）vs `winsqlite3` 形态**
- 事实：默认 `bundled`（`libscoop/Cargo.toml:29`）⇒ 编译 C amalgamation，二进制 ≈8.12 MB；
  `winsqlite3` 形态 ≈7.08 MB（差 ≈1.04 MB），仅 x64/arm64 可用，由 `scripts/build-hok.ps1 -Mode winsqlite3 -Verify` 编排，
  断言在 `crates/libscoop/build.rs:25-118`。
- 这是**分发形态决策**（默认 bundled 是有意的：不依赖 Windows 版本自带 sqlite），不建议改默认；
  已在 `binary-size-analysis-2026-09.md` 分析过，本文只做交叉引用。

**(D) `futures` 的 8 处用法 → 可用 `tokio` 原生替代**
- 事实：实际用到的只有 `futures::StreamExt` + `futures::stream::iter(...).buffer_unordered(n)`：
  `checkver.rs:315,375`、`download.rs:906,939`、`download.rs:1024,1039`、`download_fragmented.rs:90-122`、`network.rs:24,606`。
- 替代选项：① `tokio::task::JoinSet`（限 `'static + Send` 任务，`download_fragmented` 的注释 `:85` 明确说这些 future 是**非 Send** ⇒ 不适用）；
  ② `tokio_stream::StreamExt::buffer_unordered`（引入 `tokio-stream`，比 `futures` 小但仍是新依赖）；
  ③ 自写容量受限的 `FuturesUnordered` 循环（`futures-util` 仍是 `futures` 的一部分）。
- **判定：收益有限且有一处硬约束（非 Send future）**，除非同时把下载管道改成 `spawn` + `JoinSet`（那是更大的架构动作）。**倾向不做**，若做需先给行为等价性证据。

### 4.3 小型候选（收益小、需逐个拍板）

| 候选 | 现状 | 替换代价 | 倾向 |
|---|---|---|---|
| `sys-locale`（1 处，`src/i18n.rs:126`） | 取系统 locale | `windows-sys` 已在树，可调 `GetUserDefaultLocaleName`（FFI，2 处 unsafe 换成 1 处） | 可做，收益≈1 crate |
| `junction`（生产 1 处，`internal/fs.rs:149`） | `junction::create` | 自实现 reparse point 需 `DeviceIoControl` + `REPARSE_DATA_BUFFER`（约 60 行 unsafe） | **不做**：用 unsafe 换 1 个纯 Rust 依赖是净亏 |
| `json5`（1 处，`internal/json.rs:103`） | 宽容 JSON 回退链 | 去掉就丢「手改 manifest 带注释/尾逗号能装」的能力（`optimization-audit` §11 已把它定为**有意行为**并有契约测试） | **不做** |
| `indexmap`（1 处类型，`manifest.rs:28`） | `manifest.suggest` 保序 | `serde_json` 的 `preserve_order` 已启用 `indexmap` ⇒ 去掉直接声明不减树，只减一行 manifest | 可做（纯声明收口），收益≈0 |
| `sxd-document` + `sxd-xpath`（1 个生产调用点，`checkver.rs:1609`） | xpath 提取 | 换 `roxmltree`（解析）+ 自写 xpath 子集，或换 `xee`；官方 Scoop 的 xpath 面很窄 | **候选调研**：收益是去掉 2 个低活跃度 crate，代价是重写提取语义 + 新契约测试 |

### 4.4 已否决/已排除（不要重新提）

- `zstd → ruzstd`：实测否决（`ruzstd` 只解第一帧，静默截断；收益仅 ~10.6 s 冷构建）。
- `git2 → gix`：编译代价 ≈20 分钟，且 libgit2 无法用系统 DLL 替代。
- `rusqlite 0.31 → 0.40`：host 图仅 −2 crate，代价是 SQLite 3.45→3.53.2 大版本跳变 + lock 复杂化（已从历史移除）。
- `chrono → time`、`curl → reqwest`、`sysinfo → Win32 FFI`、`once_cell → LazyLock`、`rust-i18n → hok-i18n`：均已落地。
- `async-trait`：dyn-safe 约束，不能去。
- TLS 后端：已切 `native-tls`(= schannel)，去 10 个 rustls 链 crate。

---

## 5. feature 裁剪

### 5.1 现状：**本仓没有任何内部 feature**（这是最大的结构性发现）

| 事实 | 证据 |
|---|---|
| 全仓生产源码 `cfg(feature = …)` 门控 **0 处** | 对 `src/**`、`crates/*/src/**` 全量搜索 `feature = ` 零命中 |
| 根 `Cargo.toml:98-99` 有 `[features] default = []`，但该 crate 是 bin，声明无消费者 | `Cargo.toml:98` |
| `crates/libscoop/Cargo.toml:98-99` 同样 `[features] default = []`，同样无门控 | — |
| `crates/hok-shim`、`hok-i18n`、`hok-i18n-derive` 无 `[features]` | — |

**含义**：hok 目前**无法编译期裁剪任何能力**。所有可选性都靠另外三种机制表达：
① 目标平台（`[target.'cfg(windows)'.dependencies]`，`libscoop/Cargo.toml:69-92`）；
② 环境变量 + 链接器（winsqlite3 形态，`build.rs:25-118` + `scripts/build-hok.ps1`）；
③ 独立 crate/bin（shim 双变体）。

**给结论而不是给建议**：① 与 ③ 是合适的；② 是 **Cargo 之外的一层编排**，代价是「构建形态不可从 manifest 读出」，
`build.rs:55-85` 那段 panic 文案就是这笔代价的具体形态。若要改成 Cargo feature（如 `sqlite-winsqlite3`），
需要 `libsqlite3-sys` 支持（它用的是环境变量协议，不是 feature），**不建议**。

### 5.2 值得裁的 crate features（实测已启用但无人要求的）

| 依赖 | 当前启用 | 可裁项 | 证据/代价 |
|---|---|---|---|
| `junction = "1.2.0"` | 解析出 `default` + **`unstable_admin`** | 若 `default` 里含 `unstable_admin` 而 hok 不用它，可 `default-features = false` | 需实测（`cargo tree -e features -p junction`）；收益 1 个 feature 分支 |
| `criterion = "0.8"` | `plotters`、`rayon`、`cargo_bench_support` | **`plotters`**（bench 不画图） | §4.2(B)；dev-only |
| `regex`（workspace 声明） | 解析出 `unicode-age/bool/case/gencat/perl/script/segment/suggestions` 等全套 | 理论上可裁 `unicode-*`，但解析出的集合与直接声明**不一致** ⇒ 有别的 crate 也在开这些 feature；裁了不减树 | 不划算 |
| `tokio`（libscoop，`Cargo.toml:20`） | `rt, rt-multi-thread, time, macros, sync` + 传递 `net/fs/io-util...` | **`rt-multi-thread`**：库不该替应用决定 runtime 形态（应用是 `hok`，`src/cmd/mod.rs:578` 起用 `#[tokio::main]`，即 root 已开 `rt-multi-thread`）。**但**若库内 `tokio::spawn` 需要多线程调度器语义，裁掉会改变行为——**必须先确认 `libscoop` 是否有 `spawn`/`spawn_blocking`** | 见 §10 复跑命令 |
| `tokio`（root，`Cargo.toml:68`） | 额外开 `signal` | `signal` 是 `create_app()` 里 Ctrl-C 用的（`Cargo.toml:69-70` 注释） | 不要动 |
| `windows-sys`（`libscoop/Cargo.toml:77-82`） | `Win32_Foundation`、`Win32_Security`、`Win32_Security_Authorization`、`Win32_Storage_FileSystem` | 4 个都有用户（`permission.rs:22` 用 Security/Authorization；`os.rs`/`env.rs` 用 Foundation/Storage） | 不动 |
| `reqwest`（`Cargo.toml:40`） | `native-tls, stream, json, gzip` | `json` 是否用到 `Response::json()`？若只用 `bytes()/text()` 可裁 | 需实测 |
| `sevenz-rust2` | `aes256, bzip2, ppmd, util`（**已经 `default-features = false`**） | 已是最小集（各自对应 7z 的加密/内层压缩） | 不动 |

### 5.3 跨 manifest feature 漂移风险（scoped，但要登记）

`crates/libscoop/Cargo.toml:55-59` 的注释已把 `flate2` 的 `zlib-rs` 单点化。但 **`git2` 的 `ssh`/`openssl-sys`
目前在 `libscoop/Cargo.toml:24` 是裸 `"0.19.0"`（全默认 features）**，是同一类风险的反面例子：
`native-tls` 走 schannel 换来的「去掉 openssl 链」收益，被 `git2` 的默认 features 部分抵消（`openssl-sys` 回到树里，
虽然 Windows 上不编 OpenSSL 本体）。**建议**：把 `git2` 也移入 `[workspace.dependencies]` 并显式写死 features（与 A 条一起做）。

---

## 6. 建议优先级（按"问题是否真实"分层，不按工作量）

### T1 — 可立即做，且能消除真实的静默漂移（每项都小）

| 序 | 项 | 出处 | 预期收益 |
|---|---|---|---|
| 1 | 四个 nightly 判定点改走 `Package::is_nightly`（`mod.rs:383`） | §3.1 R8 | 3 行，消除一个校验路径无声分叉 |
| 2 | `query.rs:111-117` 改调 `current_meta_dir`（**保留 `CurrentVersionSource`**） | §3.1 R1 | 防止 `list`/`update` 读陈旧 `current/` 元数据（历史事故同类） |
| 3 | `manifest.rs` 的架构顺序收敛为一个 `arch_specs()` 迭代器 | §2.2 P2/N1 | 一处改、四处生效；消除 checkurls/checkhashes 校验面不一致 |
| 4 | `checkhashes.rs:65-73` 回到 `hash::format_hash_value`；`manifest.rs:1471-1475` 回到 `hash.rs:117-120` | §3.1 R5/R6 | manifest 前缀与校验算法单点化 |
| 5 | 三个"无扩展名工具解析"收敛为一个（并**接上 global shims 目录**） | §3.1 R10 | 全局安装下 7z/lessmsi 找不到的问题根治 |

### T2 — 值得做，但要接受"测试基建"这个主题

| 序 | 项 | 出处 |
|---|---|---|
| 6 | 7 份 CLI 集成测试 harness 抽成一个 `tests/common`（已漂移，会静默读真实 `~/scoop`） | §2.3 P8 |
| 7 | 9 份 `fn plain()` 抽成一个测试工具（`src/cmd/tests_common` 或 `tests/` 共享） | §2.3 P7 |
| 8 | `SyncArgs`/`QueryArgs` 二选一：要么真正 `#[command(flatten)]`（CLI 表面变化，需用户拍板），要么去掉 `#[arg]` 属性、只留纯数据 + `from_common` 构造器 | §2.2 P2/P3 |
| 9 | 5 个 `apps_dir_in`/`shims_dir_in` 孪生收敛到 `Session::*_dir_in(scope)` | §3.1 R3 |
| 10 | `git2` features 收口到 `[workspace.dependencies]` 并关 `ssh`（**需先拍板是否放弃 ssh**） | §4.2 A / §5.3 |

### T3 — 设计题，需要一次性拍板（不要与上面混批）

| 序 | 项 | 出处 | 待决点 |
|---|---|---|---|
| 11 | 事件模型折叠（`Start/Done/Progress` → `StepKind`），或把 28 个 debug-only 变体下沉到 `tracing` | §1.3 | (a)(b)(c) 三选一；影响 CLI + libscoop + 事件循环三处 |
| 12 | libscoop 公开面收窄（`package/` 24 个 `pub` 子模块 → CLI 专用者降 `pub(crate)`） | §1.4、§1.5 | 是否保留"第三方可基于 libscoop 写前端"的承诺 |
| 13 | `checkver.rs` 拆分 / `commit_one_install`+`commit_one_remove` 抽公共提交管线 | §1.9 | 必须先写清具体问题（`no-patchwork-fixes`：不得用开关掩盖 install/remove 语义差异） |
| 14 | `sxd-document`+`sxd-xpath` 替换调研 | §4.3 | 收益小、需重写提取语义 + 契约测试 |

### T4 — 明确不做（本文结论）

`build.rs`/`build-hok.ps1` 合并（职责不同）、`enum Command` 与目录扫描的双份真相（clap derive 需要）、
`async-trait` 去除、`junction` 去依赖、`json5` 去除、`zstd`/`git2` 换实现、`rusqlite` 换 winsqlite3 默认、
`futures` 替换（有一处非 Send 硬约束）、`hok-shim` 两 bin 合并、serde 字段镜像去重、测试侧"同骨架改名"克隆。

### 使用规则：衰减资产 vs 投机性重构（2026-09-29 用户拍板「只落规则、代码先不动」）

**要回答的问题**：系统性重构是不是该等功能全部实现之后再做？
**判据**：不能一刀切——两类东西判据相反。

- **衰减资产（不该等）**：同一个决定已存在于 ≥2 处且**可能静默分叉**。每多一个功能落在同一文件上，
  下一次改动要覆盖的副本就多一份，而且分叉表现是**静默错误**（校验不一致、测试读到真实 `~/scoop`），
  不是编译失败。⇒ 特征：无需新信息就能判对错，收益随代码增长**递减**（越晚做越贵）。T1/T2 属此类。
- **投机性重构（该等）**：需要**需求稳定**或**性能/外部证据**才能判对错，现在动有返工风险。
  ⇒ 特征：收益不随代码增长（什么时候做都一样贵），但做错的代价是白干。T3 属此类。

**A｜衰减资产清单（出处：本文 §2、§3）**

| 项 | 现状 | 目标（唯一入口） |
|---|---|---|
| 架构固定顺序 `64bit→32bit→arm64` | `manifest.rs` 一处手写 4 遍 | 一个 `arch_specs()` 迭代器 |
| hash 算法/前缀表 | 算法 4 份 + 前缀 2 份 | `internal/hash.rs` 单点 |
| nightly 判定 | `is_nightly()` **零生产调用** + 4 处裸比较 | 全走 `Package::is_nightly` |
| 元数据目录解析 | `current_meta_dir` 仅 1 调用点，`query.rs` 手写同款 | 全走 `current_meta_dir`（保留 `CurrentVersionSource`） |
| 测试入口 | 7 份 CLI harness（**已漂移**）+ 9 份 `fn plain()` | 各一份共享模块 |

**修法验收（防"越修越抽象"）**：① 只做"副本 → 单入口"，**不引入新抽象层**；② 零行为变更；
③ 每项独立提交、可单独 revert；④ 若某处发现两个副本**语义确实不同**，登记为"有意偏离"并停止合并
（判例见本文 §3.4）。

**A 表落地进度（2026-10-11）**：架构固定顺序、hash 算法/前缀表、nightly 判定、测试入口 **四项已收敛**
（判据仍照上一条：单入口 + 零行为变更）；**元数据目录解析不合并** —— `install_state.rs::current_meta_dir`
只按 `no_junction` 分派（与官方 `lib/core.ps1:373-383 currentdir()` 逐字对应），`query.rs` 多一支
"junction 模式下 `current` 损坏（source = `VersionDirs`）时回退版本目录"，这是 hok 有意的损坏安装识别，
两侧各有测试钉住，**合并任一侧都是行为变更**。本条是对 §3.4 措辞的补充（那里只说"保留 source 返回值"，
但 source 还决定目录选择）。附带两条未收敛项：`is_nightly` 的大小写（官方 `-eq` 不区分大小写）、
清单 `SHA256:` 大写前缀（官方 `get_hash` 的 `-notcontains` 接受，hok 拒绝）——两者均属行为变更，
已登记在 [`pending-tasks.md`](../plan/pending-tasks.md)，不随纯搬移处理。

**B｜投机性重构的触发条件（不是"以后再说"，而是写明什么时候该动）**

| 项 | 触发条件 |
|---|---|
| 事件模型折叠（87 变体 → `StepKind`，T3 第 11 项） | "新增一个事件要改 3 个文件"成为实际卡点时；或相关设计题落地后需求不再变动 |
| 库内建默认事件消费者（`libscoop::headless`，§1.6） | 出现**第一个真实第三方/嵌入需求**时；纯内部永远不需要 |
| `commit_one_install` / `commit_one_remove` 抽公共管线（T3 第 13 项） | install/remove 出现**第三次**语义分叉时（前两次已登记在 §3.3 T1/T3） |
| `checkver.rs` 拆分 / `commit_one_*` 瘦身（T3 第 13 项） | 需要先给出具体问题陈述（性能或维护痛点），否则不动 |
| 公共 API 收窄（627 项 → CLI 所需，T3 第 12 项） | libscoop 定位（§8 的路线 A / B）拍板时 |
| `sxd-xpath` 替换（T3 第 14 项）、`futures` 移除 | 先有可度量的收益证据（编译时间/依赖数），否则不做 |

**衰减指标（常驻判据；复跑命令见本文 §10）**

粗粒度即可，用途是"发现自己又抄了一份"：

- nightly 判定 ≤1 个单入口，裸比较出现**第 5 处** ⇒ 触发收敛；
- hash 算法表 ≤1 处（`internal/hash.rs`），出现**第 5 处** ⇒ 触发收敛；
- 架构顺序定义 ≤1 处，出现**第 5 处** ⇒ 触发收敛；
- `Session::*_dir_in` 类 scope 决策 ≤1 个 helper，出现**第 7 个**孪生 ⇒ 触发收敛；
- CLI 集成测试 harness 与测试侧 oracle 各 ≤1 份 ⇒ 任何第 2 份出现即触发。

> 为什么放粗阈值而不是精确计数：精确值会随代码漂移，写进常驻层就会变成第二份会过期的真相
> （见根 [`AGENTS.md`](../../../../AGENTS.md) → 「复述必漂」）。触发条件只写"该不该动"，具体数字按需重数。

---

## 7. 与既有文档的关系

| 文档 | 关系 |
|---|---|
| [`hok-overall-framework-analysis.md`](hok-overall-framework-analysis.md) | **被本文取代**（§0 列出其失真项），应归档 |
| [`optimization-audit-2026-09.md`](optimization-audit-2026-09.md) | 互补：那份是"精简/合并/优化空间"的**执行记录**（含 P1–P3 与 §8–§11 落地结果），本文是**当轮结构性盘点**。重叠项（未用依赖、clap 参数组、innospect 上游化、传递依赖收拢）本文不重复叙述，只引用其结论 |
| [`binary-size-analysis-2026-09.md`](binary-size-analysis-2026-09.md) | 体积数据来源；本文 §4.2(C) 引用 |
| [`unsafe-audit.md`](unsafe-audit.md) | unsafe 台账；本文 §4.3 的 junction 判定引用其口径 |
| [`scoop-gap-status.md`](scoop-gap-status.md) / [`command-deviation-audit-2026-09.md`](command-deviation-audit-2026-09.md) | 与官方 Scoop 的**行为**差距，本文不涉及；本文 §3.3 若确认为行为偏离，应登记到那两份 |
| [`../spec/output-system-design.md`](../spec/output-system-design.md)（若存在）/ `ARCHITECTURE.md` | §1.3 的事件模型决策若拍板，需要回写 |
| [`../plan/pending-tasks.md`](../plan/pending-tasks.md) | 未完成项索引：T1/T2（衰减资产）与 T3（投机性重构）在其中按状态登记；判据见本文 §6 的「使用规则」小节 |

> **执行口径（2026-09-29 用户拍板）**：本文的 T1/T2 不排在"等功能全实现之后"——它们是衰减资产，
> 每多一个功能落在同一文件上就更贵，且分叉表现为静默错误。但**代码本轮不动**，只落治理规则
> （见本文 §6 的「使用规则」小节）。

---

## 8. 显式登记：本文未做的判断

以下**没有被本文判定**，需要用户或后续会话决定：

1. **ssh 是否放弃**（§4.2 A）：官方 `lib/buckets.ps1` 明确支持 ssh 形态，而 hok 从未可用。
2. **libscoop 的定位**（构建面 §1.5 + 能力面 §1.6）：走路线 A（登记为"不可独立使用"，公开面随之可收窄）
   还是路线 B（补齐可独立引用；工作量为：构建架构 + `locales/` 搬位 + 发布两个 i18n crate + **库内建默认事件消费者**）。
   这一条决定 §1.4 与 §5.3 的走向，且是**唯一一个"不选就没法收口文档"的问题**。
3. **事件模型方向**（§1.3）：(a) 折叠 / (b) 下沉 / (c) 不动。
4. **`SyncArgs` 路线**（§2.2 P3）：flatten（CLI 表面变化）还是纯数据化。
5. **`tokio` 的 `rt-multi-thread` 能否从 libscoop 裁掉**：取决于库内是否有 `tokio::spawn`（§10 第 5 条给复跑命令），本文未下结论。
6. **`sxd-xpath` 替换**（§4.3）：需要先量官方 Scoop 的 xpath 用面。

---

## 9. 有意偏离与保留项（本文确认不动）

- `internal/` 全 `pub(crate)` 收口：已完成，保持。
- `archive → event` 的工具层例外依赖：保留（`hok-code-organization` 已判）。
- 领域服务 ↔ `package/` 的双向无环依赖：保留。
- `src/cmd/mod.rs` 的 clap 本地化两阶段改写：**保留但登记为高脆弱面**（clap 升级必须重跑帮助面逐字节比对）。
- `SyncOption`/`QueryOption` 等公共枚举：保持 `pub`（`tests/` 依赖）。
- `crates/libscoop/build.rs` 的 embedded-shim 前置要求：保留（构建顺序约束，无更优解）。

---

## 10. 复跑口径（每条结论对应命令）

```powershell
# 1. 生产/测试行数（首处 #[cfg(test)] 为界）——本机脚本，见文末说明
node $env:TEMP\hok-audit\loc.mjs (Get-Location) crates/libscoop/src src crates/hok-shim/src

# 2. 依赖闭包与多版本
cargo metadata --format-version 1 --all-features > $env:TEMP\hok-audit\meta.json
node $env:TEMP\hok-audit\dep.mjs $env:TEMP\hok-audit\meta.json
cargo tree -e normal,build --target x86_64-pc-windows-msvc --offline

# 3. feature 现状（应为 0 命中）
Select-String -Path 'src\**\*.rs','crates\*\src\**\*.rs' -Pattern 'feature = '

# 4. crate 使用点（示例：sys-locale / junction / sxd-xpath / git2）
Select-String -Path 'crates\libscoop\src\**\*.rs','src\**\*.rs' -Pattern 'sys_locale::|junction::|sxd_xpath::|git2::'

# 5. 结构问题 S2 与 tokio 裁剪判据
Select-String -Path 'crates\libscoop\src\event.rs' -Pattern '^    [A-Z][A-Za-z0-9]*\s*[\({,]'
Select-String -Path 'crates\libscoop\src\**\*.rs' -Pattern 'tokio::spawn|spawn_blocking|Handle::current'

# 5b. §1.5「能否独立使用」的三条复跑（B3 会破坏本仓 target/，用临时目录更稳）
cargo package -p libscoop                              # B1：应停在 "must have a version requirement"
cargo package -p hok-i18n --no-verify --list           # B2：产物只有 6 个文件，无 locales/
# B3：把 target/ 改名后再构建，应 panic 在 build.rs:204；验完改名回来
Rename-Item target target.audit-moved; cargo build -p libscoop; Rename-Item target.audit-moved target

# 5c. §1.6「能力面」的复跑
Select-String -Path 'crates\libscoop\src\**\*.rs' -Pattern 'session.emitter\(\)'   # 28 个发射点
Select-String -Path 'crates\libscoop\src\session.rs' -Pattern 'fn emitter'          # pub(crate)
Select-String -Path 'src\**\*.rs' -Pattern 'run_event_loop'                         # 外壳只在 CLI
Select-String -Path 'crates\libscoop\src\package\resolve.rs' -Pattern 'receive_response'
Select-String -Path 'crates\libscoop\src\package\sync.rs' -Pattern 'receive_response'

# 6. 逻辑冗余关键点（应各只命中单入口）
Select-String -Path 'crates\libscoop\src\**\*.rs' -Pattern 'is_nightly|nightly"'
Select-String -Path 'crates\libscoop\src\**\*.rs' -Pattern 'current_meta_dir'

# 7. 代码冗余：两个自研检测器（启发式，非仓库内）
node $env:TEMP\hokaudit\dup.mjs  (Get-Location) 8
node $env:TEMP\hokaudit\near.mjs (Get-Location) 15 50 0.7
```

**脚本归属说明**：本轮四个分析脚本（`loc.mjs`/`dep.mjs`/`feat.mjs`/`panic.mjs`）与两个检测器
（`dup.mjs`/`near.mjs`）都在系统临时目录，**未入库**（`.agents/AGENTS.md`：一次性探针用完即删）。
若后续要把它作为常规门禁（例如"重复段不得新增"），应移入 `.agents/scripts/` 并配 `--check` 模式——
这属独立议题。

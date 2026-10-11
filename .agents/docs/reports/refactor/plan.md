# 重构计划：Edition 2024 升级 + 架构演进

> 创建：2026-08-21 · 分支：`refactor/edition-2024` · 状态：进行中

## 背景与动机
- 当前 workspace 仅 `innospect` 为 edition 2024，其余 4 个 crate（root / libscoop / hok-shim / hok-i18n-derive）为 2021
- 本地 rustc 1.97.1 已就位（edition 2024 需 1.85+）
- 对齐 rake 的工程方法论：trait 化 I/O、domain 纯化、安全纪律、基准护栏（功能面 hok 已领先，学的不是功能）
- 用户确认：本项目本就是对 hok 的重构，不排斥大规模结构新调整

## 收益（Edition 2024 对本仓库的实测影响）
- `unsafe extern` 语法强制：hok-shim 8 处 + os.rs 4 处 extern block 需显式标 `unsafe`（FFI 边界可视化）
- `unsafe_op_in_unsafe_fn` 提升：unsafe fn 内部必须显式包 `unsafe {}` 块（安全审计资产）
- workspace 统一 edition，消除 2021/2024 混编心智负担
- 生态解锁：新依赖版本逐步默认 2024；与重构捆绑执行，避免二次全库 diff
- 实测无 `static mut`（static_mut_refs 影响 0）；`impl Trait` 返回仅 2 处（RPIT 生命周期捕获影响极小）

## 阶段路线
| 阶段 | 内容 | 状态 |
|---|---|---|
| 0 | 开分支 `refactor/edition-2024` | ✅ |
| 1 | Edition 2024 全量升级：cargo fix --edition → Cargo.toml 2024 → 手工修 unsafe extern / unsafe fn → build+test 全绿 checkpoint | ✅ |
| 2 | domain 纯化：Package/Manifest/Version/Config 与 I/O 剥离（零 I/O、零 async、零框架依赖） | ✅ |
| 3 | infra trait 化：network / git / archive 抽 trait + 默认实现 + 内存 fake（单测脱离真实网络/磁盘） | ✅ |
| 4 | operations 异步化（tokio）：按 I/O 密集程度分批（checkver/checkup/search/下载 → install/upgrade/cleanup），本地快操作保持同步 | ✅ |
| 5 | 护栏：criterion 基准 + vs Scoop 脚本基准 + `unsafe_code = deny`（除 hok-shim / 必要 FFI） | ✅ |

## 风险与保障
- **行为等价是生命线**：每阶段 checkpoint = `cargo build` + `cargo test` 全绿
- 重构前把既有行为清单（hok-audit-fix-history / scoop-gap-audit 索引）导出为测试矩阵
- 全异步 ≠ 快：tokio 价值在并发场景（checkver/checkup 批量），单命令链路保持同步更简单
- 提交规范：一个主题一个 commit；git 一律 `-c core.autocrlf=false`

## 执行日志
### 阶段 0 + 阶段 1（已完成，2026-08-21）
- [x] 创建分支 `refactor/edition-2024`（工作树携带上一会话遗留未提交 auto_pr.rs 改动，不入本主题 commit）
- [x] 写入本文档（.agents/docs/，不入库，沿用报告不入库约定）
- [x] cargo fix --edition（自动修复 rust_2024 兼容项：unsafe attrs / unsafe extern / unsafe-op-in-unsafe-fn / match ergonomics / use<> RPIT）
- [x] 4 个 Cargo.toml edition → 2024
- [x] cargo build：edition 2024 下全绿（33.97s）
- [x] cargo test --workspace 全绿（checkpoint，421 passed）
- [x] 提交（两个主题两个 commit）：
  - `e19a7b9` chore: migrate workspace crates to edition 2024（11 文件：4 Cargo.toml + 7 rust）
  - `fc23dd9` test(libscoop): isolate git pull tests from global core.autocrlf（1 文件）

### 阶段 1 附注
- ✅ `internal::git` 两个 pull 测试：HEAD 上即因全局 `core.autocrlf=true` 失败（既有环境问题，非本迁移回归）；测试 setup 已显式 `core.autocrlf=false` 隔离（`fc23dd9`，已完成）
- ✅ `hok-shim/src/lib.rs` 等 7 个改动文件经 rustfmt（--edition 2024）格式化；git.rs 316 行 assert 格式已手工修复（2026-08-21，随本会话提交）
- 工作树仍留有上一会话 auto_pr.rs 未提交改动（88 行）与若干未跟踪文件（AGENTS.md/.agents/docs/脚本等），未并入本主题

### 阶段 2（已完成，2026-08-21，4 个主题 4 个 commit，每步 `cargo test --workspace` 全绿）
- [x] **Manifest arch 选择纯化**（`1fd926c`）：`arch_specific_field!` 收 `$arch` 参数；13 个架构相关访问器新增纯 `*_for(arch)` 变体（bin/url/hash/env_add_path/env_set/extract_dir/pre/post_install/pre/post_uninstall/installer/uninstaller/shortcuts），`*()` 变薄 wrapper 走 `effective_arch()`（runtime arch + Scoop ARM64 fallback）
- [x] **顺带修复 ARM64 fallback 布尔反转 bug**（同 commit）：`Arch::supported` 之前收到 `aarch64.is_none()`（反转）——恰好在 manifest *有* arm64 段时触发 fallback、没有时不触发；官方 lib/manifest.ps1 以 arm64 段*缺席*为触发条件。仅 Windows ARM 主机可观测。新增 `has_arm64()` + 回归测试
- [x] **Manifest/InstallInfo 文件 I/O 移出类型**（`7ac7590`）：`Manifest::parse(path)` → 模块级 `manifest::parse_file(path)`（facade 再导出）；`InstallInfo::parse(path)` → `install_state::parse_install_info(path)`；19 个调用点迁移，类型零 I/O（纯构造走 from_json/parse_str）
- [x] **Config 纯化**（`f63c58b`）：`commit()`/写盘 `init()` 移除，改模块级 `config::write`/`config::init_default`；`set`/`set_alias` 变纯内存变更器，所有变更点（config::set/alias_add/alias_remove wrapper、bucket::update）显式持久化；`update_cooldown_remaining_at(now)` 纯变体 + 薄 wrapper
- [x] **Package 墙钟参数化**（`3402c84`）：`effective_version_at(now)` 纯变体（nightly-YYYYMMDD 对显式时钟）+ `effective_version()` 薄 wrapper；固定时钟回归测试
- [x] **Version**：`internal::version.rs::compare_versions` 本就纯（零 I/O/async/框架），仅复核，无需改动
- [x] 验收复核：四类型（Package/Manifest/Version/Config）均零 I/O、零 async、零框架依赖；host 依赖（Arch::current/墙钟）收敛为类型上的薄 wrapper，纯变体全部可确定性地单测
- 注：`internal::git::tests::pull_merge_commit` 在 full workspace 并行跑时偶发失败（libgit2/环境抖动，isolated 与整包重跑均绿）——既有 flake，非本阶段回归；clippy 无新增 warning（既有 collapsible-if 均为老代码）


### 阶段 3（已完成，2026-08-21，7 个主题 7 个 commit，每步 `cargo test --workspace` 全绿）
- [x] **infra trait 化**（`1c44c18`）：network/git/archive 各抽 trait + 零尺寸默认实现（`RealNetwork`/`RealGit`/`RealArchive` 包裹既有 internal 自由函数，行为逐字节等价）+ Session 挂载（`OnceCell<Arc<dyn …+Send+Sync>>` 槽位 + `network()`/`git()`/`archive()` 访问器 + `set_network`/`set_git`/`set_archive` 注入钩子，对齐 `set_output` 模式）+ 新增 `Error::InfraAlreadySet` 变体与 i18n。trait 定义在工具层（internal/），无反向依赖；`Arc<dyn Send+Sync>` 让 worker 线程共享 fake（Session 本身 !Sync）
- [x] **network 调用点迁移**（`914a058`）：`network.rs` 门面（head_url/download_file/download_page）与 checkver worker 线程改走 `session.network()`（worker 用 `catch_unwind(AssertUnwindSafe(..))` 容纳 `Arc<dyn Network>`）；checkurls/checkhashes/create/checkver_hash/manifest_source 经门面自动随迁
- [x] **git 调用点迁移**（`dd45fae`）：`bucket::add`/`bucket::update` 改走 `session.git()`（update 线程 clone Arc 共享 fake）；`Bucket::remote_url()/updated_at()` 保持直用 real git（session 无关惰性元数据，rayon 并行场景，记录为 gap）
- [x] **archive 调用点迁移**（`20e157b`）：`operations/extract.rs`（extract_archives/extract_markers）改走 `session.archive()`，保留 emitter 进度回调语义
- [x] **download 调用点迁移**（`71b3e0b`）：`download_fragmented`/`download_range`/`try_download_range` 改走 `&(dyn Network + Sync)` + RequestOptions；单流下载与 calculate_download_size 的 HEAD 改走 `session.network()`（HeadResult 增 `content_length` 字段，保持双 HEAD 语义）；ChunkError 合并进 RangeError（Final/Transient 语义原样保留）
- [x] **内存 fake + 单测离线化**（`e08e63b`）：test_utils 新增 `FakeNetwork`（按 URL 提供 body + range_server 行为镜像：fail_first/ignore_range/truncate_first/fail_range）+ `FakeGit`（记录 clone/pull）；新增离线测试：checkver 端到端 canned 页面、download_fragmented 分片/重试、bucket add/update 注入 fake git
- [x] **infra 注入定型**（`84d2911`）：`set_network`/`set_git`/`set_archive` 改收 `Arc<dyn …+Send+Sync>`（测试可持句柄检查 fake）；`RealNetwork` 的 cookie 注入分支按 clippy 收敛（`filter(!is_empty)`，行为等价）
- [x] 验收：`cargo test --workspace` 全绿（libscoop 432 passed）；permission 2 例 ACL 测试在本 DSH 沙箱环境因进程 token 无法授 DACL 而失败（clean HEAD 同样失败，环境约束非回归）
- 注：本沙箱 gpg 需先设 `$env:HOME=$env:USERPROFILE` 才能运行；提交经 `git rebase --exec "git commit --amend --no-edit -S"` 重签名，`git log --format="%h %G?"` 全 G。默认实现 = 原 internal 代码，零行为改动；spawn_range_server（localhost 真 TCP）测试保留作穷举覆盖，fake 已可驱动同一套 download 机制（遗留 gap）
### 阶段 4（已完成，2026-08-22，7 个主题 7 个 commit，每步 `cargo test --workspace` 全绿）
- [x] **async 边界决策**：用户确认**方案 B + reqwest**（libscoop 全面 async，HTTP 从 ureq 换异步 reqwest 兑现真 async I/O）。关键发现：ureq 是阻塞客户端，仅改 async trait 拿不到并发，必须配 reqwest；git2/archive 是阻塞操作，async 化是形态统一，真 async 收益在 Network 兑现
- [x] **Session Sync 化**（`3a8f08c`）：`RefCell<Config>`→`RwLock<Config>`（`config()`→读 guard、`config_mut()`→`try_write` 保留 `ConfigInUse`）、`Cell<bool>`→`AtomicBool`、5×`OnceCell`→`OnceLock`；`Session` 变 `Sync+Send`（编译期断言测试）+ 修复持锁跨 await 的 3 处真实问题（network facade / download / checkver_hash / commit_one_install）
- [x] **bin 引入 tokio runtime**（`94713b3`）：root `Cargo.toml` 加 tokio；`create_app()` 建多线程 runtime 并 `block_on(cmd::start())`；37 个 `execute()` + `start()` 转 async，命令分发 match 臂 `.await`
- [x] **Network trait async + reqwest**（`931b627`，30 文件）：`internal/network.rs` 换 reqwest（rustls）+ async_trait；`RequestOptions`/`HeadResult`/`RangeResponse`/`RangeError` 保留；`range_get` 错误分类 Final/Transient 对齐原 ureq 语义；`FakeNetwork` 重建 async；全部调用点（network facade / checkver / download / download_fragmented / checkurls / checkhashes / create / checkver_hash / manifest_source / download_apps / download_verify / sync_install / sync）async 化；checkver Phase2 用 `buffer_unordered` 并发；download_fragmented 用 `buffer_unordered` 并行分片 + `tokio::time::sleep` 退避
- [x] **Git trait async**（`e87211a`）：`clone_repo`/`pull` 转 async（`remote_url_of`/`head_commit_time` 保持 sync——session 无关、rayon 并行场景直用 real git，记录为 gap）；`FakeGit` async；bucket add/update 转 async，update 并发 pull 用 `tokio::task::spawn`（git2 阻塞，offload 到 runtime worker）
- [x] **Archive trait async**（`6744129`）：`extract` 转 async（`detect_format` 保持 sync 纯函数）；operations/extract_archives/extract_markers + script run_script/run_hook + sync_install/sync_remove 管线 + cmd/reset 全部 async 化
- [x] **并发交付**：checkver（`buffer_unordered` 并行网络）、download（分片并行）、bucket update（`tokio::spawn` 并行 git pull）；checkup/search/list/status/which/config/cat 等本地快操作保持同步（query 用 rayon 并行扫盘，session 无关传 root 参数，沿用阶段 3 模式）
- [x] 验收：`cargo build --workspace` + `cargo test --workspace` 全绿（libscoop 433 passed）；clippy 回到基线 50（无新增）；本次改动 fmt 合规（既有 import 排序/尾部空行 diff 按技能保留）
- 注：`internal::git::tests::pull_fast_forward` 在并行/isolated 下偶发失败（git2/libgit2 环境抖动，重跑即绿，与已知 flake `pull_merge_commit` 同类）；本沙箱 gpg 需先设 `$env:HOME=$env:USERPROFILE`，`git log --format="%h %G?"` 全 G

### 阶段 5（已完成，2026-08-22，4 个主题 4 个 commit，每步 `cargo test --workspace` 全绿）
- [x] **消除可消除 unsafe**（`950489a`）：`internal/env.rs::to_utf16_bytes` 的 `from_raw_parts` 字节重解释（非 FFI）改用安全 `u16::to_le_bytes` 循环，输出逐字节等价，消除 libscoop 唯一一处可消除 unsafe
- [x] **`unsafe_code = deny` 落地**（`10d7cb4`）：root / libscoop / hok-i18n-derive / hok-shim 四 crate 的 `[lints.rust] unsafe_code = "deny"`；必要 FFI 模块级豁免：libscoop `internal::os`（Win32 进程/提权 15 处）+ `permission`（Windows ACL 3 处）加 `#![allow(unsafe_code)]`；hok-shim（no_std FFI 本质 47+3+3 处）按方案 B 用 crate 级 `#![allow(unsafe_code)]`（lib.rs + 2 bin）+ 理由注释；innospect 已有 deny 且其 1 处 `unsafe` 字样为注释误报（复核确认）。审计清单见 `.agents/docs/active/audit/unsafe-audit.md`
- [x] **criterion 基准**（`8280e52`）：libscoop 加 `criterion` dev-dep + `[[bench]] pure_functions`（`harness=false`）+ `benches/pure_functions.rs`；新增 `pub mod url`/`pub mod hash` 薄门面（对齐 fs/os/string，供 bench 访问纯函数）；覆盖 compare_versions / glob_to_regex / matches_any_glob / hash(md5/sha256/sha512) / manifest parse / url 工具，`cargo bench` 可跑（实测见报告）
- [x] **vs Scoop 脚本基准**（`ebe5100`）：新增 `scripts/benchmark_vs_scoop.ps1`（Start-Process+文件重定向+每例 20s 超时，网络/有状态命令经 `-IncludeNetwork` 开关+网络探针跳过）；对本地只读命令（list/search/info/which/prefix）跑 hok vs 官方 Scoop，捕获退出码/输出/耗时并输出 summary.csv + 逐例 .out；生成输出 gitignore
- [x] 验收：`cargo build --workspace` + `cargo test --workspace` 全绿（libscoop 433 passed；本沙箱 danger-full-access 下 ACL 2 例亦通过）；clippy 回到基线（libscoop 恰 50，无新增）；`git log --format="%h %G?"` 全 G
- 注：vs Scoop 基准中 scoop 的 list/search 在本沙箱因代理 `127.0.0.1:7890` 不可达而挂起（>20s，脚本按 TIMEOUT 优雅跳过，不报错）；info/which/prefix 退出码等价、hok 快 8~13×（ratio 0.06–0.12）；`which git` 观察到 hok 返回 shim 路径而 scoop 返回真实 exe 路径（记录为既有差异，非本阶段引入）；criterion 基准 `matches_any_glob`/`github_owner_repo` 因每调用 `Regex::new` 偏慢（~300–400µs），记录为后续优化候选，本阶段不改行为

### 阶段 4 收尾 · 移除 ureq（已完成，2026-08-22，2 个主题 2 个 commit）
- [x] **github.rs + virustotal.rs 迁 reqwest（async）**（`4c2d30a`）：阶段 4 已把 `internal/network.rs` 换成 reqwest（async），但 `internal/github.rs`（REST + GraphQL，auto_pr 用）与 `package/virustotal.rs`（check_url）仍是阻塞 ureq——async 边界内残留的"异步里阻塞"重复 HTTP 栈。本次全部迁到 reqwest（async，调用点本就在 async 上下文）：
  - REST/GraphQL 30s/60s 超时、Authorization/User-Agent/Accept 头、POST 发体、HTTP≥400 错误处理、404 检测语义原样保留
  - virustotal 轮询由 `std::thread::sleep` → `tokio::time::sleep`（真异步，不再阻塞 runtime 线程）
  - 调用点 `auto_pr.rs`（push/request 两路径）与 `cmd/virustotal.rs` 补 `.await`
  - `Cargo.toml` 删 `ureq`，`Cargo.lock` 无残留引用
  - 顺带修掉 auto_pr 的"异步里阻塞"问题（原 ureq 阻塞调用卡在 async 链路里）
- [x] **clippy 收敛**（`4a7542a`）：`cargo clippy --fix` 折叠 59 处 `collapsible_if`（libscoop + hok + hok-i18n-derive，26 文件），`cargo clippy --workspace --all-targets` 0 警告
- [x] 验收：`cargo build --workspace` + `cargo test --workspace` 全绿（libscoop 431 passed；permission ACL 2 例本沙箱环境失败，与 clean HEAD 相同、非回归）；virustotal async doctest 编译通过；改动文件 fmt 合规（github/auto_pr 显式 rustfmt，其余保留 clippy --fix 默认输出——既有 fmt 债文件，未整包 fmt 防无关 churn）；`git log --format="%h %G?"` 全 G（rebased 重签名）
- 注：auto_pr 是真 GitHub token + 网络的 CI 命令，本环境无网无法端到端验证，行为对齐靠代码审阅；每次调用新建 reqwest Client（对齐原 ureq 每调用新建 agent 的语义），连接池复用留作后续优化候选
### 纯净化 P1 · blocking 库包 spawn_blocking（已完成，2026-08，1 个 commit）
- [x] **RealGit::clone_repo/pull 包 spawn_blocking**（`a997ad6`）：git2 是阻塞库，原 async trait impl 直接调用阻塞 free fn，并发（bucket 并行 pull）下会占用 runtime worker。改为在 trait 层把整个克隆/拉取包进 `tokio::task::spawn_blocking`（owned String/PathBuf 移入闭包；闭包 panic 映射为 `Error::Custom`）。调用点零改动。
- [x] **RealArchive::extract 包 spawn_blocking**（`a997ad6`）：7z/zip/tar/rar 解压是 CPU+IO 密集阻塞操作，原在 async 内直接执行。改为 `spawn_blocking` 封装；flume `Sender<Event>` 是 `Send`，进度事件仍在阻塞闭包内照常发出。调用点零改动。
- [x] **rusqlite 确认非并发热点**（`a997ad6`）：manifest_cache（`open`/`populate`/`query`）经 `query_synced_cached` → `query_synced` → `query` 调用，是**每命令一次**的同步 SQLite 查询，非 `tokio::spawn` 扇出、非并发热点，不会在并发下饿死 runtime。按 plan「或确认在 async 内不阻塞 runtime」分支**保持同步未包** spawn_blocking，已在 `.agents/docs/reports/refactor/tokio-purity-plan.md` 记录。
- [x] **补并发守卫测试**（`a997ad6`）：`concurrent_pulls_succeed_via_blocking_pool` / `concurrent_extracts_succeed_via_blocking_pool`（join 两个并行 pull/extract，阻塞池下均成功）；`blocking_pull_does_not_starve_runtime` / `blocking_extract_does_not_starve_runtime`（阻塞操作进行时并发 async 定时器按时触发，证明 runtime worker 未被占用）。
- [x] 验收：`cargo build --workspace` + `cargo test --workspace` 全绿（libscoop 437 passed）；`cargo clippy -p libscoop --all-targets` 0 警告；本次改动 fmt 合规（既有 import 排序/尾部空行 diff 按技能保留）；`git log --format="%h %G?"` 全 G
- 注：git2 测试（含新并发测试）在 full workspace 并行跑时偶发 `BucketUpdateLocalChanges` / libgit2 环境抖动（同类于已知 `pull_merge_commit` flake），isolated 重跑即绿；并发测试对 clone2 做了 autocrlf 隔离（config 置 false + `checkout_head` FORCE 归一 LF）消除 CRLF 干扰
### 纯净化 P2 · CancellationToken + Ctrl-C 取消传播 + 单流续传（已完成，2026-08-22，6 个 commit，每步 `cargo test --workspace` 全绿）
- [x] **Session 挂载取消令牌 + create_app 接线 Ctrl-C**（`28f77f4`）：libscoop 加 `tokio-util`；`Session` 新增 `OnceLock<CancellationToken>` 槽位 + `cancellation_token()`/`set_cancellation_token()`（未注入时返回永不过期的默认 token，库调用方不受影响）+ `Error::CancellationTokenAlreadySet` + i18n；root tokio 加 `signal` feature + tokio-util；`create_app()` 起 `tokio::signal::ctrl_c()` 任务 → `cancel()`，`cmd::start(cancellation)` 挂到 session。测试：默认/注入/二次注入拒绝/取消传播。
- [x] **分片下载取消 + 保留续传状态**（`94ce17b`，取消清理语义 `(correction)` 修正）：`download_fragmented` 加 `&CancellationToken` 参数；已取消时先于建 part 立即失败；每个 chunk 的下载用 `tokio::select!` 与 `token.cancelled()` 竞速，取消即移除该 part；串行 fallback 同样 select!；取消后清掉全部剩余 part 并返回 error（不留 partial）。`PackageSet::download` 传 `session.cancellation_token()`。测试：预取消的下载失败且无 part 残留。
  - ⚠️ **复审修正**：原实现取消时删除全部 part 文件，与**跨次续传**契约相背——`.parts/` 在多次 `install/download` 间保留（download.rs 不清理 part_dir，`try_download_range` 从既有 part 大小续传）。修正后取消**保留** part 文件（续传状态），只中止下载并返回 error；fallback 同理（取消保留、真失败才删除）。测试改为 `test_download_fragmented_cancellation_preserves_resume_parts`（预置 part.0，取消后断言其仍在）。单流 `.download` 取消时保留（后续 `aa51fb3` 为单流实现 Range 续传，取消保留即续传状态）。
- [x] **脚本/子进程等待取消**（`2542224`）：`run_checkver_script` 转 async；`try_wait`+`std::thread::sleep` 循环换 async poll（`tokio::time::sleep` 与取消竞速），超时/取消都 `kill` 子进程；已取消时在 spawn 前即失败。`run_script`（install/uninstall hook）`ps.status()` → `spawn` + 可取消 poll，取消时 kill 子进程；**刻意不加硬超时**（Scoop 安装脚本可合法跑数分钟，加超时会改变行为）。测试：预取消的 checkver 脚本不 spawn 即失败。
- [x] **单流下载取消**（`010f811`）：`PackageSet::download` 的单流路径（estimated size / aria2 关闭）用 `tokio::select!` 与取消令牌竞速，取消时**保留** `.download`（供下次续传，与分片取消一致）。
- [x] **单流下载续传**（`aa51fb3`）：`RequestOptions` 加 `range_start`（默认 0=全新）；`download_stream` 对 `range_start>0` 发 `Range: bytes={n}-`，仅当 206 时 append 续传、服务端忽略 Range 返回 200 则 truncate 全量重下（防文件翻倍损坏）；进度从续传偏移起报（绝对总量）。`PackageSet::download` 单流路径保留 `.download` 算续传点、已完成 `.download` 直接 promote、续传后校验到满尺寸（短 206 报错）；取消保留 `.download`。`FakeNetwork` 镜像续传；测试：真实 `download_stream` 从 partial 续传且进度报绝对总量。
- [x] 验收：`cargo build --workspace` + `cargo test --workspace` 全绿（libscoop 441 passed）；`cargo clippy -p libscoop --all-targets` 0 警告；本次改动 fmt 合规（既有 import/尾部空行/checkver_hash 既有 fmt 债按技能保留）；`git log --format="%h %G?"` 全 G
- 注：事件循环光标恢复无需新增代码——`CursorGuard` RAII + 全路径 done-event（`PackageSyncDone`/`DownloadDone` 在错误路径也发送，见 sync.rs:425/download_apps.rs:122）+ bin `cursor::Show` 已保证取消后恢复；`RealArchive::extract`/`RealGit` 在 `spawn_blocking` 中执行，无法中断已在跑的阻塞解压/克隆（操作完成后下游感知取消），记录为 gap 不处理（解压相对快）
### 纯净化 P4 · 事件总线 flume→tokio::sync::mpsc + 事件循环线程→tokio task（已完成，2026-08-23，2 个代码 commit + 1 个 docs commit，最终 checkpoint 全绿）
- [x] **libscoop 事件总线迁 tokio::sync::mpsc**（`7d294dc`）：`EventBus` 两信道换 `tokio::sync::mpsc` bounded（容量保留 `EVENT_BUS_CAPACITY=20`）。outbound receiver 单消费者 → 存 `Mutex<Option<Receiver>>` 取用一次；inbound receiver 存 `tokio::sync::Mutex<Option<Receiver>>` 原地 await（`EventBus::receive_response` / `Session::receive_response`）。`emitter()` 返回 tokio Sender；全部事件产生点 `tx.send`→`tx.try_send`（保留 `let _ =` 忽略错误语义）。`confirm_transaction`/`select_candidate`/`resolve_dependencies` 转 async（await inbound 响应）。测试（`shim.rs` `conflict_count` `try_recv`、`sync.rs` `setup_confirm_test`、`resolve.rs` 全测试）适配 `tokio::test`。`flume` 依赖删除（Cargo.toml/Cargo.lock 无残留）。
- [x] **hok 事件循环线程→tokio task**（原引用：`1543517`；提交号已随 2026-08 重构失效落空，且无等价提交）：`run_event_loop` 改 `tokio::spawn` 返回 `tokio::task::JoinHandle`；主循环 `tokio::select!` 等后端事件；prompt/确认的 stdin 阻塞读包 `tokio::task::spawn_blocking` 防卡 runtime worker；响应 `tx.send().await`（保证不丢，backend 必在 await，容量 20 不挂死）。CLI 调用点 `handle.join()`→`handle.await`（download/install/reinstall/uninstall/update）；`update.rs` bucket 更新内联循环同样转 tokio task。`CursorGuard` RAII 收尾保留。
- [x] **docs 标记 P4 完成**（原引用：`026492c`；提交号已随 2026-08 重构失效落空，且无等价提交）：`.agents/docs/reports/refactor/tokio-purity-plan.md` 差距 6 与路线表 P4 改 ✅，补 P4 要点。
- [x] 验收：`cargo build --workspace` + `cargo test --workspace` 全绿（libscoop 442 passed，0 failed）；`cargo clippy --workspace --all-targets` 无新增 warning（既有 `bucket.rs` 5 处 `non-binding let` 为既有债，非本次）；`git log --format="%h %G?"` 全 G（gpg 签名）。
- ✅ **已解决（对齐 rake，`d967c7f`）**：下载进度事件增加 100ms 节流（`ProgressThrottle`，对齐 rake 的 `THROTTLE_DURATION`）+ 单流路径 `send().await` 背压（`Network::download_stream` 回调改 async boxed future），消除事件循环慢时的偶发丢弃/进度条跳变；分片（`buffer_unordered`）仍 `try_send` 但已节流。原差异：outbound 用 `try_send` 后，高频下载进度事件在事件循环消费慢于产生时可能偶发丢弃（原 flume `send` 阻塞起背压保证全量送达）；进度条因此可能偶现跳变，但正确性/幂等不受影响。

### 纯净化 P5 · bucket update 事件 send() 收尾（已完成，2026-08，1 个 commit）
- [x] **bucket.rs 5 处 `let _ = tx.send(...)` 改 `send().await`**（`b70f160`）：`bucket::update` 的 5 处非绑定 future（clippy `let_underscore_future`）全部加 `.await` 保背压——空桶分支 `BucketUpdateDone`（479）、并发 pull spawn 内 `BucketUpdateProgress` start/succeeded/failed（513/520/526）、收尾 `BucketUpdateDone`（549）。`let _ =` 保留（总线满/关闭静默忽略，与 P4 一致）；低频事件容量 20 不挂死，直接 await 未引入 select! 过度设计。行为等价：事件顺序（start→pull→result 每桶、Done 在全部 `handle.await` 后）、`last_update` 写回（541–546 不变）、并发 pull 语义、幂等全部不变。
- [x] 验收：`cargo clippy -p libscoop --all-targets` 0 warning（bucket.rs 5 个 `let_underscore_future` 清零）；`cargo build -p libscoop` + `cargo test -p libscoop --lib -- --skip internal::git` 全绿（439 passed, 0 failed）；改动 hunk fmt 合规（`rustfmt --edition 2024 --check` 仅剩既有 test 尾部/格式债 657/668 行，非本次，未动）；`git log --format="%h %G?"` 全 G（gpg 签名）。

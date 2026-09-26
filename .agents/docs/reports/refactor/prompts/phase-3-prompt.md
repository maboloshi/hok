# hok 重构阶段 3 开工提示词（新会话）

> 用法：新开对话后，把本文件整段贴给 agent。本文件由上一会话（2026-08-21）生成，配合 `.agents/docs/reports/refactor/plan.md` 使用。

## 任务
继续 hok 重构计划的**阶段 3：infra trait 化**——把 network / git / archive 抽成 trait + 默认实现 + 内存 fake，让单测脱离真实网络/磁盘。分支 `refactor/edition-2024`。每步保持 `cargo test --workspace` 全绿，按分组提交规范逐主题提交。

## 背景与现状（上一会话已完成）
- 阶段 0/1：edition 2024 全量升级（已完成，见 plan 文档执行日志）
- 阶段 2：domain 纯化（已完成，4 个 commit，每步全绿）：
  - `1fd926c` Manifest arch 选择纯化：13 个架构访问器有纯 `*_for(arch)` 变体 + 薄 host wrapper；顺带修复 ARM64 fallback 布尔反转 bug
  - `7ac7590` `Manifest::parse`→模块级 `manifest::parse_file`；`InstallInfo::parse`→`install_state::parse_install_info`（类型零 I/O）
  - `f63c58b` Config 纯化：`commit()`/写盘 `init()` 移除，改模块级 `config::write`/`config::init_default`；`set`/`set_alias` 纯内存变更器；`update_cooldown_remaining_at(now)` 纯变体
  - `3402c84` `Package::effective_version_at(now)` 纯变体 + 薄 wrapper
- **阶段 2 的纯化是阶段 3 的前提**：domain 层已纯，剩下的 I/O 全在 infra（network/git/archive）与 Session 挂载点，现在可以安全地 trait 化
- 工作树干净（仅 `.agents/docs/`、`AGENTS.md`、`scripts/` 等未跟踪文件，不入库）

## 开工必读
1. `.agents/docs/reports/refactor/plan.md`（阶段路线/风险保障/执行日志；完成后更新日志）
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`、`local-scoop-source-path`、`no-patchwork-fixes`、`hok-report-archive`（新会话先查根目录报告类文件）
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`，需要核对语义时直接读

## 阶段 3 目标与验收
- [ ] network / git / archive 三个 infra 各抽 trait，默认实现 = 现有 internal 代码原样包裹（**行为等价是生命线**，默认路径一个字节都不能变）
- [ ] Session 能注入 fake 实现（参考既有 `Session::set_output` / `set_default_architecture` 的注入模式），测试可构造"带内存 fake 的 session"
- [ ] 至少一类关键逻辑的单测改用内存 fake、脱离真实网络/磁盘（如 checkver 用 fake network 返回 canned 页面；bucket update 用 fake git；download 用 fake network 替代 spawn_range_server 的 localhost 服务）
- [ ] 每步 `cargo test --workspace` 全绿；最终全量 build+test 全绿 checkpoint

## 现状盘点（已扫描，新会话可直接引用）
**network（双层）**
- 门面 `crates/libscoop/src/network.rs`：`head_url(session, url, timeout, cookies)`、`download_file(session, url, dest)`、`download_page(session, url, timeout, token)` —— 注入 proxy/PRIVATE_HOSTS/UA/Referer
- 底层 `internal/network.rs`：`head(url, &RequestOptions)`、`download(url, &RequestOptions)`、`build_agent`、`RequestOptions`、`match_private_hosts`、`strip_filename`、`HeadResult`
- 直用底层的地方：`package/download.rs`(219/483/490/496)、`download_fragmented.rs`(355)、`checkver.rs`(374-390)
- 走门面的地方：`manifest_source.rs`(48)、`checkver_hash.rs`(9 处)

**git** `internal/git.rs`（git2/libgit2）：`clone_repo`、`pull`、`remote_url_of`、`head_commit_time`；使用者 `bucket.rs`(429 clone/501 pull/127 remote/140 head_commit_time)

**archive** `internal/archive.rs`：`extract(source, dest, ...)`、`detect_format(name)`；使用者 `package/operations/extract.rs`(45/74/158)

**测试基建** `test_utils.rs`：`tmpdir(name)`、`test_session(&root)`；`download.rs` 有 `spawn_range_server`（localhost 真 HTTP，阶段 3 的迁移目标之一）

## 建议实施步骤（每步一个 commit，先出计划再动手）
1. **trait + 默认实现（纯增量）**：定义 `Network`/`Git`/`Archive` trait 与默认实现（包裹现有 internal 函数，可带 session 无关的参数形状），不动调用点 → build+test 全绿
2. **Session 挂载**：Session 加 infra 槽位（默认真实实现）+ 访问器 + `set_*` 测试钩子（对齐 `set_output` 模式）→ 全绿
3. **调用点迁移**：把 `network.rs` 门面与直用 internal 的调用点改为走 session 的 trait 对象；git/archive 同理 → 全绿（注意 `internal/` 是工具层，**禁依赖 package/ 或领域服务**；trait 定义位置不能让依赖反向）
4. **内存 fake + 测试**：写 in-memory fake（canned 响应按 URL 分派；range/HEAD 语义仿真），把关键单测切到 fake 上（优先 checkver / bucket update / download）→ 全绿
5. **收尾**：更新 plan 文档执行日志；全量 `cargo build`+`cargo test --workspace` checkpoint

## 约束与纪律
- **行为等价**：默认实现 = 原 internal 代码，不允许借机改语义/修无关 bug（发现问题记 gap 清单，别悄悄带进）
- 提交规范：一个主题一个 commit（见 grouped-commits-preference；跨主题同文件用 `git apply --cached` 拆 hunk）；git 一律 `-c core.autocrlf=false`，eol=lf
- fmt：只修本次改动 hunk（仓库大量文件有既有 fmt diff，**禁整包 fmt**）；clippy 无新增 warning
- **提交前 review**（hok-proactive-review）：官方行为对齐/边界/架构合规/测试/提交内容完整性，逐项过完才 commit
- 阶段 3 期间仍是**零 async**（异步化是阶段 4 的事），trait 方法保持同步签名
- 测试真实性与性能：fake 只用于单测路径，默认实现保持不变

## 已知坑（直接避）
- `internal/git.rs` 的 pull 测试在 full workspace 并行跑时偶发失败（libgit2 环境抖动，isolated 重跑即绿）——fake git 落地后相关单测可完全脱离真实 git，这正是阶段 3 的价值之一；不要把它当回归去"修"
- `internal::archive` 是工具层唯一允许依赖 `crate::event` 的例外（提取进度上报），trait 化时保留该进度回调语义
- Session 含 RefCell/Cell/OnceCell 非 Sync：trait 对象若进 rayon 并行闭包会出问题（现有 `internal::path::bucket_dir` 就是为此传 root 参数），fake 实现同样遵循

## 完成时汇报
- 提交清单（commit hash + 主题）+ 每步全绿证据
- 哪些单测已脱离真实网络/磁盘（文件:测试名）
- 遗留/已知差异（gap 清单，不悄悄放过）
- 更新 `.agents/docs/reports/refactor/plan.md` 阶段 3 执行日志
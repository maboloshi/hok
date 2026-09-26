# P5 开工提示词：bucket update 事件 send() 收尾（对齐 rake 背压，新会话）

> 用法：新开对话后，把本文件整段贴给 agent。配合 `.agents/docs/reports/refactor/tokio-purity-plan.md` 与 `.agents/docs/reports/refactor/plan.md` 使用。
> 分支：`refactor/edition-2024`。前置：P1–P4 已完成 ✅（纯净化全链路 tokio 已落地；下载进度已对齐 rake：100ms 节流 + 单流 `send().await` 背压，见 commit `8009f1c`）。

## 目的
把 `bucket.rs` 里遗留的 5 处 `let _ = tx.send(...)` 非绑定 future（clippy `let_underscore_future` warning）收尾：这些调用都发生在 async 上下文（`bucket::update` 及其 `tokio::task::spawn` 内的并发 pull），应改为 `send().await` 保背压、消除 warning，对齐下载路径已完成的背压纪律。

## 背景与现状
- `crates/libscoop/src/bucket.rs` 的 `update(session)` 是 async，`session.emitter()` 返回 `tokio::sync::mpsc::Sender<Event>`
- 事件总线（P4 已迁）`tokio::sync::mpsc::bounded(EVENT_BUS_CAPACITY=20)`；`tx.send()` 返回 future，被 `let _ =` 丢弃 → clippy `non-binding let on a future` warning（5 处：`bucket.rs:479/513/520/526/549`）
- 事件：`BucketUpdateProgress`（并发 pull 的 start/succeeded/failed）、`BucketUpdateDone`（空桶分支 + 收尾）——均为低频事件
- 下载路径（`8009f1c`）已示范：单流进度回调改 async boxed-future `ProgressCallback` + `send().await`；`bucket.rs` 是同构的 async 调用点，直接 `send().await` 即可

## 开工必读
1. `.agents/docs/reports/refactor/tokio-purity-plan.md`（确认 P1–P4 现状）
2. 加载技能：`hok-code-organization`、`grouped-commits-preference`、`cargo-fmt-compliant-codegen`、`git-no-autocrlf-eol-lf`、`hok-proactive-review`、`no-patchwork-fixes`
3. 行为锚：官方 Scoop 在 `D:\App\github项目\Scoop\Scoop\lib|libexec`（bucket update 语义）
4. gpg：提交前 `$env:HOME=$env:USERPROFILE`

## 目标与验收
- [ ] `bucket.rs` 全部 `let _ = tx.send(...)` 改 `let _ = tx.send(...).await`（5 处）
- [ ] `cargo clippy -p libscoop --all-targets` 无新增 warning（bucket.rs 的 5 个 `let_underscore_future` 清零）
- [ ] **行为等价**：bucket update 的事件顺序、`last_update` 写回、并发 pull 语义、幂等完全不变
- [ ] `cargo build + cargo test -p libscoop --lib -- --skip internal::git` 全绿（git pull 测试是环境 flake，不在本步范围）
- [ ] 本次改动 fmt 合规（`rustfmt --edition 2024 --check` 仅对本次改动 hunk；禁整包 fmt）

## 行为（具体改动）
1. `bucket.rs::update`：
   - 空桶分支 `let _ = tx.send(BucketUpdateDone)` → `let _ = tx.send(BucketUpdateDone).await`
   - 并发 pull 的 `tokio::task::spawn(async move { ... })` 内 3 处 `BucketUpdateProgress`（start / succeeded / failed）`send` 加 `.await`
   - 收尾 `let _ = tx.send(BucketUpdateDone)` → `.await`
2. 注意 `spawn` 闭包内已 `move` 捕获 `emitter`，`tx.send().await` 在 async 块内直接可用，无需改动结构
3. `let _ =` 保留（事件总线满/关闭时静默忽略，与现有语义一致）；`send().await` 在低频事件下几乎不会挂起

## 约束与纪律
- **行为等价是生命线**：事件顺序、并发语义、`last_update` 写回分毫不能变；一个 commit，全绿再收尾
- `send().await` 满时挂起——bucket update 事件是低频（每桶几条），容量 20 足够，不会死锁；若担心可在 `tokio::select!` + 取消分支里包一层，但**不要为追求纯而引入过度设计**（低频事件直接 await 即可）
- 提交规范：一个主题一个 commit；`git -c core.autocrlf=false`；只修本次改动 hunk（禁整包 fmt）
- 提交前 review（hok-proactive-review）：官方行为对齐/边界/架构合规/测试/提交内容完整性

## 已知坑（直接避）
- 别把 `bucket.rs` 里**其它**既有债（如 import 排序、let-chain 格式）混进本主题——只改这 5 处 `send().await`
- git pull 测试（`internal::git::tests::*`）在本沙箱偶发失败是**环境级 libgit2 抖动**，非本步回归；用 `--skip internal::git` 验证其余全绿
- 事件顺序：`BucketUpdateProgress` 的 start/succeeded/failed 与 `BucketUpdateDone` 的先后不能被并发改变——`for handle in handles { handle.await }` 已保证收尾在全部 pull 后，别动这个编排

## 完成时汇报
- 提交清单（commit hash + 主题）+ `clippy` 清零证据（bucket.rs 5 个 warning 消失）
- 5 处 `send().await` 逐一对应（行号 + 事件类型）
- 行为等价验证证据（事件顺序 / last_update / 并发 pull）
- 遗留/已知差异（gap 清单，不悄悄放过）
- 更新 `.agents/docs/reports/refactor/tokio-purity-plan.md`（如有 P 项状态）并同步 `.agents/docs/reports/refactor/plan.md` 执行日志（docs 不入库，仅工作树）

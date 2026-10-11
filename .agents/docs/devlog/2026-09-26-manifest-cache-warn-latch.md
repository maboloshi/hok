# 清单缓存告警闩锁搬家：`serving_a_stale_cache_warns_once` flaky 连根修

> 2026-09-26 ｜ 1 提交 `9af599a`，worktree `.worktrees/s-warn-latch`，分支 `fix/manifest-warn-latch`

**起因**：挂账 flaky —— `cargo test -p libscoop cache::` 并行偶发红、失败点恒为 `manifest_cache.rs` 的 `a stale cache must warn`，`-- --test-threads=1` 即绿。

**复现（主线 `7b92635` clean 树）**：同机 25 次并行跑该 filter **15/25 失败**（挂账记录 2/25，方向一致、当时负载更高），失败输出与挂账逐字相同（`23 passed; 1 failed; 772 filtered out`）。

**根因（闩锁层 ≠ sink 层）**：`STALE_WARNED: AtomicBool` 是**进程级** static，告警却写进**每会话**的 `session.output()`。两处错位叠加：① `warn_if_cache_stale` 先 `swap(true)` 再判断 stale ⇒ **新鲜缓存也吃掉闩锁**（产品语义缺陷：同一进程里"新鲜查询在前、stale 查询在后"时，后者的告警被静默）；② 并行测试里别的用例（`explicit_query_*`、`wildcard_query_stays_cached` 都走 `query.rs` 的 `query_synced_cached`）先 swap 掉，本用例 `reset_stale_warning()` 之后、`query_synced()` 之前被抢 ⇒ 拿到空 warnings。`MISMATCH_WARNED: Mutex<Vec<String>>`（key=`bucket/name`）同型；两个 `reset_*_warning()` 只能 `#[cfg(test)]`，正是"闩锁放错层"的信号。

**改动**（`crates/libscoop/src/session.rs` + `crates/libscoop/src/package/manifest_cache.rs`，用户拍板 A+B）：
- 两个闩锁搬进 `Session`：`stale_warned: AtomicBool`、`mismatch_warned: Mutex<HashSet<String>>`，由 crate 内可见的 `mark_stale_warned()` / `mark_mismatch_warned(key)` 负责"抢一次"（前者 `swap`，后者锁内 check-and-insert）；`new()` 两条路径 + `new_with()` 三个构造点同步补齐。
- `warn_if_cache_stale` 改为**先判 stale、再抢闩锁**，修掉缺陷①；`warn_if_cache_mismatch` 换成会话内 `HashSet` key。
- 删掉两个 `#[cfg(test)]` `reset_*_warning()` 与全部调用。i18n 文案零改动（`cmd.cache_stale` / `cmd.cache_mismatch` / `cmd.cache_missing` 逐字保持）。
- CLI 一进程一会话（`src/cmd/mod.rs`）⇒ 屏幕行为不变；`query.rs` 调用点签名不变。

**验证**：改后 20 次并行 `cargo test -p libscoop cache::` **0 失败**；`cargo test -p libscoop` **789 passed / 0 failed**、`cargo test -p hok` 全绿、`cargo clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings` 干净、改动文件 `rustfmt --edition 2024 --check` 干净。新增 3 条测试钉住语义（`a_fresh_query_keeps_the_stale_notice_for_later`、`every_session_warns_about_the_stale_cache`、原用例补"会话内不重复"），并做**反向验证**：临时把 `mark_stale_warned` 换回进程级 static ⇒ `every_session_warns_about_the_stale_cache` 确定性失败；临时把 `swap` 提到 stale 判断之前 ⇒ `a_fresh_query_keeps_the_stale_notice_for_later` 确定性失败（两处临时改动均已回退）。落地 `refactor/edition-2024`（`9af599a`，脚本漂移校验：1 提交 / 主题 / 2 文件集守恒 + 树逐字节相同，无 merge commit）。

**坑/教训**：① **"once" 状态必须与它作用的对象同生命周期**：告警发给哪个 sink，就该由哪个会话记账；进程级 static + 每会话 sink 必然在第二个会话上出错（产品里表现为嵌库/多会话丢告警，测试里表现为 flaky）。② **先抢闩锁再判条件**是这类缺陷的常见形态：还没确认"这次真的要告警"，就把唯一额度花在无事发生的路径上。③ flaky 的"并行互抢"与"语义缺陷"常是**同一个错位的两面**：修互抢顺手把语义一起修掉；只改 `swap` 顺序则 flaky 仍在。④ **反向验证**（把状态临时改回旧形态、看新用例确定性变红）是"测试确实钉住回归"的廉价证据，比多跑几遍有力得多。

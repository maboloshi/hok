---
name: hok-dynamic-chunking-todo
title: 动态分块：方案A+进度事件已落地
description: 动态分块方案A（失败区间兜底 142e402）+ 分片进度事件（d517bbd）已实现提交；方案B未做；余下已知项清单
metadata:
  type: project
---

**目标**：`crates/libscoop/src/package/download.rs` 把静态分块改为按失败率自适应的"动态分块"。方案 A（失败区间兜底）与进度事件**已实现并提交**；方案 B（跨文件失败率反馈）未做。

**Why**：静态分块的最大脆弱点是"单 chunk 重试 3 次耗尽 → 整文件失败"；且分片路径无进度反馈。两者已修复。

**How to apply**（已落地）：
- **方案 A — 失败区间兜底（commit `142e402`）**：`download_fragmented()` 提取为独立函数，chunk 重试耗尽后失败 range 收集进 `Arc<Mutex<Vec<(u64,u64)>>>`（失败时删除残留 part），`merge_adjacent_ranges()` 合并相邻区间后**单连接串行补下**（复用 `download_range`，自身带重试）；part 跟踪改为 `Vec<(PathBuf, u64, u64)>`（path, start, end），校验改为排序后逐段检查"大小 = end-start+1 且无空洞、总覆盖 = remote_size"。part/resume 模型未变（part 仍按固定 range 命名，`part.{idx}` + `fallback.{idx}`）。
- **进度事件（commit `d517bbd`）**：`download_range`/`try_download_range` 加 `on_progress: &(dyn Fn(u64) + Sync)` 参数（`io::copy` 改为显式 32KB read 循环，每字节恰报一次）；`download_fragmented` 透传；`download()` 用 `Arc<AtomicU64>` 聚合 delta 为文件级 dlnow + emitter clone 跨线程发 `Event::PackageDownloadProgress`（dltotal=remote_size，与单流路径一致）。**顺手删除了 `download_range`/`download_fragmented` 从未使用的 `proxy` 参数**（proxy 已 baked into agent），两函数回到 7 参数以内，clippy 无新增警告。
- 测试基建：`spawn_range_server` 加 `fail_range: Option<(usize,usize,u32)>`（匹配 range 的请求前 N 次返回 fail_status）；新增 4 个测试：`test_merge_adjacent_ranges`、`test_download_fragmented_fallback_recovers_failed_chunk`、`test_download_range_reports_progress`、`test_download_fragmented_reports_total_progress`。download 模块共 16 个测试，全套 251 个通过。

**未做（下次可选）**：
- **方案 B — 跨文件失败率反馈**：`PackageSet` 级 `effective_chunk_count`，文件失败减半（下限 2）、成功恢复；需把 `chunk_count` 从 `download()` 外层（L199）移到 per-file 循环内。归因依据弱（失败多半是链路抖动而非并发过多），优先级低。
- **方案 C（已否决）**：运行中动态并发窗口，与 resume 语义冲突。

**其他已知项（workshop 综述，未处理）**：`load_cache` 硬编码 remote_size=0 导致 cache 命中弱；`calculate_download_size` 双重 HEAD；cookie 格式化三处重复；`update_valid_state` 冗余。

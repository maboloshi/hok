# hok 开发日志

> **用途**：记录开发过程的关键决策、踩坑、性能数据与实验结论——面向维护者与后续开发会话。
> **定位**：与 `CHANGELOG.md`（用户可见变更）、`.agents/skills/`（agent 操作知识）互补；本文档回答"为什么这么做、踩过什么坑、数据是什么"。
> **CHANGELOG 分工**：`CHANGELOG.md` **仅在发布前统一更新**（从 devlog + git log 提炼，Keep a Changelog 风格按版本/类别整理）；开发过程中不动 CHANGELOG，所有过程记录写在这里。
> **维护**：一条日志 = 一个文件 `YYYY-MM-DD-<english-slug>.md`；首行 `# <主题>`，第 3 行 `> <日期> ｜ 提交 … ｜ 分支 …`。
> 本索引由 `node .agents/scripts/devlog-index.mjs` 生成，**禁手改**（规则见 [[docs/AGENTS.md]] R7）。

## 索引（51 条，自动生成）

| 日期 | 主题 | 提交 | 文件 |
|---|---|---|---|
| 2026-09-27 | `cat_style` 是 bat 的 `--style` 值：吞掉的退出码 + 写入期求证 | `95fb819` | [`2026-09-27-bat-style-fallback.md`](2026-09-27-bat-style-fallback.md) |
| 2026-09-26 | 技能内容级审查：断言过期 / 重复 / description / 同名归档件 | `2806108` `1a9249a` `8c97220` | [`2026-09-26-skill-content-audit.md`](2026-09-26-skill-content-audit.md) |
| 2026-09-26 | 规则复述收敛（7 组）与根 AGENTS.md 两处措辞 | `9eea4a6` `c2fa597` `e5c516f` | [`2026-09-26-rule-restatement-convergence.md`](2026-09-26-rule-restatement-convergence.md) |
| 2026-09-26 | 清单缓存告警闩锁搬家：`serving_a_stale_cache_warns_once` flaky 连根修 | `9af599a` | [`2026-09-26-manifest-cache-warn-latch.md`](2026-09-26-manifest-cache-warn-latch.md) |
| 2026-09-26 | 日志触发的收口与规则载体的闸门锚点审计 | `74de6a1` `c186cf5` `e1a2bce` `d3d6ffb` | [`2026-09-26-devlog-trigger-and-gate-anchors.md`](2026-09-26-devlog-trigger-and-gate-anchors.md) |
| 2026-09-25 | 下载超时/哈希详情/缓存命中三点修正 + 延迟校验竞态登记 | `4b2004b` `d49d6dd` `cdcfab1` `6620d69` | [`2026-09-25-download-timeout-hash-cache.md`](2026-09-25-download-timeout-hash-cache.md) |
| 2026-09-25 | `hok download <合法本地 manifest>` 主线程栈溢出 | `6620d69` | [`2026-09-25-download-stack-overflow.md`](2026-09-25-download-stack-overflow.md) |
| 2026-09-25 | `.parts` 残留分片清理：`remove_incomplete` 补上分片下载那半 | `7b92635` | [`2026-09-25-cleanup-parts-residual.md`](2026-09-25-cleanup-parts-residual.md) |
| 2026-09-25 | `cleanup -k` 缓存面三修：不可见的名字 / 坏安装 / 零输出 | `ff05b80` `cb75ba4` | [`2026-09-25-cleanup-cache-names.md`](2026-09-25-cleanup-cache-names.md) |
| 2026-09-25 | 7z 解压静默丢弃「无流条目」 | `4bf1397` | [`2026-09-25-7z-empty-entries.md`](2026-09-25-7z-empty-entries.md) |
| 2026-09-24 | git 子系统收尾小修 | `daa1002` `675b991` | [`2026-09-24-git-subsystem-polish.md`](2026-09-24-git-subsystem-polish.md) |
| 2026-09-22 | `cleanup -l/--list` + 删私有 `--verbose` + cache 帮助面翻译 | `cd67788` `65b83ee` | [`2026-09-22-cleanup-list-and-cache-help.md`](2026-09-22-cleanup-list-and-cache-help.md) |
| 2026-09-20 | `hok install` persist 链接 183：解压副本与已存在的 persist 目标撞车 | `8aa3dc5` `16a91cd` | [`2026-09-20-install-persist-link-183.md`](2026-09-20-install-persist-link-183.md) |
| 2026-09-20 | `cleanup` 并行化 + 逐版本进度 | `3bca476` `54a87d0` `2ad6e9d` `e8c09f1` | [`2026-09-20-cleanup-parallel-progress.md`](2026-09-20-cleanup-parallel-progress.md) |
| 2026-09-19 | 上游 innospect 0.1.4 收录 PR #3，退役 vendored fork | — | [`2026-09-19-innospect-registry-retire-fork.md`](2026-09-19-innospect-registry-retire-fork.md) |
| 2026-09-13 | Wave 1：`checkver` 挂起 + 无 home panic | `4748ae1` `43795484` | [`2026-09-13-wave1-checkver-hang.md`](2026-09-13-wave1-checkver-hang.md) |
| 2026-09-13 | update/upgrade 对齐：删冷却、`-i`、hold 口径、引擎归位 | `ab7ea6e` `77475e3` | [`2026-09-13-update-upgrade-alignment.md`](2026-09-13-update-upgrade-alignment.md) |
| 2026-09-13 | S9 `checkver` 深度对齐 | `8ad7d080` | [`2026-09-13-s9-checkver-alignment.md`](2026-09-13-s9-checkver-alignment.md) |
| 2026-09-13 | S8 路径解析类 | `c5b21d2` `41ddaeb` `4b25087` `4b250876` | [`2026-09-13-s8-path-commands.md`](2026-09-13-s8-path-commands.md) |
| 2026-09-13 | S7：低风险批量 | — | [`2026-09-13-s7-low-risk-batch.md`](2026-09-13-s7-low-risk-batch.md) |
| 2026-09-13 | S6：`cleanup`/`cache` 数据安全 | `1e6b929` `33ba858` `1c1311b` `cc2c072` | [`2026-09-13-s6-cleanup-cache-safety.md`](2026-09-13-s6-cleanup-cache-safety.md) |
| 2026-09-13 | S5：`checkup` 对齐官方环境检查 + 新增 `health` | `5ae994d` | [`2026-09-13-s5-checkup-health.md`](2026-09-13-s5-checkup-health.md) |
| 2026-09-13 | S4 `config` 键策略与子命令语义 | `38ecfa9` `f4300aa` `4cefb81` `b7c1680` `7987205` `98e28a1` `cf1baec` | [`2026-09-13-s4-config-keys.md`](2026-09-13-s4-config-keys.md) |
| 2026-09-13 | S3：`export`/`import` 与官方 Scoopfile 互通 | `a61a471` `110e160` `2dd84b2` `14b9c29` | [`2026-09-13-s3-export-import-interop.md`](2026-09-13-s3-export-import-interop.md) |
| 2026-09-13 | S2 `alias` 可用性 | `3c90095` | [`2026-09-13-s2-alias-availability.md`](2026-09-13-s2-alias-availability.md) |
| 2026-09-13 | S14 `bucket` / `shim` 参数与校验 | `dca2b51e` | [`2026-09-13-s14-bucket-shim-args.md`](2026-09-13-s14-bucket-shim-args.md) |
| 2026-09-13 | S11 第一批 | `2a18d5b` | [`2026-09-13-s11-formatjson-missing-checkver.md`](2026-09-13-s11-formatjson-missing-checkver.md) |
| 2026-09-13 | S10 `checkhashes` 深度对齐 | `e97744a` | [`2026-09-13-s10-checkhashes-alignment.md`](2026-09-13-s10-checkhashes-alignment.md) |
| 2026-09-13 | S1+S13 退出码口径 + 安装/卸载批量行为 | — | [`2026-09-13-s1-s13-exit-code-batch-policy.md`](2026-09-13-s1-s13-exit-code-batch-policy.md) |
| 2026-09-13 | 运行中进程输出 + `ignore_running_processes` 语义回正 | — | [`2026-09-13-running-process-output.md`](2026-09-13-running-process-output.md) |
| 2026-09-13 | 下载默认不发 Referer | `cf260d9` `4748ae1` | [`2026-09-13-download-referer-removal.md`](2026-09-13-download-referer-removal.md) |
| 2026-09-13 | `-d` 写盘护栏 | `be0092df` | [`2026-09-13-dflag-write-guard.md`](2026-09-13-dflag-write-guard.md) |
| 2026-09-13 | D4 下载请求头 | `6db6bfbd` | [`2026-09-13-d4-download-headers.md`](2026-09-13-d4-download-headers.md) |
| 2026-09-12 | 表格输出统一：cache / bucket / config 表格化 + 统计位置规则 | `1fbcf42` `23b4401` `1c16d69` `a8d308f` | [`2026-09-12-table-output-unification.md`](2026-09-12-table-output-unification.md) |
| 2026-09-12 | 清单缓存静默漂移修复：marker 基线 + 提示层 | `7b1effe` `8a34463` | [`2026-09-12-manifest-cache-marker-baseline.md`](2026-09-12-manifest-cache-marker-baseline.md) |
| 2026-09-12 | 清单缓存：命中即校验 + 桶更新失败可见性 | `c2e06df` `5515989` `3d4969f` | [`2026-09-12-manifest-cache-hit-verify.md`](2026-09-12-manifest-cache-hit-verify.md) |
| 2026-09-12 | `hok info` 全面对标官方 | — | [`2026-09-12-info-official-alignment.md`](2026-09-12-info-official-alignment.md) |
| 2026-09-12 | `info` 多行值的第 2 列对齐 | `fe6ad82` | [`2026-09-12-info-multiline-align.md`](2026-09-12-info-multiline-align.md) |
| 2026-09-12 | `info` 的 git 历史查询重写：树条目跟随，不 spawn 外部 git | — | [`2026-09-12-info-git-history-rewrite.md`](2026-09-12-info-git-history-rewrite.md) |
| 2026-09-12 | checkver 取哈希链路复核 | — | [`2026-09-12-checkver-hash-chain-review.md`](2026-09-12-checkver-hash-chain-review.md) |
| 2026-09-09 | 输出配色统一为单一「主题安全」调色板 | — | [`2026-09-09-output-theme-safe-palette.md`](2026-09-09-output-theme-safe-palette.md) |
| 2026-08-29 | 阶段 3b 性能基准：细粒度事件发射开销可忽略，推迟源头发射门控 | — | [`2026-08-29-phase3b-emit-gating-deferred.md`](2026-08-29-phase3b-emit-gating-deferred.md) |
| 2026-08-28 | update 误判「全部已缓存」导致 verify 文件不存在 | — | [`2026-08-28-update-false-all-cached.md`](2026-08-28-update-false-all-cached.md) |
| 2026-08-23 | `hok update` 无包名并发 bucket 同步改用 spinner UI | `2b9f5d0` | [`2026-08-23-update-bucket-sync-spinner.md`](2026-08-23-update-bucket-sync-spinner.md) |
| 2026-08-23 | 单步进度统一为 TTY 感知 | `f09d9e0` | [`2026-08-23-single-step-progress-tty.md`](2026-08-23-single-step-progress-tty.md) |
| 2026-08-23 | 纯净化 P5：bucket update 事件 send() 收尾 | `1e01967` | [`2026-08-23-purity-p5-bucket-update-send.md`](2026-08-23-purity-p5-bucket-update-send.md) |
| 2026-08-23 | i18n 一致性安全网 | — | [`2026-08-23-i18n-key-parity-net.md`](2026-08-23-i18n-key-parity-net.md) |
| 2026-08-23 | 下载进度对齐 rake：100ms 节流 + 单流 send() 背压 | `8009f1c` | [`2026-08-23-download-progress-throttle.md`](2026-08-23-download-progress-throttle.md) |
| 2026-08-21 | update verify/extract 缓存文件名不一致 + zip 反斜杠解压修复 | `fdd1114` `9e91b31` | [`2026-08-21-update-cache-name-mismatch.md`](2026-08-21-update-cache-name-mismatch.md) |
| 2026-08-16 | 快捷方式参数丢失修复 | `5beedb7` | [`2026-08-16-shortcut-args-loss.md`](2026-08-16-shortcut-args-loss.md) |
| 2026-08-15 | NSIS 提取、兼容修复、进度条与 7z 性能 | — | [`2026-08-15-nsis-extraction-and-7z-perf.md`](2026-08-15-nsis-extraction-and-7z-perf.md) |

# System Regression Report — Round 1

- Date: 2026-07-31
- Executor: maboloshi
- Branch/Commit: copilot/system-analysis-framework-construction / 0b376be
- Environment (OS/Arch): _(待填写，例：Windows 11 x64)_
- Scoop root mode (user/global): _(待填写)_

> **执行说明**：本轮为混合评估报告。  
> - `[静态分析]`：当前轮次在 Linux CI 沙箱中完成静态代码分析，给出实现状态与已知缺口，**需在 Windows 环境补充实际运行结果**。  
> - `Blocked (环境)` 表示当前 Linux 沙箱无法运行 Windows 二进制，并非功能缺陷。

## 1) Scope

- [x] P0
- [x] P1
- [ ] P2

## 2) Summary

- Total: 29 _(P0: 17, P1: 12)_
- Passed: — _(待 Windows 实测)_
- Failed: 0 _(静态分析未发现阻塞缺陷)_
- Blocked: 29 _(全部需 Windows 环境实测)_
- Pass rate: — _(待填写)_

## 3) Failed / Blocked Cases

| Case ID | Severity | Symptom | Repro Steps | Evidence Path | Issue Link |
|---|---|---|---|---|---|
| P1-MAN-001 | Medium | `checkver.useragent` 字段已解析但未在 HTTP 请求中应用 | 构造含 `checkver.useragent` 的清单执行 `hok checkver` | `src/cmd/checkver.rs:185` | — |
| P1-MAN-001 | Low | `--throw` 参数未实现，错误仅打印不抛出 | 执行 `hok checkver --throw` | `src/cmd/checkver.rs:28-46` | — |
| P0-TXN-007 | Low | shim 生成时 GUI `.exe` PE subsystem 检测未实现（所有 `.exe` 均生成控制台 shim） | 安装含 GUI exe 的包后检查 shim 行为 | `crates/libscoop/src/shim.rs:20,147` | — |

## 4) Detailed Results

> 列含义：**静态评估** = 代码分析结论；**需实测** = Windows 运行结果（待填）。

### P0：核心交易链路

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P0-TXN-001 | Blocked (环境) | — | — | — | [静态分析] `install.rs`：`package_prune_installed` 跳过已装包并警告，`package_sync` 完整实现；`--global`/`-g` 权限检查实现完整。需 Windows 实测。 |
| P0-TXN-002 | Blocked (环境) | — | — | — | [静态分析] `SyncOption::NoDependencies` 存在（默认不传），依赖递归安装由 `libscoop::operation` 内部处理。需 Windows 实测。 |
| P0-TXN-003 | Blocked (环境) | — | — | — | [静态分析] `is_admin()` 判断后 `bail!` 返回权限错误，不写入全局目录——逻辑正确。需 Windows 实测确认退出码与提示文案。 |
| P0-TXN-004 | Blocked (环境) | — | — | — | [静态分析] `update_buckets()` 实现 15 分钟 cooldown（`update_cooldown_remaining`），`--force` 可绕过；bucket 更新后自动刷新 SQLite cache。逻辑完整。 |
| P0-TXN-005 | Blocked (环境) | — | — | — | [静态分析] `execute_upgrade()` 支持 `*` 通配符，`SyncOption::OnlyUpgrade` 过滤未升级包；`--force` 可绕过。多包路径逻辑完整。 |
| P0-TXN-006 | Blocked (环境) | — | — | — | [静态分析] `reinstall.rs`：先查询 held 状态、释放 hold → uninstall → install → 重新 hold，状态保持逻辑正确。需 Windows 实测验证 held 状态保持。 |
| P0-TXN-007 | Blocked (环境) | — | — | — | [静态分析] `package_reset` → `sync::reset` 已实现。**已知缺口**：GUI exe 的 PE subsystem 检测未实现（shim.rs:147），GUI 应用的 shim 行为需重点关注。 |
| P0-TXN-008 | Blocked (环境) | — | — | — | [静态分析] `uninstall.rs`：`SyncOption::Remove` + 可选 `Purge`/`EscapeHold`/`IgnoreFailure`，实现完整。 |
| P0-TXN-009 | Blocked (环境) | — | — | — | [静态分析] `cleanup.rs`：`package_cleanup(session, &apps, ignore_failure=true)` 返回 `(name, removed_count, failed_count)` 统计，输出正确。 |
| P0-TXN-010 | Blocked (环境) | — | — | — | [静态分析] `SyncOption::IgnoreFailure` 注释明确："失败包停止+回滚，其余包继续"。`--ignore-failure` 在 install/uninstall/reinstall 均已接入。 |
| P0-TXN-011 | Blocked (环境) | — | — | — | [静态分析] `SyncOption::Offline` 存在于枚举中，`install.rs` 的 `-o`/`--offline` 正确映射。需 Windows 实测确认缓存命中路径。 |
| P0-TXN-012 | Blocked (环境) | — | — | — | [静态分析] `SyncOption::IgnoreCache` 存在，`install.rs` 的 `-D`/`--ignore-cache` 正确映射。注释明确不应与 `Offline` 同用。 |

### P0：仓库与索引

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P0-IDX-001 | Blocked (环境) | — | — | — | [静态分析] `bucket.rs`：`bucket_add` 有重复检查（`Already exists` error），`bucket_remove` 和 `bucket_list` 均已实现。 |
| P0-IDX-002 | Blocked (环境) | — | — | — | [静态分析] `bucket_update` + `last_update` 持久化逻辑在 `config.rs` 中完整实现。冷启动路径无 cooldown 阻断。 |
| P0-IDX-003 | Blocked (环境) | — | — | — | [静态分析] `update_cooldown_remaining()` 返回剩余秒数，`--force` 绕过逻辑在 `update.rs` 实现正确。 |
| P0-IDX-004 | Blocked (环境) | — | — | — | [静态分析] `use_sqlite_cache` 配置项、`refresh_manifest_cache()` 均已实现；bucket 更新后自动刷新。需 Windows 实测 `scoop.db` 文件创建/重建行为。 |
| P0-IDX-005 | Blocked (环境) | — | — | — | [静态分析] `operation.rs` 中有去重逻辑；`SyncOption::AssumeYes` 触发内置选择算法跳过交互提示。需 Windows 实测多 bucket 同名包的候选选择行为。 |

### P1：查询与可观测

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P1-OBS-001 | Blocked (环境) | — | — | — | [静态分析] `search.rs`：支持正则（默认）和 `--explicit` 精确匹配；`--with-binary`/`--with-description` 扩展搜索域；`QueryArgs::to_query_options()` 已重构。 |
| P1-OBS-002 | Blocked (环境) | — | — | — | [静态分析] `list.rs`：CJK 字符宽度修正（`visual_width`/`pad_visual`），已安装版本与可升级信息由 `package_query` 提供。 |
| P1-OBS-003 | Blocked (环境) | — | — | — | [静态分析] `info.rs` / `depends.rs` 均已实现；`manifest.suggest()` 字段在 install 后展示建议。 |
| P1-OBS-004 | Blocked (环境) | — | — | — | [静态分析] `which.rs`/`prefix.rs`/`shim.rs`(cmd) 均已实现，通过 `package_query` 获取路径信息。 |

### P1：清单质量工具

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P1-MAN-001 | Blocked (环境) | — | — | — | [静态分析] `checkver.rs` 实现完整，支持 jsonpath/regex/github/gitlab 等策略。**已知缺口（低优先）**：`checkver.useragent`/`Referer`/`PRIVATE_HOSTS` 未实现（见文件头 TODO 块）。 |
| P1-MAN-002 | Blocked (环境) | — | — | — | [静态分析] `checkurls.rs`（169 行）：递归扫描 + 超时参数实现。需 Windows 网络环境实测。 |
| P1-MAN-003 | Blocked (环境) | — | — | — | [静态分析] `checkhashes.rs`（395 行）：hash 校验与回写逻辑完整；空 hash 和 `"TODO"` 字符串有专门处理（`checkhashes.rs:168`）。 |
| P1-MAN-004 | Blocked (环境) | — | — | — | [静态分析] `formatjson.rs`（123 行）：格式化逻辑实现；幂等性需 Windows 实测第二次执行无差异。 |
| P1-MAN-005 | Blocked (环境) | — | — | — | [静态分析] `cat.rs` / `create.rs`（109 行）均已实现。需 Windows 环境验证输出结构。 |

### P1：配置与可移植性

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P1-CFG-001 | Blocked (环境) | — | — | — | [静态分析] `config.rs`（cmd）：Set/Unset/List/Edit 全覆盖；`config.rs`（lib）持久化 JSON 结构完整，别名键兼容。 |
| P1-CFG-002 | Blocked (环境) | — | — | — | [静态分析] `alias.rs`（cmd）：`alias_add`/`alias_remove`/`alias_list` 均通过 `operation` 层操作 config，立即生效。 |
| P1-CFG-003 | Blocked (环境) | — | — | — | [静态分析] `hold.rs`/`unhold.rs`：`package_hold(session, name, true/false)`，`reinstall` 中 held 状态恢复逻辑完整；`SyncOption::EscapeHold` 在 upgrade 时生效。 |
| P1-CFG-004 | Blocked (环境) | — | — | — | [静态分析] `export.rs`：导出 buckets（name→url）+ apps（bucket→{name→version}）JSON 结构；`import.rs`：读取相同结构并执行 install。需 Windows 实测端到端往返一致性。 |
| P1-CFG-005 | Blocked (环境) | — | — | — | [静态分析] `completions.rs` 已实现，shell 目标由参数指定。需 Windows 实测生成脚本可用性。 |

## 5) Exit Criteria Check

- [ ] P0 all passed
- [ ] P1 pass rate >= 95%
- [ ] No data-damage issues
- [ ] No blocking failures on x64/x86/ARM64

## 6) Conclusion

- Release recommendation: _(待 Windows 实测后填写)_
- Risk notes:
  - 静态分析未发现 P0 级阻塞缺陷；所有核心命令（install/uninstall/update/reinstall/reset/cleanup）均有完整实现路径。
  - **已知低优先级缺口**：①GUI `.exe` shim 的 PE subsystem 检测未实现（所有 exe 均产生控制台 shim，影响部分 GUI 应用快捷方式行为）；② `checkver.useragent`/Referer/PRIVATE_HOSTS 未在 HTTP 请求中应用。
  - 上述两个缺口均来自已有代码注释（`TODO`），不属于回归新引入问题，建议作为已知差距在缺陷单中跟踪，不阻塞当前发布评估。
  - 全部 29 条用例仍需在 Windows x64/x86/ARM64 三架构上实际运行，补充 Exit Code / Key Output / State Verification 后才能给出最终发布建议。

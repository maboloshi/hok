# System Regression Report — Round 1

- Date: 2026-07-31
- Executor: maboloshi
- Branch/Commit: copilot/system-analysis-framework-construction / 0b376be
- Environment (OS/Arch): _(待填写，例：Windows 11 x64)_
- Scoop root mode (user/global): _(待填写)_

## 1) Scope

- [x] P0
- [x] P1
- [ ] P2

## 2) Summary

- Total: 29 _(P0: 17, P1: 12)_
- Passed: —
- Failed: —
- Blocked: —
- Pass rate: —

## 3) Failed / Blocked Cases

| Case ID | Severity | Symptom | Repro Steps | Evidence Path | Issue Link |
|---|---|---|---|---|---|
| _(执行后如有失败/阻塞项，在此记录)_ | | | | | |

## 4) Detailed Results

### P0：核心交易链路

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P0-TXN-001 | | | | | 单包安装（user）|
| P0-TXN-002 | | | | | 依赖安装 |
| P0-TXN-003 | | | | | 全局安装权限校验 |
| P0-TXN-004 | | | | | 仅 bucket 更新 |
| P0-TXN-005 | | | | | 多包升级 |
| P0-TXN-006 | | | | | 同版本重装 |
| P0-TXN-007 | | | | | 重新应用 shim/shortcut/post_install |
| P0-TXN-008 | | | | | 卸载单包 |
| P0-TXN-009 | | | | | 清理旧版本 |
| P0-TXN-010 | | | | | `ignore_failures` 连续执行 |
| P0-TXN-011 | | | | | `--offline` 离线安装 |
| P0-TXN-012 | | | | | `--ignore-cache` 强制下载 |

### P0：仓库与索引

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P0-IDX-001 | | | | | bucket 生命周期 |
| P0-IDX-002 | | | | | 冷启动首次更新 |
| P0-IDX-003 | | | | | 短周期重复更新 |
| P0-IDX-004 | | | | | SQLite cache 刷新与重建 |
| P0-IDX-005 | | | | | 多 bucket 同名包候选处理 |

### P1：查询与可观测

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P1-OBS-001 | | | | | 显式/模糊查询 |
| P1-OBS-002 | | | | | 已安装状态观测 |
| P1-OBS-003 | | | | | 元数据与依赖展示 |
| P1-OBS-004 | | | | | 执行路径与 shim 观测 |

### P1：清单质量工具

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P1-MAN-001 | | | | | checkver 单包检查 |
| P1-MAN-002 | | | | | checkurls 递归扫描 |
| P1-MAN-003 | | | | | checkhashes 校验与回写 |
| P1-MAN-004 | | | | | formatjson 幂等性 |
| P1-MAN-005 | | | | | cat/create 查看与生成 |

### P1：配置与可移植性

| Case ID | Result (Pass/Fail/Blocked) | Exit Code | Key Output | State Verification | Notes |
|---|---|---|---|---|---|
| P1-CFG-001 | | | | | config 读写与持久化 |
| P1-CFG-002 | | | | | alias 增删改查 |
| P1-CFG-003 | | | | | hold/unhold 锁定与解锁 |
| P1-CFG-004 | | | | | export/import 可逆迁移 |
| P1-CFG-005 | | | | | completions 补全脚本生成 |

## 5) Exit Criteria Check

- [ ] P0 all passed
- [ ] P1 pass rate >= 95%
- [ ] No data-damage issues
- [ ] No blocking failures on x64/x86/ARM64

## 6) Conclusion

- Release recommendation: _(待填写)_
- Risk notes: _(待填写)_

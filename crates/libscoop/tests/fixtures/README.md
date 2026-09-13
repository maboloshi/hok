# 测试 Fixture 来源说明

本目录中的 `scoop-original/` 子目录来自原版 Scoop 仓库（见其自带 README）。
其余 `*.json` 为手写的合成 fixture，仅 `0x7eTeamTools.json` 例外：

## 0x7eTeamTools.json

- 来源: `scoop-app` bucket（https://github.com/kkzzhizhou/scoop-apps），
  路径 `bucket/0x7eTeamTools.json`
- 上游包主页: https://github.com/0x7eTeam/0x7eTeamTools
- 许可证: Apache-2.0（清单自身 `license` 字段声明）
- 复制日期: 2026-09-13
- 改动: 仅将行尾由 CRLF 归一为 LF，并补一个文末换行；其余逐字节一致

保留它的原因: 该清单是 `autoupdate.hash` 使用 `"mode": "github"` 的最小真实样例
（真实 bucket 中共 220 个此类清单曾因 hok 的严格枚举而整条解析失败）。

---
name: hok-manifest-parse-audit
title: Manifest 解析：单一入口 + 差距状态
description: Manifest 解析：单一入口 parse_manifest_spec（唯一旁路 formatjson）+ 差距状态（P0-1/2、P1 已修；P0-3 待核对）
metadata:
  type: project
---

Manifest 解析：官方 schema 差距审计 + 单一入口架构（2026-08-10，报告 hok/manifest-parse-audit.md）。**所有解析收敛到单一入口链**：`Manifest::parse/from_json → parse_manifest_spec`（serde_json 严格 → fallback json5 宽容，BOM/注释/尾逗号同走 ManifestSpec）；唯一旁路 formatjson（Value 层，不经 ManifestSpec，只格式化不解析语义，与官方一致）。改 ManifestSpec 层 = 全局生效（checkver/checkhashes/checkurls/install/update/uninstall/query/list/depends 等全部继承）；某命令仍异常先确认是否绕过 parse_manifest_spec（目前只有 formatjson）。

**差距与状态：**
- **P0-1 hash 前缀×长度强绑定**（REGEX_HASH md5=32/sha1=40/sha256=64/sha512=128，官方自由组合）→ **已宽容修复**（非标准长度不再拒收）
- **P0-2 checkver.sourceforge 对象强制 path 必填**（官方 {project,path} 均可选）→ **已修**（path 可选）
- **P0-3 env_set/cookie 只接受 string value**（官方 type:object 不限）→ 未提及修复，如未修保留（待核对）
- **P1 autoupdate 缺字段**：官方 13 字段 vs hok 5（architecture/extract_dir/hash/notes/url），autoupdateArch 8 vs 3 → **已补**（见 Autoupdate 补字段）
- P2 无影响不动（hashExtraction.type deprecated、installer._comment、shortcuts 长度、version pattern）
- 已验证兼容：license/psmodule/checkver 全键/mode 8 枚举/installer/uninstaller/architecture/autoupdate.hash 三形式/bin 对象/hook script（hok 宽容）

**Why:** 避免靠用户跑命令触发 parse bug（tim/license/bin 已踩 3 次）；主动静态穷尽。
**How to apply:** 修 manifest 解析 bug 改 ManifestSpec 层（单入口自动覆盖）；验证一个场景即可。相关 [[scoop-gap-audit]]。

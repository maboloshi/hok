---
name: hok-jsonpath-hyphen-compat
title: jsonpath 特殊字符字段兼容（normalize_jsonpath 修复）
description: jsonpath 带连字符字段名兼容：normalize_jsonpath 修复（extract_version + hash 提取两处）。
metadata:
  type: project
---

jsonpath 提取的兼容性坑与修复（2026-08-09）：

**现象**：官方 Scoop checkver 能解析 `"jsonpath": "$.dist-tags.latest"`，hok 报"无法提取版本号"。

**根因**：jsonpath-rust 1.0.4 用 pest 解析，未加引号的字段名只允许 `[A-Za-z0-9_]`，`-` 等特殊字符直接 `PestError`。且 RFC 9535 标准本身也不允许未加引号的 `-`（`member-name-shorthand` 只含 name-first/ALPHA/_/DIGIT），标准写法是 `$['dist-tags'].latest`。官方 Scoop（PowerShell）JSONPath 语义宽松，接受 `$.dist-tags.latest`，所以 hok 要对齐 Scoop 必须做兼容。

**修复**：`crates/libscoop/src/internal/string.rs` 的 `normalize_jsonpath`——只重写简单点路径 `$.a-b.c` → `$['a-b'].c`；含 `[`/`*`/`?`/`..`/`'`/`\` 或空段的表达式原样返回（保守）。**应用两处用户可写 jsonpath 查询**：`package/checkver.rs` extract_version + `package/checkver_hash.rs` extract_hash_from_page（用户最终决策：两处都加）。GitHub digest 处（`$..assets[?...].digest`）是程序自构造，无需转换。实测 reasonix-hermes 1.12.0→1.12.1 通过；libscoop checkver 测试 22 个全过。

**库选型结论**（2026-08 research）：serde_json_path 0.7.2（严格 RFC 9535、CTS、活跃、下载量大）同样拒绝 `$.dist-tags.latest`——换库不解决 `-` 问题，仍要 normalize；spath 0.3.1（serde_json_path fork、宽松）太年轻（57 stars/总下载 3.6k）不建议。jsonpath-rust 生态最大（14M/90天）、仍活跃（1.0.8 @2026-08）。当前方案 = jsonpath-rust + normalize_jsonpath，零新依赖。

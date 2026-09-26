---
name: hok-jsonpath-hyphen-compat
title: jsonpath 特殊字符字段兼容（normalize_jsonpath 修复）
description: 改 checkver/hash 的 jsonpath 提取，或遇到 $.dist-tags.latest 这类带连字符字段名解析失败时读：normalize_jsonpath 的保守重写规则、两处应用点与库选型结论。
metadata:
  type: project
---

jsonpath 提取的兼容性坑与修复：

**现象**：官方 Scoop checkver 能解析 `"jsonpath": "$.dist-tags.latest"`，hok 报"无法提取版本号"。

**根因**：jsonpath-rust（Cargo.lock 当前 1.0.8）用 pest 解析，未加引号的字段名只允许 `[A-Za-z0-9_]`，`-` 等特殊字符直接 `PestError`。且 RFC 9535 标准本身也不允许未加引号的 `-`（`member-name-shorthand` 只含 name-first/ALPHA/_/DIGIT），标准写法是 `$['dist-tags'].latest`。官方 Scoop（PowerShell）JSONPath 语义宽松，接受 `$.dist-tags.latest`，所以 hok 要对齐 Scoop 必须做兼容。

**修复**：`crates/libscoop/src/internal/string.rs` 的 `normalize_jsonpath`——只重写简单点路径 `$.a-b.c` → `$['a-b'].c`；含 `[`/`*`/`?`/`..`/`'`/`\` 或空段的表达式原样返回（保守）。**应用两处用户可写 jsonpath 查询**：`package/checkver.rs:779`（`json_path`，由 `extract_version` 调用）与 `package/checkver_hash.rs:333`（`json_path_first`）。GitHub digest 处（`$..assets[?...].digest`）是程序自构造，虽然同样经过 normalize，但含 `[` 会原样返回，等于 no-op。

**库选型结论**（一次性调研结论，下载量/star 之类数据会漂，需要时另查）：serde_json_path（严格 RFC 9535）同样拒绝 `$.dist-tags.latest`——换库不解决 `-` 问题，仍要 normalize；spath（serde_json_path 的宽松 fork）生态太年轻，不建议。jsonpath-rust 生态最大且仍在维护。当前方案 = jsonpath-rust + normalize_jsonpath，零新依赖。

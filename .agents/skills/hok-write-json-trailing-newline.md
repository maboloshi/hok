---
name: hok-write-json-trailing-newline
title: write_json 输出规范（CRLF+末尾换行）
description: 改 write_json 或新增 JSON 写回路径（checkver/checkhashes/formatjson/install.json/config.json）时读：4 空格缩进 + 全文 CRLF + 末尾换行的对齐理由与两条回归测试。
metadata:
  type: project
---

`libscoop::internal::fs::write_json` 的输出规范：**4 空格缩进 + 全文 CRLF + 末尾必带 CRLF**（`crates/libscoop/src/internal/fs.rs::write_json`）。

**Why:** 此前 write_json 直接写 serde_json pretty 输出（LF、末尾无换行），checkver/checkhashes 写回 manifest 时会抹掉文末换行，git diff 出现 "No newline at end of file"。官方 Scoop 用 `[IO.File]::WriteAllLines`（`Scoop/lib/autoupdate.ps1`）写回，每行含末行都追加换行 → 以 CRLF 结尾；hok 的 `package::formatjson::to_scoop_json` 也显式保证 CRLF 结尾。write_json 已对齐两者。

**How to apply:** 改 write_json 或新增 JSON 写回路径时保持 CRLF + 末尾换行；write_json 也被 hold/sync_install/config 写 install.json/config.json 使用（同样受益于 CRLF 风格）。相关测试在 fs.rs 的 `write_json_output_ends_with_crlf` / `write_json_preserves_trailing_newline_when_rewriting`。参照 [[local-scoop-source-path]]。

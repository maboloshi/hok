# checkver 取哈希链路复核

> 2026-09-12 ｜ base64 单实现 + Digest/hash 文件对照，已提交

- **base64 单实现**：新增 `internal/base64.rs`（`encode`/`decode`/`to_lower_hex`），删除 `checkver_hash.rs` 手写 `simple_base64_decode`（**不合规**：`filter_map` 静默丢弃非法字符、padding 用 `take(2)` 截断、尾块剩余字符直接跳过 ⇒ `YQ$===`/`YQ==extra` 静默解出垃圾而非报错）；`internal/github.rs` 编码同源。`decode` 契约 = **实测** .NET `Convert.FromBase64String`（PS 5.1 与 pwsh 7.6.5 结果一致）：忽略任意位置 ASCII 空白、要求规范 padding、**容忍非零 tail bits**（`YR==`→`61`）、空串→空。故用 `GeneralPurposeConfig::with_decode_allow_trailing_bits(true)` + `RequireCanonical` + 自行剥空白。
- **Digest 头**（官方 `lib/autoupdate.ps1:181-206 find_hash_in_headers`）逐项判定：
  - a 解码失败语义：**有意偏离**——hok 记 Err 回落 `.meta4`；官方 `FormatException` 逸出（`catch [System.Net.WebException]` 不覆盖）会中断整个 checkver。已在代码注释写明。
  - b `SHA=`/`MD5=` 分支：原只匹配 `SHA-256=`，✅ 已补（官方顺序，先匹配者胜，`([^,]+)` 不跨逗号）。
  - c 请求形状：原来 follow redirect ⇒ 读的是最终 200（通常无 Digest），该路径**实际失效**；✅ 已补 `AllowAutoRedirect=false`（`internal::network::head_no_redirect`）+ 仅 3xx 读 Digest + 2s + `Referer=strip_filename` + UA。回环测试锁定 302+Digest/200+Digest/302 无 Digest。
  - d hash 文件 base64→hex（官方 `:82-92`）：✅ 已补，仅落在 `extract` 模式 arm（对应 `find_hash_in_textfile`），不污染共用 json/xpath 抽取。
  - e `Regex::new` 每调用重建：✅ 改模块级 `LazyLock`。
- commits：`cc21264`（base64 单实现）/ `73431b3`（Digest 对齐）/ `27669df`（hash 文件 base64）。

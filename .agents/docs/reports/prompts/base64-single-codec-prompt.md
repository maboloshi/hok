# 提示词：base64 单实现（依赖合并 A3）+ 官方 Scoop 对照

> **用法**：在新会话里把本文件作为首要输入（`@.agents/docs/reports/prompts/base64-single-codec-prompt.md`）。
> 这是一份**自包含**交接：新会话没有本议题的历史上下文，全部依据写在这里。
> **前置加载的 skill**：`hok-code-organization`、`no-patchwork-fixes`、`local-scoop-source-path`、`scoop-gap-audit`、`hok-rust-gate-practice`、`git-commit-practice`、`hok-proactive-review`、`token-efficient-processing`。

---

## 0. 一句话目标

仓库里**同时存在两个 base64 实现**（一个来自 `base64` crate、一个是手写的 `simple_base64_decode`）。合并成**唯一实现**，同时把该处的 Digest 头解析**逐条对齐官方 Scoop 的 `find_hash_in_headers`** —— 顺带判定并登记 3 个已发现的偏离（源/非源、算法分支、状态门）。

来源：依赖归并分析 A3 项（`.agents/docs/active/audit/optimization-audit-2026-09.md` 之后的 A1/A2/A3 议题）。

## 1. 现状：两个实现（已实测，勿重新论证）

| 位置 | 实现 | 用途 |
|---|---|---|
| `crates/libscoop/src/internal/github.rs:25`、`:242` | `base64::engine::general_purpose::STANDARD.encode(...)` | GitHub API 提交文件正文编码 |
| `crates/libscoop/src/package/checkver_hash.rs:371-409` | **手写** `simple_base64_decode`（注释自称 "Minimal standard base64 decoder (RFC 4648) without external dependencies"，含自行实现的位置表与 padding 校验） | 解析 HEAD 响应的 `Digest: SHA-256=<base64>` 头 |

依赖声明：`crates/libscoop/Cargo.toml:43` `base64 = "0.22"`（crate 已在图里，手写版并无"省依赖"收益）。

调用链：`checkver_hash.rs:290` → `head_digest_sha256(url, proxy, 30)`（`:330-368`）→ `:353` 每次调用 `Regex::new(r"SHA-256=([^,]+)")` → `:359` `simple_base64_decode` → 手工转小写 hex（`:360`）。

## 2. 官方对照（本地副本：`D:\App\github项目\Scoop\Scoop`）

### 2.1 `lib/autoupdate.ps1:181-200` `find_hash_in_headers`（本题主对照）

```
181: function find_hash_in_headers([String] $url) {
185:     $req = [System.Net.WebRequest]::Create($url)
186:     $req.Referer = (strip_filename $url)
187:     $req.AllowAutoRedirect = $false          # ← 不跟随重定向
188:     $req.UserAgent = (Get-UserAgent)
189:     $req.Timeout = 2000                      # ← 2 秒
190:     $req.Method  = 'HEAD'
192:     if (([int]$res.StatusCode -ge 300) -and ([int]$res.StatusCode -lt 400)) {   # ← 只在 3xx 上看 Digest
193:         if ($res.Headers['Digest'] -match 'SHA-256=([^,]+)' -or $res.Headers['Digest'] -match 'SHA=([^,]+)' -or $res.Headers['Digest'] -match 'MD5=([^,]+)') {
194:             $hash = ([System.Convert]::FromBase64String($matches[1]) | ForEach-Object { $_.ToString('x2') }) -join ''
```

要点：**三种算法分支**（先 `SHA-256=`，再 `SHA=`，再 `MD5=`，`-or` 短路 → `$matches[1]` 取自成功的那次 `-match`）；解码用 `[System.Convert]::FromBase64String`，再逐字节 `ToString('x2')`（**小写** hex）；外层只 `catch [System.Net.WebException]` → **非法 base64 触发的 `FormatException` 会逸出该函数**（官方怪癖，不吞）。

### 2.2 `lib/autoupdate.ps1:82-92`（**hash 文件**路径的 base64 分支，第二处解码点）

```
83:  if ($hash -match '^(?:[A-Za-z0-9+\/]{4})*(?:[A-Za-z0-9+\/]{2}==|[A-Za-z0-9+\/]{3}=|[A-Za-z0-9+\/]{4})$') {
84:      $base64 = $matches[0]
85:      if (!($hash -match '^[a-fA-F0-9]+$') -and $hash.Length -notin @(32, 40, 64, 128)) {
86:          try   { $hash = ([System.Convert]::FromBase64String($base64) | … 'x2') -join '' }
88:          catch { $hash = $hash }        # ← 这里**吞错并保留原值**
```

同一个官方文件里，两处 base64 解码的**失败语义不同**（2.1 抛、2.2 吞）。这必须成为合并时的显式决策，而不是"统一成一种"。

### 2.3 已对齐、不要动的部分

`lib/autoupdate.ps1:3-18` `format_hash`（`toLower` → 剥 `sha256:` 前缀 → 按长度 32/40/64/128 加前缀 → 其他 `$null`）已被 `checkver_hash.rs:435-450 format_hash` 复刻（含 `to_lowercase`、`strip_prefix("sha256:")`）。本题**不要**顺手改它。

### 2.4 必须在动手前逐条判定的偏离

| # | hok 现状 | 官方 | 判定要求 |
|---|---|---|---|
| a | 手写解码器非法输入 → `Err`，调用点 `if let Ok(bytes)` 当作"没有 hash"继续走其它来源 | 2.1 会抛 `FormatException` 逸出；2.2 保留原值 | 二选一并写明理由；若要复刻"抛"，需改调用点而非只换解码器 |
| b | `:353` **只匹配 `SHA-256=`** | 还有 `SHA=` / `MD5=` | 判定是缺口还是有意收窄（并登记 `scoop-gap-audit` 记忆） |
| c | `:290` 传 `timeout_secs = 30`；未复刻 `AllowAutoRedirect=false` 与 **300-399 状态门** | `Timeout = 2000`、不跟随重定向、只在 3xx 看 Digest | 逐项实验（对同一 URL：直连 200 / 301 到 CDN），结论写进提交说明 |
| d | 未发现 hash 文件路径的 base64 分支（`grep -n 'FromBase64\|base64' crates/libscoop/src/package/checkver_hash.rs` 只有 2.1 一处） | 2.2 存在 | 判定是缺口还是新版 Scoop 已弃用；不要凭印象 |
| e | `Regex::new` 在 `:353` **每次调用重建** | 同一函数内 `-match` 无此成本 | 顺手修（提到模块级 `OnceLock`/`LazyLock`），保持本提交聚焦 |

### 2.5 语义必须实验确认（.NET vs `base64` crate）

`Convert.FromBase64String` 与 `base64` crate 的 `STANDARD` 引擎**并不等价**，至少三点要用实验固定：

1. **空白字符**：.NET 忽略 base64 串内的空白（含换行）；`STANDARD.decode` 默认把空白视为非法 → 需要显式 trim 或换用允许空白的配置。
2. **非规范 tail bits**：.NET 历史上不校验末组剩余位；`base64` crate 默认拒绝非零 tail bits（`with_decode_allow_trailing_bits`）。
3. **padding**：.NET 要求总长度（忽略空白后）为 4 的倍数，缺 padding 会抛；`STANDARD` 同样要求 padded（另有 `STANDARD_NO_PAD`）。

实验方法（两个宿主都跑，脚本用 base64 传参避免引号问题）：

```powershell
$cases = @('YQ==', 'YQ', 'Y Q = =', 'YQ==', 'YQ=8', 'YQ$=', 'YQ==extra')
foreach ($c in $cases) {
  $b = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($c))
  powershell.exe -NoProfile -Command "`$s=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$b')); try { ([Convert]::FromBase64String(`$s) | %{ '{0:x2}' -f $_ }) -join '' } catch { 'FAIL: ' + `$_.Exception.GetType().Name }"
}
```

（把 `.NET` 的输出表与 `base64` crate 的候选配置逐例对照，选定配置并写成单测。）

## 3. 合并方案（要求）

1. 新增内部薄封装（位置随 `internal/` 既有约定），例如：
   ```rust
   // crates/libscoop/src/internal/base64.rs（命名/位置可调）
   pub fn decode_standard(input: &str) -> Fallible<Vec<u8>>;   // 语义 = 经 §2.5 实验确定的契约
   pub fn encode_standard(bytes: &[u8]) -> String;
   pub fn to_lower_hex(bytes: &[u8]) -> String;
   ```
2. 删除 `checkver_hash.rs:371-409 simple_base64_decode`，改调封装；`internal/github.rs:242` 的 `.encode` 也走同一封装（保证 encode/decode 语义同源）。
3. **禁止**（`no-patchwork-fixes`）：因为官方两处失败语义不同，就在封装里加 `bool strict` 让调用点传 `true/false` 而不说明；应把"失败后怎么办"留在**调用点**，封装只负责"按契约解码"。
4. 偏离项 b/c/d 的处理**各自独立成提交或在提交说明单独列条**，不要混进"换实现"这一步。

## 4. 验收条件

1. `grep -rn "fn simple_base64_decode"` 无结果；`checkver_hash.rs` 不再出现手写位置表。
2. `to_lower_hex` 的输出与官方 `ForEach-Object { $_.ToString('x2') }` 逐例一致（含前导零，如 `0x0a` → `0a`）。
3. 新增单测覆盖 §2.5 的 7 类输入，期望值取自**实测的 .NET 行为表**（不是"我觉得应该"）。
4. Digest 头解析有**纯函数级**单测（把 header 值 → hash 的转换抽出来，不依赖网络）：覆盖 `SHA-256=` 正常、含空白、含逗号后续字段（`([^,]+)` 贪婪边界）、非法 base64。
5. 门禁：`cargo check --workspace --all-targets` + `cargo clippy --workspace --all-targets`（仅允许 vendored `innospect` 既有 4 条）+ `cargo test --workspace`（基线 515 passed）。
6. 提交说明含"官方 2.1 / 2.2 对照 + a–e 逐项判定"。

## 5. 陷阱

- `SHA-256=([^,]+)` 的 `[^,]+` 遇 `Digest: SHA-256=xxx, SHA=yyy` 时会在逗号处停止 —— 换实现时不要"顺手改成正则贪婪"。
- 官方 2.1 只在 **3xx** 响应上看 Digest；改成跟随重定向后，200 响应也看 → **行为放宽**，这属于功能变更，必须单独标明。
- `.NET` 的空白容忍若被 `base64` crate 拒绝，**不要**只在调用点 `trim()` 了事：换行嵌在中间仍会失败，需按 §2.5 选定配置。
- 相邻但**不属本题**的议题：`checkver_hash.rs:316-327` 用正则解析 RDF XML，而官方用 `$xml.SelectSingleNode`（`lib/autoupdate.ps1:176-179`）—— 属 A4（XML 抽取）议题，别在这里一并动。
- **不要整包 `cargo fmt`**：只 fmt 本次文件（`rustfmt --edition 2024 <files>`）。
- git 一律 `-c core.autocrlf=false`；`commit.gpgsign=true`。
- 不要动 vendored `crates/innospect`（它自己有一套 crypto/base64 相关实现，与本题无关）。

## 6. 建议提交拆分

1. `refactor(libscoop): decode base64 through one implementation`（删手写解码器 + 封装 + 单测）
2. `fix(checkver): align the Digest header path with Scoop`（偏离 b/c/e 中判定为"要对齐"的项；若判定为"有意偏离"，则只登记不改码）

## 7. 如果判定不做

把 a–e 的对照结论与不做理由回写 `.agents/docs/active/audit/optimization-audit-2026-09.md` 追加节，并 `hindsight_ingest_document` 一条更正（含"两个 base64 实现已确认、决定保留"及其理由）。

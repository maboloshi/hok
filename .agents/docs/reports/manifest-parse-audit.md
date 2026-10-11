# Manifest 解析对比审计：官方 schema.json vs hok ManifestSpec

日期：2026-08-10。方法：逐字段对比官方 `D:\App\github项目\Scoop\Scoop\schema.json`（JSON Schema 权威定义，684 行）
与 hok `crates/libscoop/src/package/manifest.rs` + `manifest_parse.rs` 的 Rust 反序列化实现。
辅助：官方运行时不校验 schema（`ConvertFrom-Json` 宽松解析），故"官方实际接受"比 schema 更宽；
本审计以 schema 为基准，hok 若比 schema 更宽视为可接受的宽容（标注）。

## 总体结论

- hok `ManifestSpec` 28 个顶层字段 vs 官方 28 个：**覆盖 100% 对齐**，无缺失字段。
- 所有 struct 均**无 `deny_unknown_fields`**：官方有而 hok 没有的字段被**静默忽略**（不拒收，但功能缺失）。
- 真拒收（parse 失败/静默跳过）风险**共 4 处**（P0）：P0-1～P0-3 为 2026-08-10 本审计所得；**P0-4 为 2026-09-13 新增**（`autoupdate.hash.mode: github`，见下表）。

## 差距清单

### P0（官方接受、hok 拒收 → parse 失败/静默缺失）

| # | 位置 | 官方 schema | hok 现状 | 影响 |
|---|---|---|---|---|
| P0-1 | 顶层 `hash` / `architecture.hash` | `hashPattern`: 前缀与长度**自由组合**（`md5:<32或40或64或128hex>`、`sha256:<32hex>` 等均可）| `REGEX_HASH`（constant.rs:67-70）**强绑定**：md5=32、sha1=40、sha256=64（或裸64）、sha512=128 | 非标准长度搭配的 hash 被拒收 → 整清单 parse 失败 |
| P0-2 | `checkver.sourceforge`（对象形式）| `{project, path}` 均非必填（schema.json:338-349）| `SourceforgeVisitor`（manifest_parse.rs:243）`path` 用 `missing_field` **强制必填** | `{"project": "x"}` 官方合法、hok 拒收 |
| P0-3 | `env_set` / `cookie` value | `type: object`，value 类型不限（schema.json:577/565）| `HashMap<String, String>` 只接受 string value | 数字/布尔 value 官方接受、hok 拒收（实际清单几乎全为 string，影响小） |
| **P0-4** | `autoupdate.hash.mode` | schema.json:51-62 只列 8 个枚举值，但**官方运行时不校验 schema**，`lib/autoupdate.ps1:271-302` 的 `switch` 还接受 `github`（`:298-301` 取 GitHub Releases API 的 asset `digest`，匿名、无 token）| `HashExtractionMode`（manifest.rs:536）是**严格 serde 枚举**、无 `github` ⇒ `unknown variant` ⇒ 整清单 parse 失败 | **实测本机 4 bucket 21685 个 manifest 中 221 文件/223 处命中，220 个不可解析**（scoop-app）；又因 `package/query.rs:120,285` 的 `filter_map` **静默丢弃**失败项，`install/search/info` 表现为「**找不到软件包**」而非报错。**已修**：提交 `eb67941`（加 `#[serde(rename = "github")] Github` + 注释 + 离线夹具；提取实现早已在 `checkver_hash.rs:166-198` 在位）|

### P1（官方支持、hok 静默丢弃字段 → 功能缺失，不 parse 失败）

| # | 位置 | 官方 schema | hok 现状 | 影响 |
|---|---|---|---|---|
| P1-1 | `autoupdate` | 13 字段：architecture/bin/env_add_path/env_set/extract_dir/hash/installer/license/notes/persist/psmodule/shortcuts/url（schema.json:213-281）| `Autoupdate`（manifest.rs:298-314）仅 5 个：architecture/extract_dir/hash/notes/url | autoupdate 更新时 **bin/env_add_path/env_set/installer/license/persist/psmodule/shortcuts 不会随新版本替换** |
| P1-2 | `autoupdate.architecture.*`（autoupdateArch）| 8 字段：bin/env_add_path/env_set/extract_dir/hash/installer{file}/shortcuts/url（schema.json:177-212）| `AutoupdateArchSpec`（manifest.rs:436-445）仅 3 个：extract_dir/hash/url | 架构层 autoupdate 缺 bin/env_add_path/env_set/installer/shortcuts 更新 |

### P2（边缘/风格/官方 deprecated）

| # | 位置 | 说明 |
|---|---|---|
| P2-1 | `hashExtraction.type` | 官方字段但标注 "Deprecated, hash type is determined automatically"（schema.json:63-71）；hok `HashExtraction` 无此字段 → 静默忽略，无碍 |
| P2-2 | `installer._comment` | 官方 undocumented 注释字段；hok `Installer` 无 → 静默忽略，无碍 |
| P2-3 | `shortcuts` 长度约束 | 官方每项 minItems 2 / maxItems 4（schema.json:165-176）；hok `Vec<Vec<String>>` 无约束（更宽，可接受） |
| P2-4 | `version` pattern | 官方 `^[\w\.\-+_]+$`；hok 无校验（更宽，可接受） |

## 已验证兼容（无差异）

- `license`：string（不校验 SPDX，hok 有意）+ `{identifier(必填), url}` ✓（manifest_parse.rs:155-207）
- `psmodule`：`{name}` ✓
- `checkver`：全部键覆盖——`github`（string "github" / map 形式 → url+默认 regex）、`re`/`regex`、`url`、`jp`/`jsonpath`、`xpath`、`reverse`、`replace`、`useragent`、`script`（string|array）、`sourceforge`（字符串 "project/path" 形式 ✓，对象形式见 P0-2）
- ~~`hashExtraction.mode`：8 个枚举值（download/extract/json/xpath/rdf/metalink/fosshub/sourceforge）与官方 enum 完全一致 ✓~~ **❌ 更正（2026-09-13）**：官方 `schema.json:51-62` 的 `mode` enum **不含 `github`**，但官方**运行时不校验 schema**、由 `lib/autoupdate.ps1` 的 `switch` 兜住 ⇒ 官方**运行时接受** `mode: github`（取 Releases API asset 的 `digest`，`autoupdate.ps1:298-301`）。hok 用严格 serde 枚举（`manifest.rs:536`）⇒ `unknown variant 'github'` ⇒ **`parse_file` 整条 Err**，因解析是单入口而 `package/query.rs:285`（`:120` 同型）的 `filter_map` **静默丢弃**失败项，`hok install/search/info` 表现为「**找不到软件包**」。实测本机 4 bucket 21685 个 manifest 中 `scoop-app` 有 221 文件 / 223 处，逐个 `hok info <path>` **220 失败 / 1 通过**（P0，另开会话修）。
- `installer`：`{args, file, script, keep}` 全有（keep 解析存在；install 流程是否使用是另一层问题）✓
- `uninstaller`：`{args, file, script}` ✓（hok 额外有 keep，更宽）
- `architecture`：`{32bit, 64bit, arm64}`，双方一致（官方无 `noarch`）✓
- `hashExtraction` 字符串形式（`"mode:json"` 等）：hok visit_str 接受 ✓（P0-5 已修）
- `autoupdate.hash`：对象 | 对象数组 | 字符串三种形式 ✓
- `bin` / `persist`：string | array | array-of-array ✓（deserialize_bin 宽容，P0-6 已修）
- hook script（pre/post_install、pre/post_uninstall）：string | array ✓ + hok 额外接受 `{script}` 对象形式（deserialize_hook_script，官方 schema 无——hok 宽容，官方 checkver 宽松解析亦接受）
- `suggest`、`depends`、`notes`、`extract_dir`、`extract_to`、`innosetup`、`homepage`、`description` ✓
- 顶层额外键 `$schema`/`_comment`/`##`：hok 静默忽略 ✓
- 顶层 `required [version, homepage, license]`：hok 全 Option（P0-2 已修，更宽容）✓
- `uriOrArrayOfUris` 的 `$` 排除约束、`jsonPathPattern` 等 format 约束：hok 不校验 format（更宽，可接受）

## 建议修复顺序

1. **P0-1** hash 长度宽松（对齐官方组合自由度）——改 `REGEX_HASH` + `HashString::new` 解析
2. **P0-2** sourceforge 对象 path 改为可选（官方语义）——改 `SourceforgeVisitor`
3. **P0-3** env_set/cookie value 宽容（可延后，实际影响极小）
4. **P1-1/P1-2** autoupdate 补齐缺失字段（功能补全，改动较大，可单独立项）
5. P2 项默认不动（无实际影响）

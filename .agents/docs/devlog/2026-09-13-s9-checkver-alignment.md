# S9 `checkver` 深度对齐

> 2026-09-13 ｜ 已闭环，4 提交，分支 `fix/s9-checkver` → 已线性 ff 进 `refactor/edition-2024`，tip `8ad7d080`

**决策（用户拍板）**：按 **S9-2 → S9-3 → S9-1** 逐块做；与已落地的 D4 **去重**（请求头一律走共享入口）；两条有意偏离见下。
**落地**：
- `checkver.rs`：`Substitution`（**最长键优先** + `$match<Name>` 大小写不敏感）；`$url/$baseurl/$basename(=UrlDecode(url_remote_filename))/$urlNoExt/$basenameNoExt` 按官方**逐 URL 分层**应用到 hash 配置；属性替换**驱动自 raw JSON 的 autoupdate 键**（覆盖 changelog/extract_to 等）；hash 只在 manifest 已有 hash 时写入；hash 回退下载落 Scoop 缓存；删除恒等 stub（`checkver_url.rs` 整个移除）。
- `checkver_hash.rs`：默认兜底正则 + 文件名旁 + metalink、`$sha256/$basename/$base64` 模板、**`find` 是正则取第 1 组**（不是字面子串）、"找不到"**回落下载 + 本地 SHA-256** 而不是整条失败。
- S9-1：github/sourceforge 快捷方式**显式建模**（`Checkver` 增 4 个 `#[serde(skip)]` 标记）；API 改写**仅在配置了 token 时**；`checkver.script` 复用既有子进程入口 + preamble；`--throw`（带 `--throw` 坏 URL → exit 1；不带则打印 ERROR 且 exit 0）。
- **与 D4 去重（勿回退）**：`ScoopRequest`/`HeaderProfile` 是唯一请求头入口；`ResolvedSource` 无 `user_agent`/`token` 字段；`download_bytes` 走 `HeaderProfile::Autoupdate`；**别再加内联 `RequestOptions`**。token 只认 `network::github_token`（`SCOOP_GH_TOKEN` 优先于 `gh_token`，空值视为无）。
**实测量化（21797 个真实 manifest / 4 bucket）**：hash 变量从不替换 1160/3849 manifest → 全部可解；hash 链无解 1350/1387 条目（1298/3846 manifest）→ 全部可解；github+自定义 re 被喂 API JSON 122/620 → 恢复；与已安装 Scoop 0.5.3 请求 URL 逐字一致；约 999 个 `/releases` 双写清单恢复可用。
**两条有意偏离（登记）**
1. **`Bearer` vs 官方 `token`**（用户拍板）：GitHub 官方文档标准写法是 `Authorization: Bearer`，官方 `bin/checkver.ps1:223` 用 `token`，两者 GitHub 都接受，hok 取标准拼写。
2. **`checkver.github` 已含 `/releases`（或 `/releases/latest`）时不再追加**：官方无条件追加产出 `.../releases/releases/latest`（实测 404，约 999 个真实清单受影响），hok 做**超集修复**（官方在该形态本就不工作，故不破坏任何"官方能工作"的场景）。
**其他有意偏离/未做**：Digest base64 解码失败回落 `.meta4`（§4 已登记，未碰）；`$urlNoExt`/`$basenameNoExt` 键前缀碰撞（官方实测产出 `...zipNoExt/checksum`，hok 改最长键优先）；script 子进程 vs 官方进程内；脚本失败不中断整轮；非 UTF-8 lossy；**未做** xpath 命名空间、输出文案、`checkver.url` 变量替换。
**⚠️ 过程教训**：本沙箱里提交信息一律 `-F <文件>`（PowerShell 会展开反引号与 `$`，该会话曾损坏 3 条提交信息、靠 `git fsck --unreachable` 找回重建）；**有未提交改动时绝不 `reset --hard`**（该会话曾两度丢失未提交工作）。
**How to apply**：autoupdate 属性替换只走 `Substitution`；hash 提取只改 `find_hash_in_*` 且"找不到"必须回落下载；快捷方式判定只改 `resolve_checkver_source`；请求头一律 `ScoopRequest`；token 只认 `network::github_token`。

---

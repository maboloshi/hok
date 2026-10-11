# D4 下载请求头

> 2026-09-13 ｜ S10 划出，已闭环，2 提交，分支 `fix/d4-download-headers` → 已线性 ff 进 `refactor/edition-2024`，tip `6db6bfbd`

**问题**：hok 曾有 **4～6 套互不一致的 UA 语义**，且 `download_file`（`network.rs:64`）**一个头都不发**（文件头注释却谎称已注入 UA/Referer/PRIVATE_HOSTS）；官方 GET 实测带 `user-agent="Scoop/1.0 (+http://scoop.sh/) PowerShell/7.6 …"` + `referer`。这是"单入口违规"的典型。
**实现**：唯一入口 `libscoop::network::ScoopRequest` + `internal::network::HeaderProfile`（`Download`/`CheckUrls`/`Checkver`/`Autoupdate` 四变体，每个方法注释带官方 file:line）；UA 单点 = `constant::DEFAULT_USER_AGENT` = **`Scoop/1.0 (+http://scoop.sh/) Hok/0.2.0`**（**不伪造 PowerShell 段**——实测真实源只有 GitHub API「必须非空 UA」一条硬约束，①②③ 均满足）；Referer = `strip_filename(url)`、**从不使用 bucket**，下载路径抑制 `sourceforge.net`/`portableapps.com`（照抄官方不对称）；`head_url` 不再硬编码、`github.rs` 不再固定 `hok`、`checkver_hash` 无 fallback 那处一并收敛。
**顺带修的两个真问题**：①checkver 的 **GitHub token 原本会发往任意 URL**（安全项）→ 现只在 `api.github.com`；②`checkver.useragent` 解析了却从不读取 → 现生效。
**更正的假设**：原计划 D4b 的"+cookies"**不成立**——官方 `checkhashes.ps1:122` / `autoupdate.ps1:319` 都传 `$null`，Cookie 是 no-op。（对照：私有 bucket 鉴权链路的 cookie 是另一条线，见 `hok 私有 Scoop bucket 支持` 页面。）
**验证**：本地夹具 `127.0.0.1:18877` 逐字断言 18 passed；命令回归 install/download/reinstall/update/checkhashes/checkurls 全 exit 0。
**⚠️ 环境限制（重要）**：本机**出站 HTTPS 不可用**——直连与代理 `127.0.0.1:7890` 均 TLS 失败，**纯 HTTP 可用**。故一切"真实源"验证在本机不可行，只能靠本地夹具 + 回环测试；写真实源冒烟结论前先确认这一点。
**遗留/有意偏离**：`handle_special_urls` 不做；PRIVATE_HOSTS 全入口注入（未按 ①② 拆开提交）；不伪造 PS 段。
**⚠️ 新发现的两条既有问题（基线二进制同样复现，未修，建议另开会话）**
1. `hok checkver` 在**本环境挂起**（空目录也挂）——疑与子进程/pwsh 引擎或网络等待有关。
2. `USERPROFILE` 未设时 `internal/fs.rs:63` **panic**（应降级或报错，不应 panic）。
**更正（2026-09-13 回归修复，见 devlog [`2026-09-13-download-referer-removal.md`](../devlog/2026-09-13-download-referer-removal.md)）**：D4 给下载路径加的 `Referer = strip_filename(url)` **在真实站点上是错的**——腾讯 `updatecdn.meeting.qq.com` 带它 403、不带 200，`scoop-private/we-meet` 升级因此失败；`HeaderProfile::Download` 现已**恒不发** Referer（`NO_REFERER_HOSTS` 随之下线），探针三 profile 保持原样。D4 的 UA 单入口 / PRIVATE_HOSTS / token 收敛结论仍然有效。

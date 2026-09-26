# 下载默认不发 Referer

> 2026-09-13 ｜ 回归修复）+ 「全在缓存中」误报（2 提交 `cf260d9` `4748ae1`

**背景**：用户 `hok upgrade` 升级 `scoop-private/we-meet-3.46.1.409` 时，先看到「无需下载，均已在缓存中」，紧接着 `ERROR HTTP status client error (403 Forbidden) …updatecdn.meeting.qq.com…`，缓存目录里其实什么都没有。

**坑 1（回归）**：403 的根因是 **Referer**。用 Node（自带 CA，绕过本机 schannel）对该 URL 做矩阵探测，唯一变量就是 Referer：**不带 → 200/206；带 `https://updatecdn.meeting.qq.com/cos/<md5>/` → 403；带 `https://meeting.tencent.com/` → 200**（直连与走用户配置的 `127.0.0.1:7890` 代理结论一致）。而 hok 发的正是中间那个值——D4 会话的 `e326ecf` 把官方 `lib/download.ps1:94-96` 的 `Referer: strip_filename $url` 抄进了包下载路径，该路径此前**从不发 Referer**（提交信息自述），用户 2026-09-03 成功安装旧版本正是那个状态；现在装的二进制是 09-13 21:32 构建的，已含改动。**结论：是"对齐上游"改坏的**。

**坑 2（顺带挖出的展示层缺陷）**：`src/eventloop.rs` 的交易计划块只判断 `download_size.total > 0`，忽略 `DownloadSize::estimated`。HEAD 探测一旦失败（403/404/超时），`calculate_download_size` 会把 `remote_size` 留成 0 并置 `estimated = true`，于是 `total == 0 && estimated` 被渲染成「无需下载，均已在缓存中」——正是用户看到的自相矛盾。正确谓词 `total == 0 && !estimated` 早在 `sync_install.rs:373-377`（离线判定）里写着，本次只是把它搬到展示层；未知大小改打新键 `cmd.download_size_unknown`。注意与 2026-08-28 `67ab5b5`（`0 == 0` 把空缓存判成命中，属判定层）不是同一处。

**上游同款（不必自省）**：官方 Scoop 在这个 URL 上**也会 403**——URL 带 `#/dl.7z` 片段使 `fname` 解析成 `dl.7z`，官方算出的 Referer 是 `…officialwebsite.exe#/`，仍是 `updatecdn` 域。上游相关 issue 至今未解：[#6009 Control over the 'Referer' header](https://github.com/ScoopInstaller/Scoop/issues/6009)（open，blocking 两个 issue）、[#6212 aDrive 403](https://github.com/ScoopInstaller/Scoop/issues/6212)（阿里云盘同款）、[Extras #11587](https://github.com/ScoopInstaller/Extras/issues/11587)（geeks3d **需要** Referer，但要的是页面 `https://geeks3d.com/dlz/`，Scoop 的目录 Referer 也喂不对）。Scoop 里写死的只有反向例外（`sourceforge.net`/`portableapps.com` 不发，见 CHANGELOG `Don't send referer to portableapps.com`），**不存在"需要 Referer"的 host 白名单**；aria2 路径（`lib/download.ps1:407`）甚至只排除了 sourceforge，漏了 portableapps——上游自身不一致。

**决策**：下载路径**默认不发** Referer（`HeaderProfile::Download` 恒 `None`，`NO_REFERER_HOSTS` 下线）；需要 Referer 的站点走 `PRIVATE_HOSTS`（官方 `libexec/scoop-config.ps1:106-109` 文档化的 per-host header 机制）。探针 profile（checkurls/checkver/autoupdate）保持官方语义不动。这是**有意偏离上游**，理由与实测都写进了 `HeaderProfile::referer` 的 doc comment。

**顺带记录的坑**：`libscoop` 的 build 脚本要求 `target/{debug,release}/hok-shim*.exe`，所以**任何** `cargo clippy/test` 之前必须先 `cargo build -p hok-shim`；全量 `cargo clippy --workspace -- -D warnings` 会被 vendored `crates/innospect` 的 3 个既有 `chunks_exact` lint 挡死，按包限定要加 `--no-deps`。本机 schannel 不可用（curl / reqwest-native-tls / .NET 均 TLS 失败），hok 二进制端到端复现不了 → 403 是在 HTTP 层用同一套头复现的。

**验证**：`cargo test --workspace` 741 passed / 2 failed（仅既有沙箱 ACL `error 5`：`permission::tests::grant_users_write_inherit_*`）；限包 clippy（`--no-deps -D warnings`）干净；新增/改写的 4 条测试全绿（下载契约断言无 Referer、PRIVATE_HOSTS 恰好注入一条、Download profile 恒无 Referer、其余 profile 仍带）。

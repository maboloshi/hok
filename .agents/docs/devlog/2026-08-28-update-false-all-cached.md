# update 误判「全部已缓存」导致 verify 文件不存在

> 2026-08-28 ｜ commit 67ab5b5

**用户报告**（另一台设备 hok update 输出）：「正在从缓存加载 microsoftedge-beta#151.0.4129.35#8e391b8.7z...」「正在从缓存加载 manggo#0.8.0#01af0b1.7z...」之后直接 failed to verify 'xxx': 系统找不到指定的文件 (os error 2)，且实际 cache 里根本不存在这两个新版本缓存文件。与 08-21 修过的「upgradable 文件名不一致」症状相似但根因不同。

## 根因（PackageCache::update_valid_state，download.rs）
calculate_download_size 对每个文件发 HEAD 求 remote_size。当 HEAD 失败（URL 返回非 200/服务器不支持 HEAD/设备网络问题）时 remote_size 保持 0，同时本地缓存文件缺失 local_size=0。update_valid_state 用 local_size == remote_size 判有效 → 0 == 0 判真 → 包被标成 CacheMaybeValid::Full → download() 整包跳过（发 PackageCacheHit，即日志的「正在从缓存加载」）→ 没下任何文件 → 后面 hash verify 找缓存文件「系统找不到指定的文件」。且因为 remote_size 全为 0，download_size.total=0 → 还打印「Nothing to download, all cached.」，与用户日志完全吻合。

## 修复
update_valid_state 只在 local_size > 0 && local_size == remote_size 时才计为有效缓存——本地文件真实存在且与远端大小一致才算命中。与 download() 里逐文件的既有守卫（local_size > 0 && == remote_size）语义对齐；CacheMaybeValid 补 Debug 便于断言。新增两个回归测试（缺失文件+未知远端 → 非缓存；存在且大小匹配 → Full / 大小不匹配 → None）。

## 教训
「0 == 0」这类空值相等的假命中在「本地缺失 + 远端未知」双零场景是静默的：不报错、不算下载量、还发缓存命中事件，最后只在 verify 爆一个 os error 2。判「缓存有效」必须先问「本地文件到底在不在」，不能只比大小。

---

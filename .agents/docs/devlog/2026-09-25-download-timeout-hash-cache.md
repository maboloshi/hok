# 下载超时/哈希详情/缓存命中三点修正 + 延迟校验竞态登记

> 2026-09-25 ｜ 3 提交：`4b2004b` `d49d6dd` `cdcfab1`，base `6620d69`

三项都先按 [[investigate-first]] 出报告（现象 + file:line 证据 + 含"不改"的选项 + 推荐 + 影响面），用户逐项拍板后才动手；一个主题一个提交。

## 1. 下载超时看不出"超时"（`4b2004b`）

- **现象**：把超时压到 2s、服务器只接连接不回包 ⇒ 报 `download failed: error sending request for url (http://127.0.0.1:PORT/app.bin)`，**没有任何超时字样**。
- **根因（读 reqwest 源码确认，不是猜）**：`reqwest-0.12.28/src/error.rs:231` 的 `Display` 只写 `"error sending request"` + `for url (...)`；超时原因在 `source()`（`TimedOut` → `"operation timed out"`，:382-390），`is_timeout()`（:116-135）会遍历 source 链。hok 在 `internal/network.rs:539` 只取了顶层 Display，信息就此丢失。
- **修法**：`Network::download_stream` 返回类型化 `DownloadError{Timeout{secs},Other(String)}`（`classify_download_error` 统一分类 send/中途读错误），超时在 `package/download.rs` 映射为新 `Error::DownloadTimedOut{url,timeout_secs}`，`translate_error` 走新 key `error.download_timeout`（`下载 '%{url}' 超时（%{secs} 秒）`）。非超时文案与 `error_for_status` 的原文一字未改。
- **单流不加重试（用户拍板）**：官方 `lib/download.ps1:71-196` `Start-Download`/`Invoke-Download` 单流完全不重试（只有 301/302/303/307 手动跟转），重试只在 aria2 分支（:347 `--retry-wait` + aria2 默认 max-tries）；hok 单流已有 `.download` + Range 续传覆盖中断（`download.rs:503-509`），加重试只会让 120s 变 240s。
- **顺带登记（未改）**：hok 的 120s 是 reqwest **总 deadline**（官方文档 "from when the request starts connecting until the response body has finished"，`async_impl/client.rs` 的 `timeout` 文档），官方 Scoop 走 .NET 默认 `Timeout`=100s（响应头）+ `ReadWriteTimeout`=300s（每次读）——大文件慢链路可能在 120s 被整体掐断，这是语义差异而非 bug，留给后续专题。
- **测试**：`internal/network.rs` 新增"loopback 只 accept 不回包 + `timeout_secs:1`"断言分类为 `Timeout`（放弃连接太早会变成"连接关闭"，测试里用 3s sleep 撑过 deadline）；`src/lib.rs` 断言 `translate_error` 渲染出 url 与 120。

## 2. 哈希不匹配详情硬编码英文（`d49d6dd`）

- **现象**：`package/mod.rs:549-560` 的 `Display` 写死四行英文；`translate_error` 无 `HashMismatch` 分支 ⇒ 中文环境落 `_ => err.to_string()`。
- **比题设更广的一层**：`cmd.failed_verify` 把 `error = e`（`Error` 的 Display）直接嵌进本地化模板，两处——内联校验 `download.rs:306`、尾部批量 `download_verify.rs:157`。只补 `translate_error` 修不到它们。
- **修法**：新增 key `error.hash_mismatch_detail`（en/zh，4 占位含标题行）+ `HashMismatchContext::localized()`；`translate_error` 走分支，两处嵌入改走新 `verify_error_message()`（`download_verify.rs`，`HashMismatch → localized()`，其余原样）。`Display` 保持英文稳定块（库/日志面），用户可见路径全部本地化。
- **测试**：`src/lib.rs` 断言四个字段都进了渲染串（占位名写错会静默渲染空值）；en/zh 键完整性由 `tests/i18n_keys.rs` 兜底。既有 `download_apps` 的 `mismatch_reports`（断言 `"Hash mismatch for package"`）在默认 en 下继续通过，等于顺带钉住了内联/尾部两条嵌入路径。

## 3. 缓存命中提示显示缓存文件名（`cdcfab1`）

- **现象**：`Event::PackageCacheHit(String)` 承的是 `app#1.0#<sha7>.ext`（`cache_file_name`，`mod.rs:602-623`），前端只把它填进 `detail.loading_cache`；且 emit 点是 `c.inner.keys()` **每文件一条**。官方锚点 `lib/download.ps1:60` 印的是 URL 里的远端文件名，同样不印缓存名。
- **修法**：事件改结构体载荷 `{ident,name,version}`，emit 改为**每命中包一次**（多文件包不再重复同一行），`loading_cache` 改 `Loading %{name} %{version} from cache.` / `正在从缓存加载 %{name} %{version}`；`eventloop.rs`（bar 活跃时 `note`）与 `scoop_handler.rs` 两处消费点同步。
- **测试**：`download_apps.rs` 新增双文件包跑两次 ⇒ 第二次恰好一条 `PackageCacheHit`，name/version 来自 manifest。（测试里必须先取走 receiver 再排空第一轮：容量 20 的通道无人消费时会满，`try_send` 静默丢事件。）

## 4. 延迟校验消息可能压在活跃 bar 行上 —— **登记为已知风险，未改码**

- **结构性证据**：`download_verify.rs:211-222` 先 `try_send(PackageDownloadDone)` 走**异步事件总线**，紧接着 `session.output().error(msg)` —— 而 `Output` 是**同步直印**（`output.rs:16-18`，CLI 印 stderr）。两条通道之间没有任何顺序保证：`try_send` 只入队，事件循环何时被调度、何时执行 `dlprogress.finish()`（`cui.rs:334-343`）取决于调度；bar 还有 100ms steady tick 重绘（`cui.rs:293`），压在活跃行上的 stderr 文本会被清/撕裂。
- **同类旧有行为**：`download_one_package` 在 IgnoreFailures 下直印 `cmd.failed_download`（`download.rs:655-659`），同样绕过 bar。
- **能否确定性复现：不能。** 竞态窗口是"入队返回到事件循环被调度"，没有注入点；ConPTY 抓取只放大可见性、改变不了因果顺序，写出来的只能是 flaky 测试。可确定性证明的只有"两条通道 + 语义要求同序"这一结构事实，因此**按用户裁决只登记，不臆造修复**。
- **顺带发现的更硬一洞（同源）**：`EVENT_BUS_CAPACITY = 20`（`constant.rs:21-22`）且控制事件一律 `try_send`——事件循环落后到队列满时 `PackageDownloadDone` 会被**静默丢弃** ⇒ bar 永不 `finish()`，后续直印全压在活跃 bar 行上。这是"丢事件"级失败路径，不是纯时序。
- **可选修法（若将来要做）**：延迟校验消息改走事件（专属事件，紧随 `PackageDownloadDone` 入队），由事件循环在 bar 关闭后打印；顺序由 FIFO 单消费者**确定性**保证，属正确入口而非借用语义/开关（对齐 [[no-patchwork-fixes]]）。用"事件序列断言"即可确定性钉住。

## 门禁（三项各自在改动树上跑，均通过）

- 改动文件 `rustfmt --edition 2024 --config skip_children=true`（`package/mod.rs`、`package/download.rs` 等含 `mod` 声明的文件必须带 `skip_children`，否则连带格式化子模块）；`cargo clippy --workspace --all-targets` **0 警告**；`cargo test --workspace` **exit 0**（libscoop 783+ 绿，含 3 条新测试）。
- 三项提交仅动 `crates/libscoop/**`、`src/{lib,eventloop,scoop_handler}.rs`、`locales/{en,zh}.yml`；`.agents/**` 未跟踪共享文档只在主树改，不入库。

## 教训

1. **"看不懂的错误串"要先分清是谁丢的信息**：reqwest 的 `Display` 天生不含 source 链，超时/连接/DNS 全长得一样——要么把它分类成类型，要么别指望文案能诊断；在错误串上做字符串识别是补丁。
2. **本地化要顺着"谁把错误嵌进模板"找全**：`translate_error` 只覆盖顶层传播的错误；`hok_i18n::t!("...", error = e)` 这类嵌套嵌入是第二、第三个表面，漏了就会出现"前缀中文、详情英文"。
3. **事件载荷是接口**：让前端反解 `app#1.0#hash.ext` 等于把缓存命名公式变成公开契约；结构化载荷 + 每包一次既修显示又消掉重复行。
4. **并发顺序问题别用 flaky 测试"复现"**：能确定性给出的只有通道/调用序的结构事实；该登记就登记，别为了"有测试"而造一个随机红绿的用例（对齐 [[investigate-first]] 与 [[no-patchwork-fixes]]）。
5. **`try_send` 满即丢对有"收尾语义"的事件是隐患**：bar 的 `finish()` 依赖 `PackageDownloadDone` 必达，低频控制事件与高频进度事件共用同一条容量 20 的通道时必须区分对待（进度已用 `send().await` 背压，控制事件仍是 `try_send`）。

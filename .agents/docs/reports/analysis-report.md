# 项目重构分析报告

## 一、需拆分的大文件
- crates/libscoop/src/package/checkver.rs | 1716 | checkver_source（github/http/regex 版本探测）、checkver_update（manifest 改写）、checkver_report（结果汇总）
- crates/libscoop/src/package/sync.rs | 1637 | sync_metadata（manifest.json/install.json 写入）、commit_one_install 继续拆（installer/脚本编排已含 operations 原语）
- crates/libscoop/src/package/manifest.rs | 1612 | manifest_spec（ManifestSpec 字段与解析）、install_info（InstallInfo）、license、shims/bin 访问器
- crates/libscoop/src/config.rs | 862 | config_defaults（default::* 路径常量）、config_set（set/commit 命令逻辑）
- crates/libscoop/src/internal/archive.rs | 816 | 按格式拆分：archive_7z / archive_zip / archive_tar / archive_inno
- crates/libscoop/src/package/query.rs | 798 | query_installed / query_synced（matchers 与缓存已内聚，可再拆）

## 二、职责模糊的模块
- crates/libscoop/src/package/sync.rs | install/remove/reset（与 sync 前缀不符） | 建议归宿：事务编排统一入口（现状可接受，但 install/remove/reset 与 sync 并存需文档化边界）
- crates/libscoop/src/package/checkver.rs | execute（命令处理器而非领域函数） | 建议归宿：execute 下沉到 cmd/checkver，领域逻辑保留 check_* 前缀函数
- crates/libscoop/src/package/auto_pr.rs | info/warn/err/progress/ok（输出原语，与 auto_pr 无关） | 建议归宿：提取到 cmd 层输出模块或 output.rs，auto_pr 只留 PR 生成逻辑
- crates/libscoop/src/package/checkhashes.rs、checkup.rs、checkurls.rs | check_hashes/check_installed/check_urls（各自单函数） | 建议归宿：保持，但统一为 internal/checker 风格的"扫描+报告"接口
- crates/libscoop/src/package/manifest_cache.rs | open/is_populated/populate/query（缓存基础设施） | 建议归宿：internal/db 或保留 package 内但去掉 pub 暴露面
- crates/libscoop/src/package/shim.rs 与 crates/libscoop/src/shim.rs 并存 | list_shims/shim_paths（查询） vs add/remove/Shim（安装） | 建议归宿：合并到单一 shim 模块，按职责分文件 shim/query.rs、shim/install.rs

## 三、重复代码组
- check_* 扫描/报告框架 | checkhashes.rs、checkup.rs、checkurls.rs、virustotal.rs | 建议公用位置：internal/checker（walk manifests + 逐项 check + 汇总报告）
- extract_dir/extract_to 解析 | manifest.rs、operations/extract.rs、internal/archive.rs | 建议公用位置：internal/archive（extract 参数解析集中化）
- compare_versions 实现 | internal/mod.rs、package/list.rs（compare_versions_desc） | 建议公用位置：internal/version（统一一处实现）
- remote_*（URL/文件名解析） | bucket.rs、internal/git.rs、internal/url.rs | 建议公用位置：internal/url（已存在，收敛 bucket/git 的重复解析）
- download_*（大小/进度聚合） | network.rs、package/download.rs、sync.rs | 建议公用位置：network 层（download_size 与进度事件已在 download 模块，需核对重复）

## 四、分层违规点
- crates/libscoop/src/package/auto_pr.rs | println!/eprintln! 直接写 stdout/stderr（7 处，非测试代码） | 迁移目标：移除 stdout 输出，改返回报告数据，由 cmd/auto_pr 用 output::info/err 呈现
- crates/libscoop/src/package/cleanup.rs | eprintln!（ignore_failure 分支） | 迁移目标：tracing::warn 或 emitter 事件
- crates/libscoop/src/package/sync.rs | eprintln!（commit_install/commit_remove 的 ignore_failure 分支） | 迁移目标：tracing::warn 或 emitter 事件
- src/cmd/ | 无（未发现 std::fs / reqwest / ureq / regex）

## 五、测试缺失模块
- crates/libscoop/src/package/operations/*.rs | 0 | 纯文件操作最易测：link_current/unlink_current（符号链接）、extract_markers（marker 解析）、persist_purge（目录删除）、copy_downloaded_files（跳过归档）
- crates/libscoop/src/package/auto_pr.rs | 0 | 去 println 后：PR 标题/正文/分支生成
- crates/libscoop/src/package/depends.rs | 0 | dependencies_tree 依赖图遍历
- crates/libscoop/src/package/export.rs | 0 | build_export JSON 结构
- crates/libscoop/src/package/missing_checkver.rs | 0 | scan 输出过滤
- crates/libscoop/src/package/manifest_cache.rs | 0 | open/populate/query（内存 SQLite）
- crates/libscoop/src/network.rs | 0 | download_page 的 UA/Referer/PRIVATE_HOSTS 注入（mock）
- crates/libscoop/src/config.rs | 0 | ConfigBuilder 加载、未知字段保留、set/commit 往返
- crates/libscoop/src/bucket.rs | 0 | bucket_added/manifests（含分类子目录）
- crates/libscoop/src/session.rs | 0 | new/new_with 回退链、set_global 后 effective_root_path
- crates/libscoop/src/internal/github.rs | 0 | 请求构造与响应解析（mock）
- crates/libscoop/src/internal/fs.rs | 0 | remove_symlink/symlink_dir 的目录/文件分支
- crates/libscoop/src/internal/network.rs | 0 | head/download 重定向与超时
- crates/libscoop/src/persist.rs、src/shim.rs、src/psmodule.rs、src/env.rs | 0 | link/unlink 的 persist 目录行为、shim 生成
- crates/libscoop/src/package/checkup.rs | 2 | check_installed 的 running 进程边界
- crates/libscoop/src/package/virustotal.rs | 1 | 仅字段访问，缺响应解析逻辑
- crates/libscoop/src/package/sync.rs | 15 | 均为 expand_vars/confirm/TempFileGuard 级，缺 install/remove/reset 主流程端到端测试

## 六、优先级排序（1-5条重构动作 + 估计工作量 小/中/大）
1. 修复 clippy 预存错误（download.rs 的 erasing_op/identity_op 断言、sync.rs 的 items_after_test_module），恢复 check/clippy/fmt 全绿 | 小
2. 清除 package 层 stdout/stderr 违规（auto_pr 7 处、cleanup/sync 各 1 处 eprintln!），输出统一走 emitter/tracing | 中
3. 拆分 checkver.rs（1716 行）为 source/update/report 子模块 | 大
4. 为 operations/* 原语补单元测试，并补 sync install/remove/reset 端到端测试（临时 root 上走完整流程） | 中
5. 继续拆 sync.rs 的 metadata 写入与 manifest.rs（1612 行），并清理 internal/{dag,os,path} 的整文件 allow(dead_code) 死代码 | 中

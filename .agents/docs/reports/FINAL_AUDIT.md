# 架构审计报告

> 审计范围：workspace 全部成员（crates/hok-i18n-derive、crates/hok-shim、crates/hok-shim-ref、crates/libscoop、crates/scoop_hash）+ 根 src/；已排除 target/、benches/、tests/、.github/、assets/、scripts/、locales/。
> 行数均按真实行（含空行）统计；`cargo check --workspace --all-targets` 通过。

## 1. 超大文件（>800 行）

- 文件路径 | 行数 | 建议拆分方案
- `crates/libscoop/src/package/checkver.rs` | 1716 | 拆 3 个：① checkver 核心（`execute`/`extract_version`/`apply_replace`/`run_checkver_script`）；② hash 采集（`download_and_hash_multi`/`head_digest_sha256`/`fetch_rdf_hash`/`fetch_metalink_hash`/`detect_hash_mode`/`extract_hash_from_page`/`format_hash`）；③ URL 解析（`github_api_url`/`extract_sourceforge_project`/`parse_sourceforge_url`/`parse_github_download_url`/`sub_url`）
- `crates/libscoop/src/package/sync.rs` | 1637 | 拆 3 个：① `Transaction` 模型（各 view/set 方法）；② install 流程（`install`/`commit_install`/`commit_one_install`/`run_script`/`expand_installer_vars`/`check_not_running`）；③ remove/reset 流程（`remove`/`commit_remove`/`commit_one_remove`/`reset`）
- `crates/libscoop/src/package/manifest.rs` | 1612 | 拆 3 个：① `Manifest` 数据模型 + 访问器；② serde 反序列化（各 `deserialize`/`visit_*` 块）；③ `License`/SPDX/URL 辅助
- `crates/hok-shim/src/main.rs` | 1249 | 拆 3 个：① `U16Buf` 等字符串工具；② shim 脚本解析（`parse`/`next_line`/`expand_dp0`）；③ 进程启动与环境（FFI 声明/Job/CreateProcess/env 展开）
- `crates/libscoop/src/config.rs` | 862 | 拆 2 个：结构体+访问器 / `default::` 默认值与 serde 逻辑（已有 `default` 子模块，可独立成文件）
- `crates/libscoop/src/internal/archive.rs` | 816 | 拆 2 个：解压（`extract`/密码/格式分发）与打包（zip/tar 写入）+ 测试

## 2. 职责不一致的模块

- 文件路径 | 冲突项 | 建议操作
- `crates/libscoop/src/package/sync.rs` | 文件名暗示"同步"，实际是 install/remove/reset 事务引擎（含 `reset`/`install`/`remove` 等 9 个 pub fn） | 更名 `transaction.rs`，或按安装/卸载拆分为 `install.rs`+`remove.rs`
- `crates/libscoop/src/package/checkver.rs` | 名为"版本检查"，实为 checkver + hash 采集 + GitHub/SF URL 解析混合 | 按第 1 节方案拆出 hash 与 URL 解析
- `crates/libscoop/src/package/list.rs` | 含 `compare_versions_desc`（版本比较逻辑），与"列出已安装版本"职责不符，且与 `internal::compare_versions` 双实现 | 合并进 `internal` 统一版本比较
- `crates/libscoop/src/package/formatjson.rs` | 含 `glob_to_regex`/`app_matches`（glob 匹配），与"格式化 JSON"职责不符 | 移到 `internal`
- `crates/libscoop/src/package/auto_pr.rs` | 含 `info`/`warn`/`err`/`progress`/`ok`/`done`/`header`/`named` 8 个输出 helper | 抽出独立 report 模块，或改私有
- `crates/libscoop/src/package/manifest.rs` | `is_spdx` 许可证校验混入 manifest 模型 | 提取到 `internal`

## 3. 分层违规点

- 文件路径 | 违规类型 | 修复建议
- `src/cmd/cat.rs`（:3, :71） | cmd 层直接 `std::process::Command` + `std::fs::read_to_string` | 迁移至 libscoop（package 读取能力）
- `src/cmd/create.rs`（:39） | cmd 层直接 `std::fs::write` | 迁移至 libscoop
- `src/cmd/import.rs`（:15） | cmd 层直接 `std::fs::read_to_string` | 迁移至 libscoop
- `src/cmd/config.rs`（:40-41） | cmd 层直接 `std::env::var` + `std::process::Command`（启动编辑器） | 由 libscoop 提供配置编辑能力
- `src/cmd/mod.rs`（:249）、`src/cmd/auto_pr.rs`（:61）、`src/cmd/virustotal.rs`（:30） | cmd 层直接 `std::env::var`（日志级别/令牌） | 改为参数注入或 libscoop 封装
- `src/util.rs`（:125） | 根 crate 直接 `std::fs::read_dir` | 迁移至 libscoop
- `crates/libscoop/src/package/auto_pr.rs`（:64-85） | package 层 7 处 `println!`/`eprintln!` 直出 | 返回结构化事件，由调用方渲染
- `crates/libscoop/src/package/cleanup.rs`（:98）、`crates/libscoop/src/package/sync.rs`（:848, :1126） | package 层 `eprintln!` 直接输出错误 | 返回 `Fallible`，由上层渲染

## 4. 死代码与允许项

- 文件路径 | 具体允许/死代码 | 处理建议
- `crates/libscoop/src/package/checkurls.rs`（:125, :134） | `#[allow(dead_code)]` 的 `strip_url_fragment`/`app_filter_matches`，仅测试引用 | 删除，或接入真实调用后移除 allow
- `crates/libscoop/src/package/manifest_cache.rs`（:18） | `#[allow(dead_code)]` 的 `CacheEntry` 结构 | 用于查询返回，否则删除
- `src/cui.rs`（:112-129） | `#[allow(dead_code)]` 的 `BucketUpdateUI`/`BucketState`（未接入任何命令） | 接入或删除
- `src/i18n.rs`（:73, :121） | `#[allow(dead_code)]` 的 `current_language`/`tr`（仅模块内使用） | 移除 allow 并私有化
- `src/output.rs`（:57, :66） | `#[allow(dead_code)]` 的 `is_color_enabled`/`is_scoop_style`（零使用） | 删除
- `crates/libscoop/src/package/auto_pr.rs`（:63-84） | 8 个 `pub fn` 输出 helper 无任何外部调用 | 改 `fn`（私有）
- 全 workspace | 无 `#![allow(dead_code)]`（仅上述 6 处局部 allow） | —

## 5. 测试覆盖缺口

- 模块路径 | 现有测试数 | 建议补充场景
- `package/config.rs`（862 行） | 0 | 默认路径解析、`SCOOP`/`SCOOP_GLOBAL` 环境变量覆盖、JSON 读写 roundtrip
- `package/bucket.rs`（487 行） | 0 | manifest 遍历、bucket add/remove、远程 URL 解析
- `package/session.rs` | 0 | global flag 切换、root path 解析、事件总线
- `package/network.rs` | 0 | `head_url` 重定向、下载落盘、超时
- `package/constant.rs`（549 行） | 0 | 内置列表/正则有效性
- `internal/git.rs` / `internal/github.rs` / `internal/os.rs` / `internal/env.rs` / `internal/fs.rs` / `internal/path.rs` / `internal/network.rs` | 0 | clone/ff 更新、API URL 构造、is_admin/进程判断、注册表读写、符号链接、路径归一化、HTTP HEAD
- `package/auto_pr.rs` | 0 | change 检测、repo NWO 解析、PR 构造
- `package/manifest_cache.rs` | 0 | 建表、增删查、SQL 转 `Manifest`
- `package/missing_checkver.rs` / `package/export.rs` / `package/import.rs` | 0 | 扫描、导出结构、导入 JSON 解析
- `package/operations/extract.rs` | 0 | 7z/zip/tar 解压、路径穿越防护、密码包
- `package/operations/link.rs` / `persist.rs` / `shortcut.rs` | 0 | current 链接切换、persist 目录联动、快捷方式创建
- `package/checkup.rs` | 2 | 损坏安装检测分支
- `package/shortcut.rs` | 2 | 快捷方式解析边界
- `package/virustotal.rs` | 1 | 响应解析、错误分支
- 另注：`tests/` 已按范围排除；`internal/arch.rs`(8)/`internal/dag.rs`(10)/`internal/archive.rs`(9)/`internal/url.rs`(7)、`package/checkver.rs`(27)/`sync.rs`(15)/`checkhashes.rs`(13) 覆盖良好

## 6. 重复代码组

- 功能描述 | 涉及文件 | 建议统一位置
- 版本比较（两套语义实现） | `internal/mod.rs::compare_versions` vs `package/list.rs::compare_versions_desc` | 统一到 `internal`（list 侧改为复用 + reverse）
- URL 文件名/fragment 解析（≥3 处） | `internal/url.rs::remote_filename`/`basename`、`internal/network.rs::strip_filename`、`package/checkurls.rs::strip_url_fragment`、`package/sync.rs` 内联 `split('#')` 逻辑 | 统一到 `internal/url.rs`
- GitHub/SourceForge URL 解析（≥4 个函数） | `package/checkver.rs`（`github_api_url`/`parse_github_download_url`/`extract_sourceforge_project`/`parse_sourceforge_url`）与 `internal/github.rs` | 统一到 `internal/github.rs`
- 目录递归遍历 read_dir 模式（≥4 处） | `package/bucket.rs::par_read_dir`、`package/list.rs`、`package/shim.rs`、`package/auto_pr.rs::read_manifests` | 统一到 `internal/fs.rs`
- 下载并计算哈希 | `package/checkver.rs::download_and_hash_multi`/`head_digest_sha256` vs `package/download.rs` vs `internal/network.rs` | 统一到 `internal/network.rs`

## 7. 依赖方向错误

- 违反依赖关系的文件 | 具体依赖 | 修复方案
- `src/util.rs`（:17） | 根 crate 直接 `use libscoop::internal::os::encode_wide` | 经 libscoop 公开 API 暴露
- `src/cmd/cat.rs`（:58） | 直接 `libscoop::internal::os::is_program_available` | 同上，封装进 libscoop
- `src/cmd/install.rs`（:13）、`src/cmd/reinstall.rs`（:2）、`src/cmd/uninstall.rs`（:2）、`src/cmd/update.rs`（:2） | 直接 `libscoop::internal::os::is_admin` | 提供 `Session::is_admin()` 之类的公开接口
- 说明：`libscoop/src/internal/` 本身无违规（use 核对仅依赖 `crate::error`/`crate::event`/`crate::Error`/`test_utils`，未引用 `Package`/`Bucket`）；但 `internal` 以 `pub mod` 全量暴露（`internal/mod.rs` 注释已承认），是根 crate 得以越层的根因，建议收敛为 `pub(crate)` + 按需 re-export

## 8. 综合优先级修复清单（≤5 条）

1. 拆分三大巨型文件 checkver.rs / sync.rs / manifest.rs（各自平级 2-3 个文件）| `crates/libscoop/src/package/` | 工作量：大
2. 收敛分层违规：src/cmd 与 src/util.rs 的直接系统调用迁入 libscoop，同时封堵根 crate → `libscoop::internal` 越层（暴露公开 API）| `src/cmd/*`、`src/util.rs`、`crates/libscoop/src/lib.rs`、`internal/os.rs` | 工作量：中
3. 统一重复实现：版本比较、URL 文件名/fragment 解析、GitHub/SF URL 解析、目录遍历收敛到 internal 并消除双实现 | `internal/mod.rs`、`internal/url.rs`、`internal/github.rs`、`internal/fs.rs`、`package/list.rs`、`package/checkurls.rs`、`package/checkver.rs`、`internal/network.rs` | 工作量：小
4. 清理死代码：删除/接入 6 处 `#[allow(dead_code)]` 项，私有化 auto_pr.rs 的 8 个 pub helper | `package/checkurls.rs`、`package/manifest_cache.rs`、`src/cui.rs`、`src/i18n.rs`、`src/output.rs`、`package/auto_pr.rs` | 工作量：小
5. 补测试盲区：优先为 0 测试的高风险模块补核心场景（config 路径解析、session 全局开关、bucket 遍历、operations/extract 解压与路径穿越、internal/git+github+os）| `crates/libscoop/src/{config.rs,session.rs,bucket.rs,package/operations/*,internal/*}` | 工作量：中

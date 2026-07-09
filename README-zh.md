# Hok — Scoop 的 Rust 再实现（社区维护版）

> 本项目是 [chawyehsu/hok](https://github.com/chawyehsu/hok) 的社区维护分支。
> 原作者已暂停开发，本分支独立维护，暂不合并上游。

---

## 关于本分支

原版 hok 是一个优秀的项目，用 Rust 实现了一个高效的 Scoop 替代品。但原作者[已有一段时间未更新](https://github.com/chawyehsu/hok)，而 Scoop 生态仍在演进。

本分支在继承原版所有功能的基础上，做了以下方向的改动：

### 轻量化改造

原版 hok 依赖了不少重型 crate，其中有些是纯 Rust 生态中「太重了但没更好选择」的妥协。本轮改造逐一复核并替换：

| 原依赖 | 替换方案 | 理由 |
|--------|---------|------|
| `chrono` | `jiff`（BurntSushi） | 纯 Rust 轻量时间库，regex 作者出品 |
| `futures` + thread-pool | `std::thread::spawn` | 项目中只用了一次异步，不值得拉整个运行时 |
| `sysinfo` | Win32 FFI（kernel32） | 只用了扫进程一个功能，sysinfo 是整个系统监控 |
| `curl` + static-curl | `ureq`（纯 Rust HTTP） | 去掉了 libcurl 的 C 编译，完全纯 Rust |
| `once_cell` | std（LazyLock/OnceCell） | Rust 1.70+ 已标准化，无需第三方 |
| scoop_hash selfcontained | rustcrypto 后端 | 手写 MD5/SHA1/SHA256/SHA512 ~4700 行是维护债务 |

### 功能补全

原版 hok 只实现了约一半的 Scoop 命令。本分支逐步补齐：

- **checkver 全套** —— 7 种版本提取模式（regex / JSONPath / XPath / Script / reverse / replace / GitHub / SourceForge）+ autoupdate 回写
- **SQLite manifest 缓存** —— `use_sqlite_cache`，兼容 Scoop 格式
- **新命令** —— `depends`、`prefix`、`which`、`checkup`、`alias`、`export`、`import`、`create`、`virustotal`、`shim`

### 修复的原版 bug

本分支修复了原版 hok（以及原版 Scoop）中的一些问题：

| Bug | 影响 | 修复方式 |
|-----|------|---------|
| **多包操作批量中断** | `install`/`update`/`cleanup` 等操作中，某个包失败会导致整个操作中断 | 实现 `ignore_failures` 配置 + `-f` 参数，失败时打印错误并继续处理剩余包 |
| **reset 不跑 post_install** | `hok reset <app>` 不会执行 manifest 中的 `post_install` 脚本，这其实是 **Scoop 原版的遗留 bug** | reset 命令现在正确执行 `post_install` |
| **版本比较不完整** | `compare_versions()` 对文本段直接返回 `Equal`（如 `1.0.0-beta` vs `1.0.0-alpha`） | 重写比较逻辑，支持数值/文本混合段、pre-release 优先级 |
| **死代码残留** | `get_content_length` 函数未使用，产生 warning | 删除，项目现为 **0 warning** |
| **下载无断点续传** | 分片下载中断后全部重来 | 支持 HTTP Range 续传，已下载的分片跳过，不完整的续传 |

### Aria2 配置复用

hok 虽然不使用 aria2c，但**复用了 Scoop 的 aria2 配置项**来控制内置的 HTTP 分片下载行为。二者使用的配置项完全兼容，用户无需额外配置。

| Scoop 配置项 | hok 行为 | 默认值 |
|-------------|---------|--------|
| `aria2-enabled` | 是否启用分片下载 | `true` |
| `aria2-split` | 分片连接数 | `5` |
| `aria2-max-connection-per-server` | 单服务器最大连接数 | `5` |
| `aria2-min-split-size` | 触发分片的最小文件体积 | `5M` |

当 `aria2-enabled` 为 `true`、文件大小超过 `min-split-size`、且分片数 > 1 时，hok 使用 `std::thread::scope` 启动多个线程，每个线程通过 HTTP `Range` 头并发下载一个分片，最后合并。这与 aria2c 的 Range 分片逻辑本质上是一致的，只是省去了 aria2c 这个外部进程调用。

```bash
# 配置示例（与原版 Scoop 完全一致）
hok config aria2-enabled true
hok config aria2-split 10
hok config aria2-min-split-size 10M
```

### 设计原则

- **纯 Rust 优先，但有底线** —— 能不用 C 编译就不用，但 `git2`（libgit2）比 `gix`（20 分钟编译）更务实。「Pure Rust first」有实际边界。
- **不要重复造轮子，但也不当冤大头** —— 标准算法（MD5/SHA）用现成 crate，平台特定 API（COM/Win32）用 raw FFI。后者没有合适的轻量 crate，几十行 FFI 比引入整个 crate 更合理。
- **兼容原版 Scoop** —— SQLite 缓存 schema、config 格式、autoupdate 行为均保持兼容。
- **零 warning 策略** —— 所有代码 0 warning，25 测试全过。

---

## 相关文档

- [用 Rust 写的 Scoop 再实现 — Chawye Hsu（原版博文）](./用%20Rust%20写的%20Scoop%20再实现%20-%20Chawye%20Hsu.md)
- [更新日志](./CHANGELOG.md)
- [命令列表（英文）](./README.md#commands)

## License

**hok** © [Chawye Hsu](https://github.com/chawyehsu) and [contributors](https://github.com/maboloshi/hok/graphs/contributors).
Released under the [Apache-2.0](../LICENSE) license.

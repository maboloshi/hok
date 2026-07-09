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

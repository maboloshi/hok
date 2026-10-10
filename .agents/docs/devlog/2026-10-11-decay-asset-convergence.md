# 衰减资产收敛：架构顺序 / hash 表 / nightly / 测试入口四处单点化

> 2026-10-11 ｜ 提交 `964bdcd`, `e33a474`, `1405580`, `26e88af` ｜ worktree `.worktrees/d1-decay-assets` ｜ 分支 `refactor/decay-assets`

"副本 → 单入口"，**零行为变更**、每项独立提交。判据本体：
`.agents/docs/active/audit/systemic-optimization-audit-2026-09.md` → 「使用规则：衰减资产 vs 投机性重构」A 表。

## 落地的三项 + 测试入口

- **架构固定顺序**（`964bdcd`）：`manifest.rs` 新增 `arch_specs()`，按 64bit→32bit→arm64 产出
  `(JSON pointer, &ArchitectureSpec)`；`all_urls` / `all_hashes` / `all_hash_segments` / `checkhashes_accessors`
  四处共用。`&arch.amd64` 手写数组 **4 → 1**。
- **hash 算法/前缀表**（`e33a474`）：`internal/hash.rs` 的 `HASH_ALGORITHMS` +
  `algorithm_for_digest_length()` 成为唯一表；`constant.rs` 的 `REGEX_HASH` 改由它生成（生成串与旧字面量逐字符相同），
  `checkver_hash.rs` 的长度 match 表删除。
- **nightly**（`1405580`）：`download.rs` ×2 与 `download_verify.rs` 的裸 `version() == "nightly"` 改走
  `Package::is_nightly`；裸比较 **3 → 0**。
- **测试入口**（`26e88af`）：`src/test_support.rs`（`cfg(test)`）与 `tests/common/mod.rs` 各一份；
  `fn plain()` **9 → 1**、`CARGO_BIN_EXE_hok` 生成点 **8 → 1**。合并前后各 harness 保留自己的根布局与额外 env。
  **用例数守恒**：1053 → 1053，名称集合 `Compare-Object` 完全相同。

## 停手并登记（语义确实不同，不得合并）

- **元数据目录解析**：`install_state.rs::current_meta_dir` 只按 `no_junction` 分派，与官方
  `lib/core.ps1:373-383 currentdir()` 逐字对应；`query.rs` 多一支——`current` 损坏（source = `VersionDirs`）时
  回退版本目录，这是 hok 有意的损坏安装识别，两侧各有测试钉住。**登记为有意偏离，不合并**（审计 §3.4 措辞需补）。
- **`checkhashes.rs` 的 nightly**：保留 `eq_ignore_ascii_case`（官方 `-eq` 不区分大小写），与 `is_nightly`
  的精确比较语义不同；已登记。⇒ `is_nightly` 的大小写对齐属**独立行为面**，未混入本次搬移。

## 官方对照（本机 `../Scoop` v0.6.0）

- 架构顺序：`bin/checkhashes.ps1:73-83`、`bin/checkurls.ps1:96-101` —— 顶层 `url` 优先，否则
  64bit → 32bit → arm64，与 hok 同序。
- hash：**`lib/` 下没有 `hash.ps1`**（勿按此名找）。算法集合真锚点是 `lib/download.ps1:761-773 get_hash()` 的
  `@('md5','sha1','sha256','sha512')`；前缀格式真锚点是 `lib/autoupdate.ps1:3-20 format_hash()`。
- nightly：`lib/install.ps1:21-25`（`-eq`，不区分大小写）。
- 元数据目录：`lib/core.ps1:373-383 currentdir()` + `lib/versions.ps1:31-105`。

## 门禁

`clippy -p libscoop --all-targets` 0；`clippy --workspace --all-targets` 0；`test -p libscoop` 0（868 passed）；
`test --workspace` 0；改动文件 `rustfmt --edition 2024 --check` 0（`src/i18n.rs` 的 2 条 diff 是 base 既有偏差，非本次）。
落地：base 与主线同 tip，`worktree-merge.ps1` 漂移校验四项全过、落地树与实测树逐字节相同。

## 新登记（不属本次范围）

- `SHA256:xxx` 大写前缀：官方 `get_hash` 的 `-notcontains`（不区分大小写）接受，hok `REGEX_HASH` 拒绝 —— 待拍板。
- `manifest.rs HashString` 的算法名映射：单点化需搬类型或加宏，违反"不引入新抽象层"，不做。

# `hok install` persist 链接 183：解压副本与已存在的 persist 目标撞车

> 2026-09-20 ｜ 2 提交 `8aa3dc5` `16a91cd`，worktree `.worktrees/s13-persist-original`

**现象**：`hok install scoop-private/sublime-text` 解压到 88/88 后立刻 `✗ sublime-text (当文件已存在时，无法创建该文件。 (os error 183))`。

**根因**：manifest `"persist": "Data"`，而 sublime 的 zip 自带**非空** `Data/KEEPME`，用户 `persist\sublime-text\Data` 又留着旧装的设置。`persist::link` 只处理了「目标不存在」，「目标已存在 + 源也存在」落进一个空分支；前置那三行 best-effort 清理里 `std::fs::remove_dir` 是**非递归**的，删不掉非空目录，随后 `symlink_dir` 在原路径建 junction → `ERROR_ALREADY_EXISTS`。上游 `persist_data`（lib/install.ps1:465-469）在这一支是 `Move-Item -Force $source "$source.original"`——`7b80fa5` 新增 `link()` 时只抄了三态里的两态，还顺手把 `fs::symlink_dir` 里的 `remove_symlink(lnk)?` 改成 `let _ =`，于是清理失败被吞掉，报错错位成建链的 183。

**修法**：`link()` 补上游分支（archive 副本重命名为 `<source>.original`，重复安装先清旧 `.original`）；前置清理只对**链接**生效（`symlink_metadata().file_type().is_symlink()`，Windows 的 junction 也算 symlink——探针实测 `is_symlink=true / is_dir=false`），因为 `remove_symlink` 连普通文件一起删，原写法会把 archive 自带的持久化文件悄悄换成空文件；`symlink_dir` 的清理失败改成「路径仍在则上报」，报 `DirectoryNotEmpty` 而不是 183。

**A/B 实测（隔离 scoop 根，同 zip 同 manifest）**：把 `USERPROFILE` 指向 scratch（hok 配置路径走 `$HOME/.config/hok`，**不是** `APPDATA`；Start Menu 快捷方式才走 `APPDATA`），预置 `persist\sublime-text\Data\settings.dat` 复现用户状态后跑 `install --offline`：**旧二进制（`D:\App\Scoop\shims\hok.exe` 0.2.0）逐字复现 183 且 exit 1**；**修复后的二进制 ✓ sublime-text**——`Data` 是指向 persist 的 Junction、`Data.original\KEEPME` 保留、旧 `settings.dat` 原样、`KEEPME` 未并进 persist、shim/快捷方式/install.json 齐全。真实 `D:\App\Scoop` 全程只读（沙箱），未受任何改动。

**门禁**：`rustfmt --edition 2024` 干净；`cargo test -p libscoop --lib` **757 passed / 2 failed**（仍是既有的沙箱 ACL 两条 `permission::tests::grant_users_write_inherit_*`，error 5）；`cargo clippy -p libscoop --all-targets -- -D warnings` 干净。新增 `persist` 单测 5 条 + `internal::fs` 2 条（该函数此前零测试）。

**教训**：① 「对齐上游」只抄一半比不抄更危险——落代码前把上游那一支读全；② `let _ =` 吞错会把故障点搬到下游，于是报错文本开始撒谎（183 指向建链，真因是清理）；③ 用例要覆盖「目标已存在 + archive 自带同名数据」这一真实包形态（sublime-text / sublime-merge 都是）。

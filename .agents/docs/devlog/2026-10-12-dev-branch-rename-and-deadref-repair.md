# 集成分支改名 dev 与两轮历史引用修复

> 2026-10-12 ｜ 提交 `eb5deca`（历史引用与落地策略），以及 0.3.0 发布准备｜ worktree 主树 ｜ 分支 `dev`（合并后由 `main` 承接发布提交）

本会话把「开发线要合并进 main」这件事一次做完：分支改名、落地策略由「禁 merge」放宽、把工作区里查不到
对象的提交号全部处置一遍、`dev` merge 进 `main`、并备好 0.3.0 发布。

## 1. `refactor/edition-2024` → `dev`

`git branch -m` 改本地名（416 条提交与全部 sha 不变），并 push 到 `origin/dev`。**历史记录里的旧名不回改**
（devlog / reports / active 的落地记录都写的是当时的名字）；从本条目起，引用集成分支一律写 `dev`。

## 1.1 `dev` merge 进 `main`

合并在 `main` 上做 `--no-ff -X theirs`，得到提交 `9027d8b`（双亲 `9e91b31` + `63793e1`）。校验：合并结果树与
`dev` tip **逐字节相同**（`git diff --stat dev HEAD` 为空），`main` 独有的两条修复（zip 反斜杠
分隔符、upgradable 缓存名）其内容早已由 dev 内的等价实现覆盖，故零内容变更。随后把 `dev` 指针对齐到同一提交，
两条线不再分叉。发布提交落在 `main` 而非 `dev`（早先误落在 dev 上，已重排到 `main`）；
`main` 与 `dev` 均已 push 到 origin。

## 2. 落地策略：禁 merge → 优先 ff、分叉允许 merge

判据是**不再重写已落地提交**：本仓 devlog 与审计大量引用提交号（本轮实测 241 条引用落在开发线上），
rebase 会重写这些提交，旧对象一旦被 gc 就永久读不出（本轮实测：50 条引用已因此变成死链，且本地对象库、
远端、reflog、`../hok_`、`../hok~` 全部找不回）。规则本体改在：

- `AGENTS.md` → Parallel work → Worktrees；
- `.agents/skills/hok-worktree-parallel.md` → 本仓事实（随 `eb5deca` 落地）。

## 3. 历史引用修复（两轮，共 79 处换活引用）

- **上游 Scoop / nsis-rs（64 处）**：补 40 位全 sha，并改成 markdown 链接写法——显示文本用 7 位短 sha、链接目标为上游仓库的 commit 页，显示位统一 7 位。
- **本仓有等价提交的死链（15 处）**：换成现役提交（P4/P5 那批：`7d294dc`/`d967c7f`/`b70f160`/`a80ea33`）。
- **本仓无等价提交的死链（23 处）**：保留原号 + 就地说明「已随 2026-08 重构失效落空」，不伪造替代。

**`reports/**` 冻结例外**：按用户明确指示修了 4 篇归档件的不可解析提交号与补注，另有 2 篇 `research/**`；
**正文一字未改**，只动 sha 与说明。这与 R5「归档后永久冻结」冲突，属**一次有意例外**。
归档件正文里的 `refactor/edition-2024` 指 §1 改名前的那条线，不另改。

## 4. 0.3.0 发布准备（在 `main` 上，即发布线 tip）

workspace 与四个 crate（`hok`/`libscoop`/`hok-shim`/`hok-i18n`）bump 到 0.3.0，`Cargo.lock` 同步；
`hok-i18n-derive` 保持独立的 0.1.0 线。`CHANGELOG.md` 手写 0.3.0 段（release-please 在本 fork 永不运行，
按 v0.2.0 的既有格式与 7 位短 sha 链接惯例）。**尚未 tag、未建 release 对象、未跑 `release.yml`**。

## 5. 门禁

`devlog-index.mjs --check` 一致、`doc-links.mjs` 断链 0、`docs-index-check.mjs` 缺口 0，均 exit 0。

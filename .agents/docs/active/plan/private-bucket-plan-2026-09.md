# hok 私有 Scoop bucket 支持 — 方案（2026-09-12，已评估待实现）

> 状态：**方案定稿，实现挂起**（用户 2026-09-12 决定：当前只落实方案，实现另开会话）。
> 2026-09-12 追加决策：多服务商鉴权走 `private_hosts` + `SCOOP_GH_TOKEN` / `SCOOP_GIT_TOKEN_<HOST>`，**不读**厂商标准名，不新增 hok 自有配置键。
> 本文是活文档（专题计划）；收官后按 `.agents/skills/hok-report-archive.md` 决定归档去向。
> 前置 skill：全局 `investigate-first`、`hok-code-organization`、`hok-rust-gate-practice`、`git-commit-practice`、全局 `proactive-review`、`no-patchwork-fixes`、`token-efficient-processing`。

---

## 1. 结论

- **私有 HTTPS bucket 今天就能用**（走 libgit2 → 系统 git credential helper 链，实测通过）；不需要改动即可跑通 GitHub 私有库。
- **私有 SSH bucket 完全不可用**：既缺 SSH 凭据分支，也缺 hostkey 校验（libgit2 无 known_hosts 处理）。
- **`PRIVATE_HOSTS` 与 `gh_token` 没有接入 git 传输**，私有库鉴权只能依赖系统凭据助手，与 hok「不依赖系统 git」的既有立场不一致。
- token 内嵌 URL 可用，但会明文落到 `buckets/<name>/.git/config`，并被 `hok bucket list` 打印。
- **已定设计（2026-09-12）**：多服务商鉴权 = `private_hosts`（通用口子）+ `SCOOP_GH_TOKEN` / `SCOOP_GIT_TOKEN_<HOST>` 环境变量层；**不读厂商标准名**（`GITHUB_TOKEN`/`GITLAB_TOKEN`/`GITEA_TOKEN`），不新增 hok 自有配置键。

## 2. 实测证据（2026-09-12）

命令：临时 `$SCOOP` 根 + `hok target/debug/hok.exe bucket add`，私有库取 `gh api '/user/repos?visibility=private'`。

| 场景 | 沙箱内（workspace-write） | 非沙箱（提权重跑） |
|---|---|---|
| 私有 HTTPS，无用户级凭据配置 | 失败 | **成功** |
| 私有 HTTPS + 临时 `credential.helper` 供 token | 失败 | **成功** |
| 私有 HTTPS + 本机 GCM（`GCM_INTERACTIVE=never`） | 失败 | **成功** |
| 私有 HTTPS + token 内嵌 URL | 失败 | **成功**；`.git/config` 含 token，`hok bucket list` 输出含 token |
| 私有 SSH（`git@github.com:<repo>.git`） | — | **失败** `invalid or unknown remote ssh hostkey; class=Ssh (23); code=Certificate (-17)` |
| 对照：公有 bucket（`ScoopInstaller/Main`） | 失败 | 成功 |

沙箱内失败原因与 hok 无关：沙箱下 Schannel 全域不可用（`curl`、Git for Windows 的 `git ls-remote` 同样报 `schannel: AcquireCredentialsHandle failed: SEC_E_NO_CREDENTIALS`，`gh` 走 Go TLS 不受影响）。**因此 TLS 相关结论只能以非沙箱实测为准。**

失败后只留下空 `buckets/` 目录，无半成品 `<name>` 目录 → 不会误报 `bucket_already_exists`（早期假设撤销）。

附带发现（非本议题）：`USERPROFILE` 与 `HOME` 同时缺失时 hok panic 于 `crates/libscoop/src/internal/fs.rs:63` 的 `unwrap()`；正常环境不可达，仅记录。

## 3. 现状代码依据

| 能力 | 现状 | 位置 |
|---|---|---|
| bucket 克隆入口 | 任意 git URL；libgit2（`git2` 0.19 / `libgit2-sys` 0.17+1.8.1，含 `libssh2-sys`） | `crates/libscoop/src/bucket.rs:421,446-449` |
| 凭据回调 | 仅 `USERNAME` / `USER_PASS_PLAINTEXT`（→ `Cred::credential_helper`）/ `DEFAULT`；**`SSH_KEY` 直接报错** | `crates/libscoop/src/internal/git.rs:16-31` |
| hostkey 校验 | 无 `certificate_check` 回调 | `crates/libscoop/src/internal/git.rs:12-48` |
| proxy | 仅 `config.proxy`，不读 `https_proxy` | `bucket.rs:426`、`git.rs:35-45` |
| `PRIVATE_HOSTS` | 只作用于资产/清单 HTTP 下载 | `config.rs:292,303-323`、`network.rs:41,108`、`checkver.rs:165,382` |
| `gh_token` | 只用于 GitHub API | `config.rs:232` |
| bucket 展示 | `Source` 列 = `remote_url`（token 会外显） | `bucket.rs:125,152-153` |
| 官方对照 | `git clone` + 失败清理目录 + `ls-remote` 预检 | `Scoop/lib/buckets.ps1:141-162` |
| 可复用的库能力 | `Cred::ssh_key_from_agent`（`git2 cred.rs:45`）、`FetchOptions::custom_headers`（`git2 remote.rs:503,586`）、`certificate_check`（`FnMut(&Cert, &str)`） | 均已具备，**无需新依赖** |

## 4. 实施方案

### P1a — SSH 凭据（`crates/libscoop/src/internal/git.rs`）

- `fetch_options` 的 credentials 回调新增 `CredentialType::SSH_KEY` 分支，来源顺序：
  1. `Cred::ssh_key_from_agent(user)`；
  2. `~/.ssh/id_ed25519` → `id_ecdsa` → `id_rsa`（`Cred::ssh_key(user, None, path, None)`）。
- libgit2 在一次连接中会重复回调直到某个凭据成功；用**回调调用计数器**推进来源，穷尽后报错并列出已尝试的来源。
- 加密私钥需要 ssh-agent（hok 无口令交互）→ 写入错误信息与文档。

### P1b — SSH hostkey TOFU（新模块 `internal/ssh_known_hosts.rs`，注册 `internal/mod.rs`）

语义对齐 `StrictHostKeyChecking=accept-new`：

| 情形 | 行为 |
|---|---|
| 同主机、同 key 类型、key 一致 | 接受（`Known`） |
| 同主机、同 key 类型、key 不一致 | **拒绝**，错误信息给出 known_hosts 路径与处置建议 |
| 主机未见过的 key 类型 | 追加写入 `~/.ssh/known_hosts`（`AcceptedNew`） |
| `@revoked` 命中 | 拒绝 |
| 哈希主机名（`|1|…`） | 无法比对 → 按未知处理（写明文条目，与 accept-new 一致） |
| 无 home 目录 / 文件不可写 | 接受但不落盘（`AcceptedUnrecorded`） |

- 挂接 `RemoteCallbacks::certificate_check`；非 SSH 证书返回 `CertificatePassthrough`，X.509 行为不变。
- home 目录复用 `internal::path::home_dir()`（`internal/path.rs:31`）。
- 单测（临时文件，绝不碰真实 known_hosts）：首次接受并落盘、变更拒绝且不覆盖旧条目、第二 key 类型追加、已有内容追加不丢失、`@revoked`、hosts 字段匹配（列表/`[host]:port`/哈希/不匹配）。

### P2 — 凭据接入 git 传输（`PRIVATE_HOSTS` + token 环境变量层）

- `Git` trait 签名加 headers：
  - `clone_repo(&self, remote_url: &str, path: &Path, proxy: Option<&str>, headers: &[String])`
  - `pull(&self, path: &Path, proxy: Option<&str>, headers: &[String])`
  - `RealGit` 内 `headers.to_vec()` 进 `spawn_blocking`，用 `FetchOptions::custom_headers` 注入（空切片 = 现行为）。
  - 波及：`internal/git.rs`（trait + RealGit + 4 处测试调用点）、`test_utils::FakeGit`（记录 headers）、`bucket.rs`（`add` 与 `update` 两处调用点）。
- headers 派生放**消费方 `bucket.rs`**（避免 internal 反向依赖 config），按 URL **主机**匹配，优先级从高到低：

| 优先级 | 来源 | 说明 |
|---|---|---|
| 1 | `private_hosts` 命中的 `Authorization` 头 | 用户显式配置，最高优先；通用口子，任意服务商/自建实例 |
| 2 | `SCOOP_GIT_TOKEN_<HOST>` | host 作用域 token；`<HOST>` = 主机名大写、非字母数字→`_`（`gitlab.com` → `SCOOP_GIT_TOKEN_GITLAB_COM`，`gitea.example.com` → `SCOOP_GIT_TOKEN_GITEA_EXAMPLE_COM`）；**只认主机名，不含端口**（端口区分场景用 `private_hosts`） |
| 3 | `SCOOP_GH_TOKEN` | 官方 Scoop 已有约定（`Scoop/lib/download.ps1:581`），env 覆盖 config，仅 github.com；hok 当前缺失，本次补齐 |
| 4 | config `gh_token` | Scoop 兼容，仅 github.com |

- token → 头：`Authorization: Basic base64("<user>:<token>")`，用户名默认按已知主机推断（github.com → `x-access-token`；gitlab.com → `oauth2`；bitbucket.org → `x-token-auth`），token 值支持 `user:token` 写法覆盖（自建 Gitea 等未知主机**必须**显式给用户名，否则不注入）。
- **安全约束（写入实现与文档）**：
  - 只对 `https://` URL 注入 token；`http://` 一律不注入（避免明文泄 token）；
  - **不做全局兜底** `SCOOP_GIT_TOKEN`：它会把 token 发给任意主机（含不可信 bucket URL）。若将来要做，必须配主机白名单；
  - `ssh://` / `git@host:` URL 静默跳过整个 token 层（SSH 传输没有 header 概念），不报错。
- 测试：`bucket.rs` 单测覆盖优先级（private_hosts > host env > `SCOOP_GH_TOKEN` > `gh_token`）、`user:token` 解析与默认用户名、`http://` 不注入、SSH URL 跳过；fake 记录 headers，`add` 与 `update` 两路径断言传递。

### HTTPS 与 SSH 并存语义

两套鉴权彼此独立，同一私有库可用任一协议添加：

| 协议 | 鉴权 | token 层 |
|---|---|---|
| `https://` | 系统凭据助手（现状，实测可用）→ 其次上面的 token 层 | 生效（仅 https） |
| `git@host:` / `ssh://` | 密钥/agent（P1a）+ known_hosts TOFU（P1b） | **静默跳过**（SSH 无 header 概念） |

建议文档：CI 用 HTTPS+token，桌面用 SSH。

### P3 — 文档

`README.md` / `README-zh.md` 增「私有 bucket」小节：

- HTTPS 走系统凭据助手（实测可用），以及 token 层：`SCOOP_GH_TOKEN`、`SCOOP_GIT_TOKEN_<HOST>`、config `gh_token`、`private_hosts` 的优先级表与 `user:token` 写法；
- 安全约定：token 只对 `https://` 生效；**不要**把 token 写进 bucket URL（会落到 `buckets/<name>/.git/config`，并被 `hok bucket list` 打印）；
- SSH：TOFU 行为（首次写入 `~/.ssh/known_hosts`、key 变更拒绝）、加密私钥需 agent；提示 libgit2 预期**不读** `~/.ssh/config`（无 `IdentityFile`/`ProxyCommand`/`Host` 别名，待实测确认），非默认端口/密钥路径需写全 URL 或用 agent；
- `hok config proxy`：git 传输只读该配置，不读 `https_proxy`（沙箱内 Schannel 故障曾复现此点的重要性）。

必要时同步 `config list --all` 的 proxy 描述文案（`locales/*.yml`）。

## 5. 提交拆分（一主题一 commit）

1. `feat(libscoop): authenticate git transports with private-host headers`（P2 接入 `FetchOptions::custom_headers` + trait/fake/调用点）
2. `feat(libscoop): resolve bucket credentials from Scoop-compatible token env vars`（`SCOOP_GH_TOKEN` / `SCOOP_GIT_TOKEN_<HOST>` / 优先级 / `user:token`）
3. `feat(libscoop): support SSH buckets (agent/keys + known_hosts TOFU)`（P1a + P1b）
4. `docs: document private buckets and proxy for git transports`（P3）

## 6. 验收

- `cargo test -p libscoop` 全绿（含新增用例）；`cargo clippy --workspace --all-targets -D warnings` 干净；rustfmt 按 edition 2024 单独跑（勿传含 `mod` 声明的文件，勿整包 fmt）。
- 非沙箱手工三验（沙箱内 TLS 不可用，**必须在普通 shell 验证**）：
  1. `hok config gh_token <token>` 后 `hok bucket add` 一个 GitHub 私有库成功；
  2. `private_hosts` 命中的自定义头路径成功；
  3. SSH URL 首次写入 `~/.ssh/known_hosts`，再次连接仍成功，篡改 known_hosts 中该条目后连接报「host key changed」。
- 手工可用脚本参考：`.agents/tmp/private-bucket-probe.ps1`（gitignored，本次评估所用）。

## 7. 风险与未验证项

| 项 | 说明 |
|---|---|
| 沙箱 TLS | workspace-write 沙箱下 Schannel 不可用，全部 TLS 结论只能非沙箱验证 |
| credential-helper 解析 | libgit2 如何在 PATH 上找到 `git-credential-manager`（本机 helper 可执行文件只在 git 的 `mingw64\bin`，不在 PATH）未查清；本机非沙箱无用户配置也克隆成功，说明凭据来自 Windows 凭据管理器。无 GCM 的机器行为未知，P2 正是其兜底 |
| 加密 SSH 私钥 | 不支持（无口令交互），需 agent |
| 哈希 known_hosts | `|1|` 条目无法比对，会追加明文条目（与 accept-new 一致） |
| token 只对 https 生效 | `http://` 不注入（防明文泄 token）；因此内网 http git 服务只能用 `private_hosts`/凭据助手 |
| 不做全局兜底 token | `SCOOP_GIT_TOKEN` 会把 token 发给任意主机（含不可信 bucket URL）→ 默认不实现；要做必须配白名单 |
| 未知主机需显式用户名 | 自建 Gitea/GitLab 等不在厂商表内，token 须写成 `user:token`，否则不注入（文档写清） |
| libgit2 与 `~/.ssh/config` | 预期不读（无 `IdentityFile`/`ProxyCommand`/`Host` 别名）→ **实现时实测确认**后写入文档 |
| 不引入外部 git | 全程 libgit2-only，符合 `internal/git.rs:1-5` 记录的既有决策（gix 因依赖树过大弃用） |

# 专项报告：hok 解析"过严"的性质、范围与选型（2026-09-18）

> **来源**：W2-A（H3 可见性）调研过程中发现解析侧才是根因，用户指示"可能需要解决 hok 解析严格的问题"。
> **关系**：本文件是 `h3-parse-visibility-design-2026-09.md`（可见性策略）的**姊妹篇**，两者是**独立工作流**。
> **状态**：**决策门——未改任何代码**（worktree `.worktrees/w2a-parse-visibility` 仍零改动）。

---

## 1. ★ 最重要的一个发现：这 80 个**全部**是"schema 非法、官方运行时容忍"

对 §3.3 的 80 个逐类核对官方 `schema.json` 与实际运行时代码后：

| 类别 | 数 | 官方 schema | 官方运行时 |
|---|---|---|---|
| `checkver.jsonpath` 对象形式 | 24 | ❌ 非法（`jsonPathPattern` 是 string） | ✅ `[String]` 强转，install 根本不看 checkver |
| `hash` 空白 `" "` / `---` / 裸 40-hex / 65-hex | 12 | ❌ 非法（`hashPattern` 只允许裸 64 或 `algo:` 前缀组合） | ✅ 解析通过；`get_hash` 无前缀一律当 **sha256** ⇒ 安装时 **hash mismatch** |
| 重复 JSON 键（`bin`/`url`/`installer`/`extract_dir`/`post_install`） | 7 | ❌ 非法 | ✅ `ConvertFrom-Json` **取末值** |
| `license` 对象缺 `identifier` | 7 | ❌ 非法（`required: [identifier]`） | ✅ 不校验 |
| `"keep": "true"` 字符串当 bool | 6 | ❌ 非法（`type: boolean`） | ✅ PS 自动转换 |
| `shortcuts` 扁平数组 / 对象数组 | 6 | ❌ 非法（每项须是 `minItems:2` 的数组） | ✅ 逐项 `@()` 包装，容错 |
| `suggest` 为字符串（`imagemagick`/`wavedb`/`openssl`） | 3 | ❌ 非法（须是 object） | ✅ 解析通过 |
| `uninstaller` / `autoupdate` 为字符串 | 2 | ❌ 非法（须是 object） | ✅ 解析通过（install 不看 autoupdate） |
| `autoupdate.hash.regex` 为数组 | 2 | ❌ 非法（须是 string） | ⚠️ `[String]` 强转成空格串 ⇒ 实际也提取不到 |
| 顶层 `hash` 为 `{url}` 对象 | 2 | ❌ 非法 | ✅ 解析通过 |
| 其它单例（`depends` 为 map、`floating point 1.43`、`integer 2` …） | 9 | ❌ 非法 | ✅ 解析通过 |

**结论（这是整个决策的前提）：**

> **hok 现在是"schema 严格 + 运行时严格"；官方是"schema 严格（仅文档/CI 用）+ 运行时极宽"。**
> 这 80 个里**没有一个**是"官方支持、hok 缺失的功能"，**全都是"官方 schema 判非法、但官方运行时照单全收"的形状**。

这一点直接决定了目标契约该怎么定 —— 我们不是在"补齐 hok 缺失的特性"，而是在**选择 hok 要不要模仿官方运行时的宽容度**。

> ⚠️ **必须同时纠正一个说法**："80/80 官方都接受" ≠ "80/80 官方都能装"。其中 **12 个 hash 形状异常的，官方能搜到、但装到 hash 校验时会失败**（`download.ps1:721-731` + `get_hash:754-766`：无前缀一律按 sha256 比，必然 mismatch）。
> 但官方的失败**指向真问题**（"hash 不符"），hok 的失败**指向假问题**（"包不存在"）—— 后者仍然更差。
> （出站 HTTPS 不可用，**install 侧未实测**，此判定来自官方源码分析。）

---

## 2. 现状：既有审计的 P0 表已基本关闭，这 80 个是**新的一类**

对照 `.agents/docs/reports/manifest-parse-audit.md`（2026-08-10，2026-09-13 加 P0-4）：

| 审计项 | 当前状态（本次核实） |
|---|---|
| P0-1 `hashPattern` 长度组合 | ✅ **已修**：`constant.rs:98-101` 现在镜像官方 pattern（裸 64 / `algo:` + 32/40/64/128 任意组合） |
| P0-2 `sourceforge.path` 必填 | ✅ **已修**：`manifest_parse.rs:258-259` 两者皆 `Option` |
| P0-3 `env_set`/`cookie` value 类型 | ✅ **已修**：`deserialize_string_map`（`manifest_parse.rs:495`）+ 测试 `test_env_set_coerces_non_string_values` |
| P0-4 `hash.mode: github` | ✅ **已修**：`eb67941` |
| **"schema 非法但官方运行时容忍"这一整类** | ❌ **审计里完全没有这一栏** ⇒ 这就是那 80 个的归属，需要新建一栏 |

⇒ 本工作流**不是**在续做旧 P0，而是**给审计补一个缺失的分类维度**。

---

## 3. 逐类改法、代价与风险

位置均在 `crates/libscoop/src/package/`。风险列的"**接受后 hok 会怎么做**"是关键 —— 有些字段被丢掉的后果是**静默少做事**（比报错更糟）。

| # | 类别 | 数 | hok 位置 | 改法 | 接受后 hok 的行为 | 代价 | 风险 |
|---|---|---|---|---|---|---|---|
| G1 | `license` 缺 `identifier` | 7 | `manifest_parse.rs:193-214` | `identifier` 局部改 `Option`，缺省 `""`（与 `License::default()` 一致） | 完全正确（`info` 显示空标识符，同官方） | ~3 行 | **极低** |
| G2 | bool 为字符串（`keep`） | 6 | `Installer.keep: Option<bool>` 等 | 复用 `deserialize_string_map` 的思路加一个 lenient bool | 完全正确 | ~15 行 | **低** |
| G3 | `shortcuts` 扁平数组/对象数组 | 6 | `ManifestSpec.shortcuts: Option<Vec<Vec<String>>>` | 抄现成的 `deserialize_bin`（`:383-443`）写 `deserialize_shortcuts` | 正确（扁平数组按官方"逐项 `@()`"语义解释） | ~40 行 | **低** |
| G4 | `suggest` 为字符串 / `depends` 为 map | 4 | 同型字段 | 同 G3 思路的 lenient visitor | 字符串 → 视作单候选；map → 用 key 作候选名 | ~40 行 | **低-中** |
| G5 | `autoupdate` / `uninstaller` 为字符串 | 2 | `Autoupdate` / `Installer` | 自定义 visitor：字符串 → 视作 `{url: s}` / `{script: s}` | install 不看 autoupdate ⇒ 无影响 | ~30 行 | **低** |
| G6 | `hash` 为空白 `" "` / `---` | 9 | `manifest.rs:1435` | `raw.trim()` 后为空 → `HashString::Empty` | 与官方 `!$hash` 语义一致（无校验 + 警告） | **1 行** | **低** |
| G7 | 重复 JSON 键（取末值） | 7 | 无（serde derive 默认拒收） | **`parse_file` 严格解析失败时，回退 `Value → from_value` 重试** | 与官方 `ConvertFrom-Json` 完全相同 | ~8 行 | **低**（见下） |
| G8 | `checkver.jsonpath` 对象形式 | **24** | `Checkver.jsonpath: Option<String>`（`manifest.rs:294`）+ `manifest_parse.rs:302` | 自定义 visitor 接受 string \| map | map 形式 hok 无模型 ⇒ **只能忽略**（官方也只是强转成串、实际失效） | ~30 行 | **中**（静默忽略一个字段） |
| G9 | `autoupdate.hash.regex` 数组 | 2 | `manifest_parse.rs:130` | 放宽为字符串或数组 | 只能取首项或忽略（官方强转成空格串，同样提不到） | ~10 行 | **低** |
| G10 | 顶层 `hash` 为 `{url}` 对象 | 2 | `ManifestSpec.hash: Option<Vectorized<HashString>>` | 接受 map（`{url,hash}` 模型已在 autoupdate 层存在） | 正确 | ~25 行 | **中** |
| G11 | 其它单例 | 9 | 混杂 | 逐例判定 | — | 中 | 中 |
| G12 | 裸 40-hex / 65-hex hash | 3 | `constant.rs:98` `REGEX_HASH` | 放宽或按长度推断算法 | 官方按 sha256 ⇒ 安装时 mismatch | 小 | **高（与 S10 决定冲突，见 §5）** |
| G13 | 非 UTF-8 文件 | 1 | `manifest.rs:837-842` | **不修**（hok 这里比官方更正确） | — | — | — |

**G7 的细节（值得单独说）**：`serde_json` 的 `Value` 本身允许重复键（末值胜），只有 derive 出来的 struct 才拒收。
所以**不必**把 `parse_str` 全局改成 `Value` 中转（那会打在主瓶颈上，17k 清单每次多一次分配）。做法是：
> 严格路径**先跑**；仅当它失败时，才用 `Value → from_value` 重试 —— 于是**只有现在就会整条失败的 0.4% 文件**才付这份代价，热路径零开销，且只会"救活"重复键这一类（其它类型不匹配重试照样失败）。

---

## 4. 目标契约的三种定法（这是真正要拍板的）

| 选项 | 契约 | 覆盖 | 代价 | 风险 |
|---|---|---|---|---|
| **P-A 运行时等价** | "官方运行时能解析的，hok 都能解析" | 80/80（除 G13） | 中-大（逐字段 lenient 化，长期契约变模糊） | **高**：hok 会接受它**无法正确建模**的清单，然后在安装时用错误的数据做事（静默少装依赖、hash 语义错位） |
| **P-B 只做可见性**（= H3） | 保持严格，但**永不静默** | 0（不减少不可用包） | 小 | 低。但 80 个包在 hok 里仍然不可用 |
| **P-C 分层放宽（推荐）** | 按"**hok 会不会据此做危险动作**"划线：<br>· **可以安全忽略/能正确建模**的字段 → 放宽（G1-G11）<br>· **关乎正确性与安全**的 → 保持严格（G12 hash 形状、G13 非 UTF-8） | **~66/80** | 中 | 中-低。剩下 14 个必须靠 P-B 兜底 |

**为什么推荐 P-C 而不是 P-A**：G3/G4/G8 这类字段一旦"接受但忽略"，hok 会**静默少做事** —— 例如 `depends` 被丢掉，`install` 就不装依赖了；`checkver.jsonpath` 被丢掉，`checkver` 就永远查不到新版本。**"拒绝整条清单"至少是响亮的**（一旦补上可见性），"接受但丢字段"反而更难发现。所以放宽必须**逐字段**判定，并且"接受了但没建模"的字段要留下诊断。

**为什么 P-C 也替代不了 P-B**：P-C 之后仍有 14 个会被拒收；而且官方宽容度是 `ConvertFrom-Json` 的行为、**"不是一个稳定契约"**（hok 模块文档自己的原话），新形状会持续出现。**没有可见性，下一次新形状又是"包凭空消失"。**

---

## 5. ⚠️ 与既有决定冲突的点（必须你拍板，不能顺手改）

**G12（hash 形状）与 S10 的决定冲突。**
S10（`command-deviation-fix-plan-2026-09.md` §S10）当初明确决定：
> "`TODO`/未知算法/`{url,hash}` 对象**不再静默 skip**，改显式报解析失败（**不放宽 `HashString` 解析层**，避免波及 install/check_hash）"。

现在若放宽 `REGEX_HASH`/`HashString`，就是在**反向推翻 S10 的一半**。所以 G12 必须单列为一档，且要先回答：

- 放宽度到哪？(a) 只做 `trim()`（G6，无争议）；(b) 再加"按长度推断算法"（裸 40 → sha1，对齐官方 `get_hash` 的实际**能力**而非其当前行为）；(c) 全放宽。
- 放宽后 `checkhashes` 的"显式报解析失败"要跟着改吗？（S10 的核心诉求是"别静默"，放宽后这类文件根本不会到那一步）

> 注意 (b) 有个反直觉点：官方 `get_hash` 对无前缀**一律当 sha256**（不按长度推断），所以裸 40-hex 在官方那里**装不上**。若 hok 按长度推断成 sha1，hok 反而**比官方更宽容、也更能装**—— 这是"偏离官方"，需要你确认为"更好"还是"不碰"。

---

## 6. 建议的工作流切分与顺序

两个工作流**独立**，但**都做才闭环**：

| 序 | 工作流 | 内容 | 量级 | 依赖 |
|---|---|---|---|---|
| 1 | **H3 可见性（P-B）** | 显式查询逐条警告 + 全量扫描汇总 + `checkver`/`checkurls` 对齐 + `parse_file` UTF-8 分支补 warn | ~80-120 行 | 无。**先做**，因为它是 P-C 的验收手段（"80 → N" 必须在产品里可测，而不是只在我的分析里） |
| 2a | **容忍度 W1（低风险）** | G1-G7、G9（license/bool/shortcuts/suggest+depends/autoupdate+uninstaller/hash 空白/重复键/regex 数组） | ~170 行 + 测试 | 与 2b 无交集 |
| 2b | **容忍度 W2（大块，需定契约）** | G8 `checkver.jsonpath` 对象（24 个）+ G10 顶层 hash 对象 + G11 单例 | ~70 行 + 测试 | 需先定"忽略但留诊断"的机制 |
| 3 | **G12 hash 形状** | 与 S10 决定冲突，单独拍板 | 小 | §5 的答复 |
| 4 | **G13 非 UTF-8** | 不修，靠可见性 | — | — |
| 5 | **审计回填** | `manifest-parse-audit.md` 新增"schema 非法/运行时容忍"一栏；旧 P0-1/P0-2/P0-3/P0-4 标 ✅；`scoop-gap-audit` 条目 | — | 2a/2b/3 |

**预期效果**：W1 + W2 落地后，本机 4 bucket 不可解析数 **80 → ~14**（G12 的 12 + G13 的 1 + G11 残留）；再拍板 G12 后可到 **~2**。

---

## 7. 需要你拍板

1. **目标契约选哪个？** P-A（运行时等价）/ **P-B（只可见性）** / **P-C（分层放宽，推荐）**。
2. **顺序**：同意"**先可见性、再放宽**"（推荐，可见性是放宽的验收手段），还是先放宽？
3. **G12 hash 形状**：既然是 S10 的反向决定，怎么办？(a) 只 `trim()`；(b) 加"按长度推断算法"（hok 会比官方更能装）；(c) 全放宽；(d) 不动。
4. **G8 `checkver.jsonpath` 对象（24 个，最大一块）**：接受但忽略（官方也提不到版本），还是继续拒收、（若选 P-B）只把它变成响亮的错误？
5. **审计回填**：同意在 `manifest-parse-audit.md` 新增"schema 非法 / 官方运行时容忍"一栏，并把旧 P0 标 ✅ 吗？

---

## 8. 可复跑的证据命令

```powershell
# 类别 → 文件 的完整映射（含 serde 原始报错）
$env:RUST_LOG="libscoop=warn"; $env:SCOOP="D:\App\Scoop"
.worktrees\w2a-parse-visibility\target\verify\hok.exe missing-checkver `
  -d D:\App\Scoop\buckets\scoop-app\bucket 2>&1 |
  Select-String "failed to parse manifest|Error parsing JSON|无法解析"

# 官方运行时是否接受（80/80）
pwsh       -NoProfile -File $env:TEMP\w2a-official-parse.ps1   # ps=7.6.6      ok=80 fail=0
powershell -NoProfile -File $env:TEMP\w2a-official-parse.ps1   # ps=5.1.26100  ok=80 fail=0

# 官方 hash 判定逻辑（证明 12 个 hash 类是"装时 mismatch"而非"能装"）
#   Scoop\lib\download.ps1:721-731 check_hash / :754-766 get_hash
#   Scoop\schema.json:6-8      hashPattern（证明这些形状本来就是 schema 非法）
```

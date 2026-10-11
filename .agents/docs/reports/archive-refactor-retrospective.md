# 复盘：archive.rs 反复修改的教训（2026-08-16）

> 触发：doubao 更新无进度条 → 一路修到 `extract_7z` 分流重构，同一函数在
> 4 个提交里改了 550+ 行（230/207/69/46），且出现"加-删-加"重复。本文提炼
> 过程性教训，供后续会话避免同类返工。

## 1. 事件线（本轮）

| 提交 | 动作 | 规模 |
|---|---|---|
| `b268f63` | 修 7z.exe 进度（`\r` 分隔） | +15/-3 |
| `58e82ef` | 加 zip SFX 提取（EOCD 定位） | +230 |
| `379b7b0` | zip 并行 worker 池 | +207/-90 |
| `5fa74c3` | 分流平行化 + Arc 共享 + worker 排序 | +69/-34 |
| `5cb14a6` | 单遍 classifier + Inno 分支 + 删死代码（原 `2679be7`，已 amend） | +46/-20 |

同一文件 1772 行,其中 extract_7z 分流逻辑被重写 3 遍。

## 2. 教训一：先理清"它是什么"再写"怎么解"

**错误模式**：拿到 doubao 无进度条的 bug,第一反应是"7z.exe 进度解析坏了"。
实际 doubao 根本不是 7z——它是 **PE 前缀 ZIP(SFX)**,连 7z.exe 路径都不该走。
进度条只是表象,真实问题是**格式识别分流缺失**。

**正确顺序**（本次教训）：
1. **先鉴定文件真实格式**（magic / EOCD / installer 签名）→ doubao = PE+zip;
2. 再决定走哪条路（原生 zip vs 7z.exe）;
3. 最后才轮到"进度条为什么不显示"这类次要问题。

**可操作检查**：改解压逻辑前,先问"这个文件到底是什么格式?"用
`7z.exe l -slt` / 手工读 magic 确认,而不是假设扩展名=格式。

## 3. 教训二：所有权(move)会扭曲控制流,先想清楚生命周期

**错误模式**：`extract_zip_embedded(file_data, ...)` 把 614MB 的 `Vec<u8>`
move 进 worker,导致 zip 失败后 `file_data` 没了,NSIS 检查无法执行 → 被迫在
zip 分支内 `return extract_with_7z_exe`（重复兜底）→ 后来为去重又引入
`Arc<Vec<u8>>` 共享 → 又导致"NSIS 判定要提前算布尔"的别扭结构。

**正确做法**：数据要跨线程共享时,**一开始就用 `Arc`**,不要等 move 冲突
冒出来再补。`Arc<Vec<u8>>` clone 是 O(1),代价可忽略;它让所有分支都能
`clone()` 后继续用原引用,控制流保持自然。

**可操作检查**：函数签名里出现 `Vec<u8>` 且会被多线程借用 → 先想是否该是
`Arc<Vec<u8>>`。

## 4. 教训三：检测逻辑要"单遍分类",不要按需拼谓词

**错误模式**：先有 `is_nsis_installer`（扫 1MiB）,加 Inno 时又写
`is_innosetup_installer`（再扫 1MiB）,然后 `is_installer_pe` 扫第三次,
最后 `extract_7z` 里还要再调一次。同一文件的头部被扫 3~4 遍,且各谓词
可能对同一文件得出矛盾结论。

**正确做法**：一个 `classify_installer() -> InstallerKind` 单遍扫描,
所有消费方（SFX 探测 / dispatch / 测试）都基于同一分类,结构上不可能
不一致。

**可操作检查**：多个 `is_*_installer(data)` 谓词存在 → 合并为
`classify(data) -> Enum`,谓词变薄包装或删除。

## 5. 教训四：两个看似平行的检测,先确认是否真的互斥/同级

**错误模式**：把 zip-SFX 检测嵌套在 `pe_embedded_sfx_searchable`（7z SFX
探测）里,因为"都是 PE 里的东西"。实际上 zip 的 EOCD 探测与 NSIS/Inno
签名无关,嵌套使 zip 分支被错误地限制在"非 installer 的 PE"内,还制造了
"zip 失败后要不要再试 NSIS"的困惑（当时还加了个恒为 false 的死分支）。

**正确做法**：用**文件真实格式**做分类,而非"看起来像什么"。7z SFX 和
zip SFX 是并列的两种 payload 类型,NSIS/Inno 是并列的两种 installer 类型
——各自独立检测,按优先级串联。

**可操作检查**：画一遍 `match` 分支树,每个分支的前提条件是否互斥、是否
覆盖全部情况;出现"某分支的前提隐含了另一分支不可能"时要警觉（死代码
信号）。

## 6. 教训五：防御性"万一"代码大多成为死代码,别预先写

**错误模式**：zip 分支里加 `if is_nsis && extract_nsis(...)` 重试,理由是
"万一 zip 失败还能试 NSIS"。但进入 zip 分支的前提（`pe_embedded_sfx_searchable`
= MZ && !NSIS && !Inno）已排除 NSIS——这段代码**永远不执行**。

**可操作检查**：写"万一"分支前,用前置条件推理一遍它是否可达;不可达就
不写。更早的教训"no-patchwork-fixes"同源：别用额外的语义掩盖主路径。

## 7. 教训六：一次会话反复改同一文件,要警惕"范围蔓延"

从"进度条不显示"到 "zip SFX 原生解压" → "zip 并行" → "分流重构" →
"classifier 合并" → "删死代码" → "合并提交" → "再合并提交"……每个都是
合理的小改进,但累积起来就是 4 个提交 550 行,而且中间还穿插了
"拆提交→又合并提交"的历史整理返工。

**可操作检查**：
- 修 bug 时只做最小修复,性能/重构另立任务（本次拆出 zip 并行是正确示范）;
- 提交粒度：一个主题一个提交,但**同一主题内别反复 amend/合并**——先
  想清楚提交边界再落笔;
- 文件超过 ~1500 行或 extract_7z 这种多分支函数,主动考虑拆模块。

## 8. 已经沉淀的正确状态（供后续直接引用）

- `extract_pseudo_7z` 分流（当前 `5cb14a6`，2026-08-16 最终形态）：
  ① 7z magic → 纯 7z；② `classify_installer` 单遍分类（唯一检测入口）：
  `None` 分支 → MZ 限定搜 7z SFX + 无条件 zip EOCD 探测（zip crate 并行）；
  `Nsis`/`Inno` → nsis/innospect crate；③ 兜底 7z.exe。
- 检测：`classify_installer` 单遍扫描 1MiB → `InstallerKind`；`is_installer_pe`
  是唯一保留的薄包装。
- 数据：`Arc<Vec<u8>>` 全程共享,zip 分支 clone 不 move,单一 7z.exe 兜底。
- 性能：zip worker 按 compressed_size 降序领取 + 目录缓存;doubao
  26.3s → 21.1s（debug）。
- 设计文档见 `.agents/docs/research/zip-sfx-research.md`（rc-zip 对比、算法、验证数据）。

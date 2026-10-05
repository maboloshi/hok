# search 输出修改演示

Status: active
Updated: 2026-10-05

参考来源：`hok list` 表格布局（`src/cmd/list.rs:layout`）、`hyperscoop` 边框风格、`scoop-search.ps1` 原版行格式。

## 1. 无参数行为（修无参数输出问题）

当前：`hok search` → 执行 `*` 全量搜索，输出数千行 `status()` 灰色行（带 `...`）。
修正：无参数时直接提示退出，不执行搜索。

```
$ hok search
WARN No query provided. Usage: hok search <query> [options]
```
退出码：1（与 scoop `search` 无参数时的行为一致，可被脚本检测）。

## 2. 有参数时的新表格输出（参考 `hok list` 表格风格）

列布局（参考 `list.rs` 的 `layout` 函数：accent_bold 表头 + `-` 规则线 + emphasis 名称）：

- 始终存在列：`Name` | `Version` | `Source` | `Installed`
- 条件列（根据 `--with-binary` / `--with-description` 添加）：`Binary` | `Description`

### 2.1 基础搜索（无额外标志）

输入：`hok search git`

```
  :: Search results for 'git' ...
  Name      Version     Source    Installed
  --------  ---------   ------    ---------
  git       2.51.0      main      2.51.0
  git-lfs   3.6.1       extras
```

说明：
- `Installed` 列：已安装时显示版本号；未安装时留空（不显示 `-`，避免视觉噪音，参考 `list` 对空列的处理）。
- 包名（`Name`）用 `style::emphasis`（粗体默认前景色），与 `list` 一致。
- 表头用 `style::accent_bold`，分隔线用 `style::dim("-".repeat(...))`，与 `list` 完全一致。

### 2.2 带 `--with-binary`

输入：`hok search -B 7zip`

```
  :: Search results for '7zip' ...
  Name      Version     Source    Installed  Binary
  --------  ---------   ------    ---------  ------
  7zip      26.03       main      26.03      7z.exe, 7zG.exe
  7-zip     24.09       extras
```

### 2.3 带 `--with-description`

输入：`hok search -D firefox`

```
  :: Search results for 'firefox' ...
  Name      Version     Source    Installed  Description
  --------  ---------   ------    ---------  -----------
  firefox   140.0      main                 Mozilla Firefox web browser
```

### 2.4 带安装版本差异标注（保留现状语义）

输入：搜索结果中包含已安装但版本不同的包（例如已安装 `2.50.0`，最新为 `2.51.0`）：

```
  :: Search results for 'git' ...
  Name      Version     Source    Installed
  --------  ---------   ------    ---------
  git       2.51.0      main      2.50.0 → 2.51.0
```

注意：已安装版本与最新版本不同时，`Installed` 列内显示已安装版本并附版本差异提示（参考 `list` 的升级箭头语义）。

### 2.5 结果数量提示（参考 `hyperscoop` 的计数提示）

在表头上方增加一行计数提示（参考 `hyperscoop` 的 `Results from local buckets...`）：

```
  3 results found.
  :: Search results for 'git' ...
  Name      Version     Source    Installed
  ...
```

底部汇总行（参考 `list` 等查询命令的底部总结）：

```
  找到 256 个匹配 'git' 的包
```

## 3. 代码修改点概览

文件：`src/cmd/search.rs`

修改内容：
1. `execute()` 开头：检查 `args.query.is_empty()`，若为空则 `output::warn(...)` 并 `return Err(crate::ExitCode(1).into())`。
2. 移除当前 `for pkg in packages { ... output::status(&line); }` 的逐行输出，替换为表格布局函数（参考 `list.rs:layout`）。
3. 新增 `Columns`（条件列控制）、`SearchRow` 结构体。
4. 根据 `args.with_binary` / `args.with_description` 动态决定列数和列宽。
5. `Installed` 列数据来源：`pkg.is_strictly_installed()` → 显示 `pkg.installed_version()`（若与 `pkg.version()` 相同则显示版本；不同则显示已安装版本并附差异提示）。若未安装则留空。
6. 底部增加汇总行（参考 `list` 等查询命令）。

参考实现：直接复用 `list.rs` 的 `visual_width`、`pad_visual`、`style` 模块，无需新增依赖。

## 4. 与参考源的对应关系

| 参考源            | 在本修改中采用的部分                              |
|--------------------|---------------------------------------------------|
| `hok list`         | 表头样式（`accent_bold`）、分隔线（`dim("-"...)`）、列对齐（`pad_visual`）、包名强调（`emphasis`）、底部汇总 |
| `hyperscoop (hp)`  | 计数提示行（`3 results found.`）、表格边框概念（本实现不画 `│` 边框，保持简洁，与 `list` 一致）|
| `scoop-search.ps1` | 无参数时提示退出（参考 `search` 无参数行为）、已安装标注（`[installed]` 语义保留为 `Installed` 列）|

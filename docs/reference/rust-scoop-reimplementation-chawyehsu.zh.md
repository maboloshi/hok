# 用 Rust 写的 Scoop 再实现 - Chawye Hsu

> 原文：https://chawyehsu.com/blog/reimplementing-scoop-in-rust
> 作者：Chawye Hsu（2023/08/10）

开门见山，先放上项目地址：[https://github.com/chawyehsu/hok](https://github.com/chawyehsu/hok)

## 前言

了解 Scoop 或者已经是其用户的朋友从标题已经知道本文主题，大概点了上面的链接跳走了。没点的只好留下来，听我先给不了解的人稍微介绍下 Scoop。[Scoop](https://scoop.sh) 是一个在 Windows 下的工具，通过一套规则来描述应用软件，然后提供命令行的方式来让用户相对方便地安装和管理使用的软件。它最初的开发受到过 Homebrew 的启发，虽然随着发展二者已大相径庭， Scoop 仍然在其 README 中留着对 Homebrew 的 Credit。

一直以来 Scoop 都没有标称自己是一个包管理器，但是它的主要功能和主流包管理器的功能又是有几分相似，所以一些用户愿意称其为 Windows 下的包管理器。相应的，也有（比如偏好 MSYS2 的）用户批评过 Scoop 根本没有提供包管理器「标配」的软件构建环境及 build from source 的能力，到底只不过是一个「软件安装器」。我作为使用 Scoop 已经有 8、9 年，于 2015、2018 两年先后为其写过两篇「荐」的用户，以为 Scoop 的这种形式更多是 Windows 软件分发现状下的一个些许无奈的折中选择，只能说是有人喜有人愁。强如微软官方出品 Windows Package Manager —— Winget 也还是没有走出这个窠臼。AppX/MSIX 的打包分发方式不是所有应用开发者都接受并统一采用，曾几何时我从 UWP 上看到的一点点软件分发整合的希望也在其放宽 Win32 API 使用后慢慢消去（虽说就开发者角度来说这是好事）。所以 Scoop 在某些场景下仍有它的一席之地，放到末端用户就是各取所需。

---

## 这是什么

接下来谈论的是本文的主角 [Hok](https://github.com/chawyehsu/hok)。它是一个用 Rust 写的 Scoop 再实现，提供与 Scoop 类似的 CLI 接口，目标是实现 Scoop 已有的功能，如应用的检索、安装卸载、列表与状态、桶管理等等。实际上 Hok 只是一个 CLI 前端，其背后的 [libscoop](https://crates.io/crates/libscoop) 才是这个再实现的核心。后文会再提到 libscoop。

Scoop 用户在饱受着 `scoop search` 极度缓慢的折磨，带防火墙 debuff 的用户更是如此。早期的 Scoop 还不支持远程搜索，那时候搜索速度其实是很可以的。为了解决搜索体验问题，社区里出现了各种各样的解决方案，比如 Go 实现的 scoop-search 是社区里很流行的一个，也有其它如 Python + SQLite 的实现，以及依托 Azure Search 实现的 ScoopSearch（后来成为了 Scoop 官网上提供的在线搜索）等等。

然而社区里这些方案，都不约而同地只专注于解决搜索问题，并没有在其它方面做更多的尝试。也许是搜索问题太过于「碍眼」，导致其它如 `scoop list`、`scoop status` 及 `scoop update` 等命令的效率问题不入法眼。而 Hok 便是在解决了搜索问题的基础上，更进一步去尝试解决这些也让我难受的点，最终成为一个奔着完整实现而进行的项目。

---

## 开始尝试

假设你已经有在使用的 Scoop 环境，那么你可以使用一下命令来安装体验 Hok：

```powershell
scoop bucket add dorado https://github.com/chawyehsu/dorado
scoop install dorado/hok
```

dorado 是我维护了许久的一个桶，里面有不少我在用的或者接收 Pull Request 进来的软件。 Hok 是 native 的，可以脱离 Scoop 运行，所以其实也可以从 GitHub Release 页面下载二进制文件直接使用。但是因为 Scoop 的一些功能在 Hok 上还未被实现，所以单独用 Hok 的话会缺失关键功能，建议还是附在 Scoop 上。

安装完成后，可以使用 `hok help` 先查看 Hok 的帮助信息，了解下 Hok 的命令行接口：

```text
$ hok help
Hok is a CLI implementation of Scoop in Rust

Usage: hok.exe <COMMAND>

Commands:
  bucket     Manage manifest buckets
  cache      Package cache management
  cat        Inspect the manifest of a package
  cleanup    Cleanup apps by removing old versions
  config     Configuration management
  hold       Hold package(s) to disable changes
  home       Browse the homepage of a package
  info       Show package(s) basic information
  install    Install package(s)
  list       List installed package(s)
  search     Search available package(s)
  unhold     Unhold package(s) to enable changes
  uninstall  Uninstall package(s)
  update     Fetch and update subscribed buckets
  upgrade    Upgrade installed package(s)
  help       Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

Type 'hok help <command>' to get help for a specific command.
```

为了照顾一些个使用习惯，Hok 的命令行接口被设计得与 Scoop 的接口有一定的相似性，比如 `hok bucket`、`hok home`、`hok hold/unhold` 等。但是由于 Hok 背后的 libscoop 在内部实现上与 Scoop 不尽相同（主要是多了一些我个人的思考及针对我个人使用习惯的调整），所以体现到 Hok 的命令行接口上也有了一些变化。接下来我会逐一对目前 Hok 提供的各个命令做具体的介绍。这部分也许该写成使用文档放到项目里，但目前就先放在这吧。

---

### hok bucket

桶管理命令。这个子命令与 Scoop 的 `scoop bucket` 子命令几乎一致，唯一的一个不同是 Scoop 使用 `scoop bucket known` 来列出内置的（官方）桶，而 Hok 使用 `hok bucket list -k|--known` 来做同样的事情。

### hok cache

下载缓存管理，对应 `scoop cache`。Scoop 使用 `scoop cache [show]` 列出下载缓存列表， Hok 使用 `hok cache list [query]`，其中 `query` 可以作为过滤条件，只列出包含 `query` 的缓存。Scoop 使用 `scoop cache rm <app|*>` 来删除缓存，类似的 Hok 使用 `hok cache remove <query>` 来删除缓存，其中 `query` 同样支持通配符 `*` 删除全部缓存。

### hok cat

全文输出包的 manifest 详情。`hok cat` 与 `scoop cat` 的用法完全一致，但是 `hok cat` 多了对同名包的选择功能，如果你本地的不同桶之间存在同名的包的话，`scoop cat <app>` 会自动选择第一个找到的包，而 `hok cat <app>` 则会列出所有同名包，让你自己选择。同名包的选择功能在 `hok home` 等其它命令中也同样存在。

### hok cleanup

清理命令，对应 `scoop cleanup`。Hok 暂未实现 cleanup 功能，所以在这里它只是个 placeholder。

### hok config

配置管理命令。Scoop 使用 `scoop config` 展示配置，Hok 使用 `hok config list` 以 JSON 格式展示。Scoop 使用 `scoop config <key> <value>` 设置配置，Hok 使用 `hok config set <key> <value>` 来设置配置，Scoop 允许给任意 `key` 设置值，而 Hok 会检查 `key` 是否合法。Scoop 使用 `scoop config rm <key>` 删除配置，Hok 使用 `hok config unset <key>` 来删除配置。Hok 支持使用 `hok config edit` 命令调用外部编辑器来编辑配置文件。Hok 完全继承并兼容 Scoop 的配置文件。

### hok hold/unhold

包锁定/解锁命令，对应 `scoop hold/unhold`。Hok 的 `hold` 和 `unhold` 命令均与 Scoop 对应命令的用法一致。

### hok home

打开包主页的命令。Hok 的 `home` 命令与 Scoop 对应命令的用法一致，但是 `hok home` 多了对同名包的选择功能。

### hok info

包信息命令，对应 `scoop info`。Hok 的 `info` 命令与 Scoop 对应命令有所不同， `scoop info <app>` 只会精确匹配包名，而 `hok info <query>` 中的 `query` 是一个正则表达式入参。

### hok install

包安装命令。在对包的安装、更新、卸载操作上，Hok 有着一套相对于 Scoop 改动比较大的设计。依托于这部分设计改动， Hok 得以实现对同名包的选择与替换、锁定包强制更新等功能。但是截至目前 libscoop 在这部分功能的实现上还不完整，所以 Hok 的 `install` 命令也是不完整的。

### hok list

已安装包的列表命令。Hok 的 `list` 命令与 Scoop 对应命令的基础用法一致，同时提供复杂的筛选功能。Hok 将原本属于 `scoop status` 的查看可更新包的功能移到了 `hok list` 中，使用 `hok list --upgradable` 可以查看可更新包的列表。

### hok search

包搜索命令。搜索功能可以说是整个 Hok 项目的核心。Hok 的 `search` 命令与 Scoop 对应命令的不同点在于， `scoop search` 会远程搜索那些没有挂载到本地的官方桶，`hok search` 只会搜索本地桶。另外，`scoop search` 强制对包名、包描述以及 shim 均进行匹配，而 `hok search` 默认只对包名进行匹配，提供 `--with-description` 和 `--with-binary` 选项来开关对 shim 和包描述的匹配。这些调整为的都是最大化搜索效率。

### hok uninstall

包卸载命令。`hok uninstall` 执行时默认会检查待卸载包的依赖关系，如果有其他包依赖待卸载包，则会终止卸载操作，避免破坏依赖。

### hok update

桶更新命令。Hok 的这个命令只保留了 `scoop update` 里更新所有订阅到本地的桶的功能。

### hok upgrade

包更新命令。与 Scoop 的 `scoop update` 更新包相对应的功能，在 Hok 中被移到了 `hok upgrade` 这里。

以上就是 Hok 目前阶段提供的所有命令的介绍。

---

## 为什么做

一开始完全是因为 `scoop search` 的搜索效率问题，继而想去解决更多我个人使用 Scoop 时遇到的痛点。Hok 这个项目其实是「老坑新开」的，有心的可以去仓库翻 git log 了解一下，早在两年前我就已经下锄头挖坑了，只是中途断了一段时间，最近才又开始填坑。

### 为什么不直接贡献 Scoop

我作为 Scoop 的 maintainer team 成员之一，不去贡献 Scoop 而是自己写一个新的东西，这好像是有点说不过去。但是 Scoop 作为一个近 10 年的项目，拥有我觉得还挺庞大的用户量，我是不太能按个人想法去改动一些东西的，会很容易影响到其他用户。按自己的需求来做个新的相对容易推动些，另外这也算是个试验田，也许能在后续给 Scoop 带来一些反向作用力，进而反哺回去。

### 为什么是 Rust

理由其实很简单，我单纯想学 Rust，所以就用 Rust 来开坑了。这几年是切身感受到了 `ripgrep`、`bat`、`hyperfine` 以及 `starship` 等项目给我带来的愉悦，也许这些项目背后的 Rust 语言不过是一个次要因素。

---

## libscoop 的 roadmap

Hok 项目的核心是其中的 libscoop 库，之所以抽出 libscoop 这层抽象，当然是为了后续 hok-gui 甚至 hok-tui 的可能，不过这都是待定的事情。目前短中期的路线图主要是完成 `(un)install`/`upgrade` 等核心命令的实现。

## 结语

Hok 毕竟是一个实验性质的个人项目，但我想把它分享出来，收获一些正向反馈也好负向反馈也罢，至少有些作用力。

如果你对 Hok 这个项目感兴趣，或者有什么想法，欢迎借由各种途径留言。

回见。

---

*原文发表于 2023-08-10，作者 Chawye Hsu*
*此文档收录于 hok 项目以供参考*

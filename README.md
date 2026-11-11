# MarkText Rust 引擎

MarkText（`packages/muya`）核心编辑引擎的 Rust 移植。目标不是「再写一个 Markdown 库」，
而是**逐条复刻原版的解析与序列化契约**，让 MarkText 桌面端可以直接换掉 Electron/JS 内核。

原版参考仓库：`https://github.com/marktext/marktext`（只读克隆在本机 `D:/workbuddy/marktext-src`）。

---

## 零、下载与安装

预编译的 release 产物发布在 GitHub Releases：
<https://github.com/yaoyongli1974-dotcom/marktext-rust/releases>

| 平台 | 文件 | 内容 |
| :--- | :--- | :--- |
| Linux x86_64（glibc） | `marktext-x86_64-unknown-linux-gnu.tar.gz` | `marktext`（命令行）+ `marktext-app`（桌面应用） |
| Linux x86_64（静态） | `marktext-x86_64-unknown-linux-musl.tar.gz` | `marktext` 静态链接，无系统依赖 |
| Windows x86_64 | `marktext-x86_64-pc-windows-msvc.zip` | `marktext.exe`（命令行）+ `marktext-app.exe`（桌面应用） |

```bash
# 校验
sha256sum -c SHA256SUMS.txt

# 命令行
./marktext html README.md
./marktext roundtrip README.md
./marktext toc README.md --text
./marktext search README.md --query 强调

# 桌面应用（带参数即打开该文件）
./marktext-app README.md
```

Windows 上 `marktext-app.exe` 是 GUI 程序，**不带控制台窗口**；启动失败时会弹对话框提示原因。

自己构建：

```bash
cargo build --release -p marktext-cli -p marktext-app
```

> Windows 上用 `x86_64-pc-windows-gnu` 工具链时需要 `dlltool-proxy`（仓库自带，见
> `scripts/build.ps1`）；用 MSVC 工具链则不需要。

---

## 一、合规状态

| 规范套件 | 用例数 | 通过 | 失败 | 说明 |
| :--- | ---: | ---: | ---: | :--- |
| CommonMark 0.31.2 | 652 | **652** | 0 | 全量通过 |
| GFM 0.29（`cmark-gfm`） | 672 | 661 | 9 | 全部为强调算法的版本冲突，见下 |

对照：上一轮在同一台机器上实测，原版 `packages/muya` 跑同一套用例是
**CommonMark 59 失败 / GFM 72 失败**。

### 那 9 个 GFM 失败为什么不可修复

GFM 0.29 基于 **CommonMark 0.29**，用的是旧的强调算法（规则 13/14/15）；CommonMark 0.31.2
与 muya 用的是**分隔符栈 + 规则三（rule of three）**算法。两套算法在下列输入上期望不同结果，
**不可能同时满足**：

| 用例 | 输入 | GFM 0.29 期望 | CommonMark 0.31.2 / 本实现 |
| :--- | :--- | :--- | :--- |
| #389 | `__foo, __bar__, baz__` | `<strong>foo, <strong>bar</strong>, baz</strong>` | 同左（一致） |
| #464 | `****foo****` | `<strong>foo</strong>` | `<strong><strong>foo</strong></strong>` |
| #466 | `******foo******` | `<em><strong>foo</strong></em>` | `<strong><strong><strong>foo</strong></strong></strong>` |
| #468 | `_____foo_____` | `<em><strong>foo</strong></em>` | `<em><strong><strong>foo</strong></strong></em>` |

本实现选择跟随 **CommonMark 0.31.2 / muya**（即上游 MarkText 的实际行为）。
这 9 条被写进 `tests/fixtures/expected-failures.json` 的**回归锁**，作为已知的、
且不得被「悄悄修好」的差异。

---

## 二、工程结构

```
crates/
  marktext-core/     引擎本体（无 GUI 依赖，可单独当库用）
    src/parser/      块级解析器：行 → 块树
    src/inline.rs    行内解析器：强调/链接/代码跨度/HTML/自动链接
    src/render.rs    块树 → HTML
    src/serializer.rs 块树 → Markdown
    src/document.rs  块树的数据模型（TState 对应物）
    src/toc.rs       大纲
    src/search.rs    全文搜索
    src/unicode.rs   Unicode 标点/空白/CJK 表（生成物）
  marktext-app/      egui/eframe 桌面应用
  marktext-cli/      命令行工具（parse / html / toc / roundtrip / search）
tools/
  dlltool-proxy/     Windows 下定位 GNU dlltool 的构建辅助
```

### 三条不可动摇的设计约定

1. **文档 = 块树，叶子块保存原始行内 Markdown 文本。**
   行内语法是**惰性解析**的：只有在渲染或搜索时才解析成 token。这正是原版 muya
   「无损往返」能够成立的根本原因，也是本移植必须 1:1 复刻的地方。

2. **往返契约是「收敛性」而非「恒等」。**
   `roundTrip(roundTrip(x)) == roundTrip(x)`，并且往返前后**块结构不变**。
   两条断言都在 `crates/marktext-core/tests/roundtrip.rs`。
   序列化器允许把「多写的缩进」规范化掉，但不允许丢块或改结构。

3. **规范用例用回归锁管理。**
   `tests/fixtures/expected-failures.json` 列出当前**必须继续失败**的用例；
   其中任何一条开始通过，测试就会失败并要求把条目删掉 —— 合规率只许升不许降。

---

## 三、移植过程中啃下来的硬骨头

以下几处不是「照抄规格」就能过的，需要对齐参考实现的**具体算法**：

### 1. 强调解析：分隔符栈 + 规则三

规格的「规则 13/14/15」无法直接实现，本移植改用 cmark 的**分隔符栈**算法：

- 可配对判定：`!(closer.can_open || opener.can_close) || closer.len % 3 == 0 || (opener.len + closer.len) % 3 != 0`
  —— 与 muya 的 `violatesRuleOfThree` 完全等价。
- 匹配时 opener **从右端消耗**（保留前缀），closer 的剩余量语义与 cmark 一致。
- 实现了 `insert_emphasis` / `remove_delimiter` / token 下标平移。

### 2. Unicode 标点定义：0.31.2 与 GFM 0.29 冲突

CommonMark 0.31.2 把「Unicode 标点字符」定义为 **P 类或 S 类**（符号也算），
GFM 0.29 只算 P 类。muya 的 `UNICODE_PUNCTUATION = /[\p{P}\p{S}]/u` 跟随 0.31.2。
本移植用 `scripts/generate-unicode-tables.py` 生成区间表（不引第三方 crate），
并额外支持 muya 的 **CJK 扩展**（#4307）：CJK 字符作为 flanking 边界。

### 3. 列表松紧（tight/loose）判定

cmark 的 `finalize` 里有两处判定，且 `last_line_blank` 的**设置层级**极易出错：

```
if (S_last_line_blank(item) && item->next) → loose
for subitem in item.children:
    if ((item->next || subitem->next) && S_ends_with_blank_line(subitem)) → loose
```

`last_line_blank` 必须落在**最内层仍打开的块**上（段落的父级 = item），
并清掉所有祖先的标志。围栏代码块内部的空行不计入。

### 4. 制表符的列语义（本轮最后攻下的 Tabs 用例）

制表符宽度取决于它在文档中的**绝对列**，而不是「重基到第 0 列后的相对位置」。
cmark 用 `parser->column` 跟踪虚拟列，并在跨越边界时**部分消耗**制表符：

- `S_find_first_nonspace`：从当前列算制表符宽度。
- `S_advance_offset(count, columns=true)`：若制表符跨越边界，置
  `partially_consumed_tab`，剩余列在 `add_line` 里**物化成空格**。
- `parse_block_quote_prefix`：`>` 之后**只消耗 1 列**空白；后面若是制表符，
  只用掉 1 列，其余列留给内容当缩进。

本移植为此引入了**逐行的绝对列基址**（`Line.base` / `FragmentLine.base`），
`tab_to_columns` / `strip_columns` 全部改为按绝对列计算。
例：`>\t\tfoo` → 引用块内的缩进代码块，内容为 `"  foo"`（2 空格 + foo）。

### 5. GFM 表格

- 表头与分隔行的**列数必须一致**，否则不成表。
- 正文行可少（补空）/ 可多（截断）。
- 无正文行时**不生成 `<tbody>`**。
- `\|` 解析为字面 `|`（muya #4849），序列化时由 `escape_table_cell` 重新加回。

### 6. GFM tagfilter

黑名单：`title, textarea, style, xmp, iframe, noembed, noframes, script, plaintext`。
匹配规则同 cmark-gfm 的 `is_tag()`：`<` 或 `</` + 名字（大小写不敏感）+ 边界
（空白 / `>` / `/>`），**只把 `<` 换成 `&lt;`**。
HTML 块内部是**逐个 `<` 扫描**的 —— 块里的 `<blockquote>` 不受影响，
块里的 `<xmp>` 会被过滤。

---

## 四、怎么跑

```bash
# 全部测试（规范 + 往返 + 单元）
cargo test --workspace

# 规范套件（含回归锁校验）
cargo test -p marktext-core --test spec

# 看某个章节的失败详情
MARKTEXT_SPEC_DUMP=1 MARKTEXT_SPEC_SECTION=Tabs \
  cargo test -p marktext-core --test spec commonmark_0_31_2_conformance -- --exact

# 重建回归锁基线（两个用例要分别跑，避免互相覆盖）
MARKTEXT_UPDATE_EXPECTED_FAILURES=1 cargo test -p marktext-core --test spec commonmark_0_31_2_conformance -- --exact
MARKTEXT_UPDATE_EXPECTED_FAILURES=1 cargo test -p marktext-core --test spec gfm_0_29_conformance -- --exact

# 命令行工具
cargo run -p marktext-cli -- html  README.md
cargo run -p marktext-cli -- toc   README.md --text
cargo run -p marktext-cli -- roundtrip README.md
```

> **注意**：`cargo test --workspace` **不会**编译 `marktext-app`（它的 bin 设了
> `test = false`，因为 GUI 测试会拉起窗口）。改动 core 的公开 API 后，
> 必须额外跑 `cargo build --workspace --all-targets` 才能发现 app 侧的断裂。

---

## 五、当前已知限制

| 项 | 状态 |
| :--- | :--- |
| GFM 强调的 9 条版本冲突用例 | 不可修复，已登记在回归锁 |
| 正则搜索 | 未实现，退化为字面量搜索（`search_regex` 会降级） |
| 嵌套片段里的制表符 | 会被物化成空格，往返不再逐字节一致（收敛性仍成立） |
| 列表项内多余缩进的规范化 | 序列化器会重排缩进，往返不保证逐字节一致 |

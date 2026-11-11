# 致谢与出处

本仓库是 **MarkText**（<https://github.com/marktext/marktext>）核心编辑引擎
`packages/muya` 的 **Rust 移植**。

## 上游

- 项目：MarkText —— 一款简洁优雅的 Markdown 编辑器
- 仓库：<https://github.com/marktext/marktext>
- 许可证：MIT（见本仓库的 `LICENSE`）
- 版权所有者：Luo Ran、MarkText Contributors

## 本仓库做了什么

把上游的 **TypeScript / Vue 编辑引擎**用 Rust 重新实现，目标是**逐条复刻原版的
行为契约**，而不是另写一个 Markdown 库：

| 上游（TypeScript） | 本仓库（Rust） |
| :--- | :--- |
| `packages/muya/src/lib/parser/` 块级解析 | `crates/marktext-core/src/parser/` |
| `packages/muya/src/lib/parser/render/` 行内解析 | `crates/marktext-core/src/inline.rs` |
| `packages/muya/src/lib/state/` 状态树 | `crates/marktext-core/src/document.rs` |
| `packages/muya/src/lib/state/markdownToHtml.ts` | `crates/marktext-core/src/render.rs` |
| `packages/muya/src/lib/parser/commonMarkToMarkdown.ts` | `crates/marktext-core/src/serializer.rs` |

保留的三条核心契约（详见 `README.md`）：

1. 文档是块树，叶子块保存**原始行内 Markdown 文本**，行内语法惰性解析；
2. 往返契约是**收敛性**而非恒等，且往返前后块结构不变；
3. 规范用例由**回归锁**管理，合规率只许升不许降。

## 合规率

- CommonMark 0.31.2：**652 / 652**
- GFM 0.29：661 / 672（剩余 9 条为两个规范版本间强调算法的固有冲突）

上游 `packages/muya` 跑同一套用例为 CommonMark 59 失败 / GFM 72 失败。

## 第三方参考

实现过程中对照了以下开源项目的**行为契约**（未复制其代码）：

- [CommonMark](https://github.com/commonmark/cmark) 0.31.2 —— 块级解析、
  强调算法、列表松紧判定、制表符列语义
- [cmark-gfm](https://github.com/github/cmark-gfm) —— GFM 表格与 tagfilter 扩展

//! Command line interface for the MarkText Rust engine.
//!
//! The CLI exposes the engine the same way the desktop app uses it: parse a
//! Markdown file into the state tree, render it to HTML, list its outline,
//! verify that a document survives a lossless round trip, or search it.

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use marktext_core::{
    build_toc, parse_document, render_html, render_to_html, search_document, serialize_document,
    ExportOptions, RenderOptions, SearchOptions,
};

const USAGE: &str = "\
marktext —— MarkText Rust 引擎命令行工具

用法：
    marktext <命令> [参数]

命令：
    parse <文件>              解析 Markdown，输出状态树 JSON
    html <文件> [--full]      渲染为 HTML（--full 输出完整 HTML 文档）
    toc <文件> [--text]       输出大纲（默认 JSON，--text 输出可读大纲）
    roundtrip <文件>          校验「解析 → 序列化」与原文是否逐字节一致
    search <文件> <查询>      全文搜索，输出命中位置 JSON
    help                      显示本帮助
    --version                 显示版本号

搜索选项：
    --query <查询>            查询内容（也可写作第二个位置参数）
    --regex                   查询按正则表达式解释（RE2 语法）
    --case-sensitive          区分大小写
    --context <n>             每条命中附带上下文字符数
    --                        其后的参数一律按位置参数处理

示例：
    marktext parse README.md
    marktext html README.md --full > README.html
    marktext roundtrip README.md
    marktext search README.md --query 强调 --context 12
    marktext search README.md --regex '^#{1,6} '
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("marktext: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Dispatch a command line.
fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first() else {
        println!("{USAGE}");
        return Ok(());
    };

    match command.as_str() {
        "parse" => cmd_parse(&args[1..]),
        "html" => cmd_html(&args[1..]),
        "toc" => cmd_toc(&args[1..]),
        "roundtrip" => cmd_roundtrip(&args[1..]),
        "search" => cmd_search(&args[1..]),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        "--version" | "-V" => {
            println!("marktext {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(format!("未知命令 `{other}`\n\n{USAGE}")),
    }
}

/// Read a file as UTF-8 text.
fn read_source(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    fs::read_to_string(path).map_err(|err| format!("无法读取 `{}`：{err}", path.display()))
}

/// Take the single positional path out of an argument list.
fn one_path(args: &[String]) -> Result<String, String> {
    let mut path = None;
    for arg in args {
        if arg.starts_with("--") {
            continue;
        }
        if path.is_some() {
            return Err("参数过多，请只指定一个文件".to_string());
        }
        path = Some(arg.clone());
    }
    path.ok_or_else(|| "缺少文件路径".to_string())
}

/// `parse` — dump the state tree as JSON.
fn cmd_parse(args: &[String]) -> Result<(), String> {
    let path = one_path(args)?;
    let source = read_source(&path)?;
    let document = parse_document(&source).map_err(|err| format!("解析失败：{err}"))?;
    let json =
        serde_json::to_string_pretty(&document).map_err(|err| format!("无法生成 JSON：{err}"))?;
    println!("{json}");
    Ok(())
}

/// `html` — render the document.
fn cmd_html(args: &[String]) -> Result<(), String> {
    let mut full = false;
    let mut allow_raw_html = true;
    let mut path = None;

    for arg in args {
        match arg.as_str() {
            "--full" => full = true,
            "--no-raw-html" => allow_raw_html = false,
            other if other.starts_with("--") => {
                return Err(format!("未知选项 `{other}`"));
            }
            other => {
                if path.is_some() {
                    return Err("参数过多，请只指定一个文件".to_string());
                }
                path = Some(other.to_string());
            }
        }
    }

    let path = path.ok_or_else(|| "缺少文件路径".to_string())?;
    let source = read_source(&path)?;
    let document = parse_document(&source).map_err(|err| format!("解析失败：{err}"))?;

    let mut options = RenderOptions {
        allow_raw_html,
        ..RenderOptions::default()
    };
    options.references = document.references.clone();
    let html = if full {
        render_to_html(&document.blocks, options)
    } else {
        render_html(&document.blocks, options)
    };
    print!("{html}");
    Ok(())
}

/// `toc` — list the outline of the document.
fn cmd_toc(args: &[String]) -> Result<(), String> {
    let mut as_text = false;
    let mut path = None;

    for arg in args {
        match arg.as_str() {
            "--text" => as_text = true,
            other if other.starts_with("--") => {
                return Err(format!("未知选项 `{other}`"));
            }
            other => {
                if path.is_some() {
                    return Err("参数过多，请只指定一个文件".to_string());
                }
                path = Some(other.to_string());
            }
        }
    }

    let path = path.ok_or_else(|| "缺少文件路径".to_string())?;
    let source = read_source(&path)?;
    let document = parse_document(&source).map_err(|err| format!("解析失败：{err}"))?;
    let items = build_toc(&document.blocks, "");

    if as_text {
        for item in &items {
            let indent = "  ".repeat(item.level.saturating_sub(1) as usize);
            println!("{indent}- {} (#{})", item.content, item.slug);
        }
        return Ok(());
    }

    let json: Vec<_> = items
        .iter()
        .map(|item| {
            serde_json::json!({
                "level": item.level,
                "content": item.content,
                "content_html": item.content_html,
                "slug": item.slug,
                "github_slug": item.github_slug,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json).map_err(|err| format!("无法生成 JSON：{err}"))?
    );
    Ok(())
}

/// `roundtrip` — check that a document survives the engine unchanged.
fn cmd_roundtrip(args: &[String]) -> Result<(), String> {
    let mut output_path: Option<String> = None;
    let mut path = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--out 缺少文件路径".to_string())?;
                output_path = Some(value.clone());
            }
            other if other.starts_with("--") => {
                return Err(format!("未知选项 `{other}`"));
            }
            other => {
                if path.is_some() {
                    return Err("参数过多，请只指定一个文件".to_string());
                }
                path = Some(other.to_string());
            }
        }
        index += 1;
    }

    let path = path.ok_or_else(|| "缺少文件路径".to_string())?;
    let source = read_source(&path)?;
    let document = parse_document(&source).map_err(|err| format!("解析失败：{err}"))?;
    let output = serialize_document(&document, ExportOptions::default());

    if let Some(target) = output_path {
        fs::write(&target, output.as_bytes())
            .map_err(|err| format!("无法写入 `{}`：{err}", target))?;
        println!("重写结果已写入 {target}");
    }

    if output == source {
        println!("往返一致：{path}（{} 字节）", source.len());
        return Ok(());
    }

    println!("往返不一致：{path}");
    println!("  原文 {} 字节，重写后 {} 字节", source.len(), output.len());
    for difference in describe_differences(&source, &output) {
        println!("  {difference}");
    }
    Err("往返校验失败".to_string())
}

/// Describe the first few line-level differences between two documents.
fn describe_differences(source: &str, output: &str) -> Vec<String> {
    let source_lines: Vec<&str> = source.split('\n').collect();
    let output_lines: Vec<&str> = output.split('\n').collect();
    let mut out = Vec::new();

    for index in 0..source_lines.len().max(output_lines.len()) {
        let left = source_lines.get(index).copied().unwrap_or("<缺失>");
        let right = output_lines.get(index).copied().unwrap_or("<缺失>");
        if left != right {
            out.push(format!("第 {} 行", index + 1));
            out.push(format!("    原文：{left}"));
            out.push(format!("    重写：{right}"));
        }
        if out.len() >= 15 {
            out.push("    …… 其余差异省略".to_string());
            break;
        }
    }
    out
}

/// `search` — find every occurrence of a query.
fn cmd_search(args: &[String]) -> Result<(), String> {
    let mut regex = false;
    let mut case_insensitive = true;
    let mut context = 0usize;
    let mut query_option: Option<String> = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut options_ended = false;

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if options_ended {
            positional.push(arg);
            index += 1;
            continue;
        }
        match arg {
            "--" => options_ended = true,
            "--regex" => regex = true,
            "--case-sensitive" => case_insensitive = false,
            "--query" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--query 缺少查询内容".to_string())?;
                query_option = Some(value.clone());
            }
            other if other.starts_with("--query=") => {
                query_option = Some(other["--query=".len()..].to_string());
            }
            "--context" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--context 缺少数值".to_string())?;
                context = value
                    .parse()
                    .map_err(|_| format!("--context 需要一个非负整数，收到 `{value}`"))?;
            }
            other if other.starts_with("--") => {
                return Err(format!("未知选项 `{other}`"));
            }
            other => positional.push(other),
        }
        index += 1;
    }

    // The query can be given as the second positional argument, the way the
    // usage line shows, or with `--query` so that a query beginning with a
    // dash does not have to be escaped.
    let (path, query) = match positional.as_slice() {
        [path, query] if query_option.is_none() => (*path, (*query).to_string()),
        [path] => (
            *path,
            query_option.ok_or_else(|| "用法：marktext search <文件> <查询>".to_string())?,
        ),
        [] => return Err("用法：marktext search <文件> <查询>".to_string()),
        _ => return Err("参数过多，请只指定一个文件和一条查询".to_string()),
    };

    let source = read_source(path)?;
    let options = SearchOptions {
        case_insensitive,
        regex,
        context,
    };
    let matches = search_document(&source, &query, options).map_err(|err| err.to_string())?;

    let json: Vec<_> = matches
        .iter()
        .map(|item| {
            serde_json::json!({
                "start": item.start,
                "end": item.end,
                "text": item.text,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&json).map_err(|err| format!("无法生成 JSON：{err}"))?
    );
    Ok(())
}

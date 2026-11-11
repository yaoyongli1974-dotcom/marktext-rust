//! `dlltool` 代理 —— 让 `x86_64-pc-windows-gnu` 上的 `raw-dylib` 链接可用。
//!
//! # 背景
//!
//! `windows-link` 0.1/0.2 都无条件地使用 `#[link(kind = "raw-dylib")]`。在
//! `x86_64-pc-windows-gnu` 上，rustc 必须调用 GNU `dlltool` 依据生成的 `.def`
//! 文件产出导入库（`kernel32.dll_imports.lib` 等），而 GNU `dlltool` 在汇编
//! 生成的 `.s` 文件时依赖 GNU 汇编器 `as.exe`。
//!
//! rustup 分发的 `rust-mingw` 组件在 Windows 上只带
//! `dlltool.exe / ld.exe / x86_64-w64-mingw32-gcc.exe`，**不含 `as.exe`**，
//! 于是 `windows-sys`、`windows-result`、`parking_lot_core` 等依赖
//! `raw-dylib` 的 crate 全部编译失败：
//!
//! ```text
//! error: dlltool could not create import library with ...dlltool.exe ...
//! ...dlltool.exe: CreateProcess error: ... as.exe
//! ```
//!
//! # 方案
//!
//! 补上一份 GNU 汇编器即可。MSYS2 的 `mingw-w64-x86_64-binutils` 提供了
//! 与 `dlltool` 配套的 `as.exe`（见 `scripts/install-mingw-binutils.ps1`）。
//! 本代理负责：
//!
//! 1. 找到可用的 `dlltool.exe` —— 优先使用工具链旁安装的 mingw-w64 版本
//!    （它与 `as.exe` 版本一致），否则退回 Rust 工具链自带的 `dlltool.exe`；
//! 2. 把 `as.exe` 所在目录加入 `PATH`，让 `dlltool` 一定能找到汇编器；
//! 3. 原样转发 rustc 传入的全部参数。
//!
//! 由真正的 GNU `dlltool` 生成的导入库是 GNU ld 期望的「短导入」格式，
//! 链接与运行都完全正确，且不挑 DLL —— `api-ms-win-*` 这类 API Set 也能处理。
//!
//! # 为什么不用 lld-link
//!
//! rustup 同时分发 `rust-lld.exe`（内含 `lld-link`），它能用
//! `lld-link /lib /def:<file> /out:<lib>` 由 `.def` 生成导入库。生成的库是
//! **MSVC 风格（长导入表）**：GNU ld 能解析其中的符号，但不会为其生成正确的
//! IAT，程序运行时会以 `STATUS_ACCESS_VIOLATION` 崩溃（实测 `user32.dll`
//! 导入的程序启动即崩）。因此本代理不再自行生成导入库。

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("dlltool-proxy: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[OsString]) -> Result<ExitCode, String> {
    let (dlltool, tool_dir) = find_dlltool()?;
    let args = rewrite_args(args);

    if env::var_os("DLLTOOL_PROXY_DUMP").is_some() {
        eprintln!("dlltool-proxy: {} {:?}", dlltool.display(), args);
    }

    // 让 dlltool 一定能找到 GNU 汇编器。
    let mut path = env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = env::split_paths(&path).collect();
    if !dirs.iter().any(|existing| existing == &tool_dir) {
        dirs.push(tool_dir.clone());
    }
    path = env::join_paths(dirs).map_err(|err| format!("cannot rebuild PATH: {err}"))?;

    let status = Command::new(&dlltool)
        .args(args)
        .env("PATH", path)
        .status()
        .map_err(|err| format!("cannot spawn {}: {err}", dlltool.display()))?;

    // 透传退出码：rustc 会据此判断导入库是否生成成功。
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// rustc 传给 dlltool 的 DLL 名带有**重复的** `.dll` 后缀。
///
/// `windows-link` 的宏写的是 `#[link(name = "user32.dll", kind = "raw-dylib")]`，
/// 而 rustc 在生成导入库时又给它补了一次扩展名，于是 dlltool 收到的是
/// `user32.dll.dll`。dlltool 会把 `-D` 的值原样写进导入库的 DLL 名，Windows
/// 加载器因此找不到 `user32.dll.dll`，不会填充 IAT —— 程序在第一次调用导入
/// 函数时就会以 `STATUS_ACCESS_VIOLATION` 崩溃。
///
/// 这里把名字还原成真正的 DLL 名。
fn normalize_dll_name(name: &str) -> String {
    if let Some(stripped) = name.strip_suffix(".dll.dll") {
        format!("{stripped}.dll")
    } else if let Some(stripped) = name.strip_suffix(".dll") {
        stripped.to_string()
    } else {
        name.to_string()
    }
}

/// 修正参数中所有与 DLL 名相关的取值。
fn rewrite_args(args: &[OsString]) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::with_capacity(args.len());
    let mut idx = 0usize;

    while idx < args.len() {
        let arg = args[idx].to_string_lossy().into_owned();
        match arg.as_str() {
            "-D" | "--dll-name" | "--dllname" | "--temp-prefix" => {
                out.push(OsString::from(arg));
                if let Some(value) = args.get(idx + 1) {
                    let text = value.to_string_lossy();
                    out.push(OsString::from(normalize_dll_name(&text)));
                    idx += 2;
                    continue;
                }
            }
            _ => {}
        }
        out.push(args[idx].clone());
        idx += 1;
    }

    out
}

/// 找到可用的 `dlltool.exe` 及其所在目录。
///
/// 顺序：
/// 1. `DLLTOOL_PROXY_DLLTOOL` 显式指定；
/// 2. 代理自身旁的 `../mingw-bin`（`scripts/install-mingw-binutils.ps1` 的安装位置）；
/// 3. `PATH` 中的 `dlltool.exe`；
/// 4. Rust 工具链自带的 `dlltool.exe`。
fn find_dlltool() -> Result<(PathBuf, PathBuf), String> {
    if let Some(explicit) = env::var_os("DLLTOOL_PROXY_DLLTOOL") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            return Ok((path, dir));
        }
        return Err(format!(
            "DLLTOOL_PROXY_DLLTOOL points to a missing file: {}",
            path.display()
        ));
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("..").join("mingw-bin").join("dlltool.exe");
            if candidate.is_file() {
                let candidate = candidate.canonicalize().unwrap_or(candidate);
                let dir = candidate.parent().unwrap_or(Path::new(".")).to_path_buf();
                return Ok((candidate, dir));
            }
        }
    }

    for dir in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        let candidate = dir.join("dlltool.exe");
        if candidate.is_file() {
            return Ok((candidate, dir));
        }
    }

    // 最后退回 Rust 工具链自带的 dlltool —— 只要 as.exe 在 PATH 里也能工作。
    if let Some(sysroot) = rustc_sysroot() {
        if let Some(host) = rustc_host_triple() {
            let bin_dir = Path::new(&sysroot)
                .join("lib")
                .join("rustlib")
                .join(&host)
                .join("bin")
                .join("self-contained");
            let candidate = bin_dir.join("dlltool.exe");
            if candidate.is_file() {
                return Ok((candidate, bin_dir));
            }
        }
    }

    Err("cannot locate dlltool.exe; run scripts/install-mingw-binutils.ps1 or set DLLTOOL_PROXY_DLLTOOL".into())
}

fn rustc_sysroot() -> Option<String> {
    let out = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn rustc_host_triple() -> Option<String> {
    let out = Command::new("rustc").arg("-vV").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|line| line.strip_prefix("host: ").map(|v| v.trim().to_string()))
}

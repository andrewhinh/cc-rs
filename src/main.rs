use std::{
    env, fs,
    io::{self, Write},
    path::Path,
    process,
};

use cc_rs::{
    Token, TokenKind, add_include_path, codegen::emit_assembly, get_input_files,
    preprocess::preprocess, tokenize::tokenize_file,
};
use tempfile::NamedTempFile;

struct Args {
    opt_cc1: bool,
    opt_s: bool,
    opt_c: bool,
    opt_e: bool,
    opt_hash_hash_hash: bool,
    opt_o: Option<String>,
    base_file: Option<String>,
    output_file: Option<String>,
    input_paths: Vec<String>,
    include_paths: Vec<String>,
}

fn usage(status: i32) {
    eprintln!("Usage: cc-rs [ -o <path> ] <file>");
    process::exit(status);
}

fn parse_args() -> Args {
    let args: Vec<String> = env::args().collect();
    let mut opt_cc1 = false;
    let mut opt_s = false;
    let mut opt_c = false;
    let mut opt_e = false;
    let mut opt_hash_hash_hash = false;
    let mut opt_o: Option<String> = None;
    let mut base_file: Option<String> = None;
    let mut output_file: Option<String> = None;
    let mut input_paths: Vec<String> = Vec::new();
    let mut include_paths: Vec<String> = Vec::new();
    let mut i = 1;

    while i < args.len() {
        if args[i] == "--help" {
            usage(0);
        }

        if args[i] == "-###" {
            opt_hash_hash_hash = true;
            i += 1;
            continue;
        }

        if args[i] == "-cc1" {
            opt_cc1 = true;
            i += 1;
            continue;
        }

        if args[i] == "-S" {
            opt_s = true;
            i += 1;
            continue;
        }

        if args[i] == "-c" {
            opt_c = true;
            i += 1;
            continue;
        }

        if args[i] == "-E" {
            opt_e = true;
            i += 1;
            continue;
        }

        if args[i] == "-o" {
            i += 1;
            if i >= args.len() {
                usage(1);
            }
            opt_o = Some(args[i].clone());
            i += 1;
            continue;
        }

        if args[i].starts_with("-o") {
            opt_o = Some(args[i][2..].to_string());
            i += 1;
            continue;
        }

        if args[i] == "-cc1-input" {
            i += 1;
            if i >= args.len() {
                usage(1);
            }
            base_file = Some(args[i].clone());
            i += 1;
            continue;
        }

        if args[i] == "-cc1-output" {
            i += 1;
            if i >= args.len() {
                usage(1);
            }
            output_file = Some(args[i].clone());
            i += 1;
            continue;
        }

        if args[i].starts_with("-I") {
            if args[i].len() > 2 {
                include_paths.push(args[i][2..].to_string());
                i += 1;
            } else {
                i += 1;
                if i >= args.len() {
                    usage(1);
                }
                include_paths.push(args[i].clone());
                i += 1;
            }
            continue;
        }

        if args[i].starts_with('-') && args[i].len() > 1 {
            eprintln!("unknown argument: {}", args[i]);
            process::exit(1);
        }

        input_paths.push(args[i].clone());
        i += 1;
    }

    if input_paths.is_empty() && base_file.is_none() {
        eprintln!("no input files");
        process::exit(1);
    }

    Args {
        opt_cc1,
        opt_s,
        opt_c,
        opt_e,
        opt_hash_hash_hash,
        opt_o,
        base_file,
        output_file,
        input_paths,
        include_paths,
    }
}

fn open_output_file(path: Option<&String>) -> Box<dyn Write> {
    if path.is_none() || path.unwrap().as_str() == "-" {
        return Box::new(io::stdout());
    }

    let file = fs::File::create(path.unwrap()).expect("cannot open output file");
    Box::new(file)
}

fn run_subprocess(opt_hash_hash_hash: bool, args: &[String]) -> Result<(), String> {
    if opt_hash_hash_hash {
        eprintln!("{}", args.join(" "));
    }

    let status = process::Command::new(&args[0])
        .args(&args[1..])
        .stdin(process::Stdio::inherit())
        .status()
        .map_err(|e| format!("exec failed: {}: {}", args[0], e))?;

    if !status.success() {
        process::exit(1);
    }
    Ok(())
}

fn run_cc1(
    opt_hash_hash_hash: bool,
    orig_args: &[String],
    input: Option<&str>,
    output: Option<&str>,
) -> Result<(), String> {
    let mut new_args = orig_args.to_vec();
    new_args.push("-cc1".to_string());

    if let Some(inp) = input {
        new_args.push("-cc1-input".to_string());
        new_args.push(inp.to_string());
    }

    if let Some(out) = output {
        new_args.push("-cc1-output".to_string());
        new_args.push(out.to_string());
    }

    run_subprocess(opt_hash_hash_hash, &new_args)
}

fn print_tokens(tok: &Token, opt_o: Option<&String>) -> Result<(), String> {
    let mut out = open_output_file(opt_o);
    let files = get_input_files();
    let mut tok = tok;

    while tok.kind != TokenKind::Eof {
        if tok.at_bol {
            writeln!(out).map_err(|e| format!("write error: {e}"))?;
        }
        if tok.has_space && !tok.at_bol {
            write!(out, " ").map_err(|e| format!("write error: {e}"))?;
        }

        if tok.kind == TokenKind::Str {
            let str_content = tok.str.as_ref().unwrap();
            write!(out, "\"").map_err(|e| format!("write error: {e}"))?;
            for &c in str_content.iter().take_while(|&&c| c != 0) {
                if c == b'"' || c == b'\\' {
                    write!(out, "\\").map_err(|e| format!("write error: {e}"))?;
                }
                write!(out, "{}", c as char).map_err(|e| format!("write error: {e}"))?;
            }
            write!(out, "\"").map_err(|e| format!("write error: {e}"))?;
        } else {
            let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
            let token_str: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();
            write!(out, "{}", token_str).map_err(|e| format!("write error: {e}"))?;
        }
        tok = tok.next.as_ref().unwrap();
    }
    writeln!(out).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn cc1(args: &Args) -> Result<(), String> {
    let input = args.base_file.as_ref().ok_or("no input file for cc1")?;
    unsafe {
        std::env::set_var("CC_RS_BASE_FILE", input);
    }

    for path in &args.include_paths {
        add_include_path(path.clone());
    }

    let tok = tokenize_file(input).ok_or("cannot open input file")?;
    let tok = preprocess(tok)?;

    if args.opt_e {
        return print_tokens(&tok, args.opt_o.as_ref());
    }

    let asm = emit_assembly()?;

    let out_path = args.output_file.as_ref();
    let mut out = open_output_file(out_path);
    out.write_all(asm.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn replace_extn(path: &str, extn: &str) -> String {
    let dot_pos = path.rfind('.');
    let base = match dot_pos {
        Some(pos) => &path[..pos],
        None => path,
    };
    format!("{}{}", base, extn)
}

fn create_tmpfile() -> Result<(NamedTempFile, String), String> {
    let tmpfile = NamedTempFile::new().map_err(|e| format!("mkstemp failed: {e}"))?;
    let path = tmpfile.path().to_string_lossy().into_owned();
    Ok((tmpfile, path))
}

fn assemble(input: &str, output: &str, opt_hash_hash_hash: bool) -> Result<(), String> {
    let args = vec![
        "as".to_string(),
        "-c".to_string(),
        input.to_string(),
        "-o".to_string(),
        output.to_string(),
    ];
    run_subprocess(opt_hash_hash_hash, &args)
}

fn endswith(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len() && &s[s.len() - suffix.len()..] == suffix
}

fn file_exists(path: &str) -> bool {
    Path::new(path).exists()
}

fn find_file(pattern: &str) -> Option<String> {
    let paths: Vec<_> = glob::glob(pattern).ok()?.filter_map(Result::ok).collect();
    paths.last().map(|p| p.to_string_lossy().into_owned())
}

fn find_libpath() -> Result<String, String> {
    if file_exists("/usr/lib/x86_64-linux-gnu/crti.o") {
        return Ok("/usr/lib/x86_64-linux-gnu".to_string());
    }
    if file_exists("/usr/lib64/crti.o") {
        return Ok("/usr/lib64".to_string());
    }
    Err("library path is not found".to_string())
}

fn find_gcc_libpath() -> Result<String, String> {
    let patterns = [
        "/usr/lib/gcc/x86_64-linux-gnu/*/crtbegin.o",
        "/usr/lib/gcc/x86_64-pc-linux-gnu/*/crtbegin.o",
        "/usr/lib/gcc/x86_64-redhat-linux/*/crtbegin.o",
    ];

    for pattern in &patterns {
        if let Some(path) = find_file(pattern)
            && let Some(parent) = Path::new(&path).parent()
        {
            return Ok(parent.to_string_lossy().into_owned());
        }
    }

    Err("gcc library path is not found".to_string())
}

fn run_linker(inputs: &[String], output: &str, opt_hash_hash_hash: bool) -> Result<(), String> {
    let libpath = find_libpath()?;
    let gcc_libpath = find_gcc_libpath()?;

    let mut args = vec![
        "ld".to_string(),
        "-o".to_string(),
        output.to_string(),
        "-m".to_string(),
        "elf_x86_64".to_string(),
        "-dynamic-linker".to_string(),
        "/lib64/ld-linux-x86-64.so.2".to_string(),
        format!("{}/crt1.o", libpath),
        format!("{}/crti.o", libpath),
        format!("{}/crtbegin.o", gcc_libpath),
        format!("-L{}", gcc_libpath),
        format!("-L{}", libpath),
        format!("-L{}/..", libpath),
        "-L/usr/lib64".to_string(),
        "-L/lib64".to_string(),
        "-L/usr/lib/x86_64-linux-gnu".to_string(),
        "-L/usr/lib/x86_64-pc-linux-gnu".to_string(),
        "-L/usr/lib/x86_64-redhat-linux".to_string(),
        "-L/usr/lib".to_string(),
        "-L/lib".to_string(),
    ];

    for input in inputs {
        args.push(input.clone());
    }

    args.push("-lc".to_string());
    args.push("-lgcc".to_string());
    args.push("--as-needed".to_string());
    args.push("-lgcc_s".to_string());
    args.push("--no-as-needed".to_string());
    args.push(format!("{}/crtend.o", gcc_libpath));
    args.push(format!("{}/crtn.o", libpath));

    run_subprocess(opt_hash_hash_hash, &args)
}

fn run() -> Result<(), String> {
    let args = parse_args();

    if args.opt_cc1 {
        return cc1(&args);
    }

    let orig_args: Vec<String> = env::args().collect();

    if args.input_paths.len() > 1
        && args.opt_o.is_some()
        && (args.opt_c || args.opt_s || args.opt_e)
    {
        return Err("cannot specify '-o' with '-c', '-S' or '-E' with multiple files".to_string());
    }

    let mut ld_args: Vec<String> = Vec::new();
    let mut _tmpfiles: Vec<NamedTempFile> = Vec::new();

    for input in &args.input_paths {
        if endswith(input, ".o") {
            ld_args.push(input.clone());
            continue;
        }

        if endswith(input, ".s") {
            if !args.opt_s {
                let output = args
                    .opt_o
                    .clone()
                    .unwrap_or_else(|| replace_extn(input, ".o"));
                assemble(input, &output, args.opt_hash_hash_hash)?;
                ld_args.push(output);
            }
            continue;
        }

        if !endswith(input, ".c") && input != "-" {
            return Err(format!("unknown file extension: {}", input));
        }

        if args.opt_e {
            run_cc1(args.opt_hash_hash_hash, &orig_args, Some(input), None)?;
            continue;
        }

        if args.opt_s {
            let output = args
                .opt_o
                .clone()
                .unwrap_or_else(|| replace_extn(input, ".s"));
            run_cc1(
                args.opt_hash_hash_hash,
                &orig_args,
                Some(input),
                Some(&output),
            )?;
            continue;
        }

        if args.opt_c {
            let output = args
                .opt_o
                .clone()
                .unwrap_or_else(|| replace_extn(input, ".o"));
            let (tmpfile, tmpfile_path) = create_tmpfile()?;
            _tmpfiles.push(tmpfile);
            run_cc1(
                args.opt_hash_hash_hash,
                &orig_args,
                Some(input),
                Some(&tmpfile_path),
            )?;
            assemble(&tmpfile_path, &output, args.opt_hash_hash_hash)?;
            continue;
        }

        let (tmpfile1, tmpfile_path1) = create_tmpfile()?;
        let (tmpfile2, tmpfile_path2) = create_tmpfile()?;
        _tmpfiles.push(tmpfile1);
        _tmpfiles.push(tmpfile2);
        run_cc1(
            args.opt_hash_hash_hash,
            &orig_args,
            Some(input),
            Some(&tmpfile_path1),
        )?;
        assemble(&tmpfile_path1, &tmpfile_path2, args.opt_hash_hash_hash)?;
        ld_args.push(tmpfile_path2);
    }

    if !ld_args.is_empty() {
        let output = args.opt_o.clone().unwrap_or_else(|| "a.out".to_string());
        run_linker(&ld_args, &output, args.opt_hash_hash_hash)?;
    }

    Ok(())
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("{msg}");
        process::exit(1);
    }
}

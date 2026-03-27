use std::{
    env, fs,
    io::{self, Read, Write},
    path::Path,
    process,
};

use cc_rs::codegen::emit_assembly;
use tempfile::NamedTempFile;

struct Args {
    opt_cc1: bool,
    opt_s: bool,
    opt_hash_hash_hash: bool,
    opt_o: Option<String>,
    input_path: String,
}

fn usage(status: i32) {
    eprintln!("Usage: cc-rs [ -o <path> ] <file>");
    process::exit(status);
}

fn parse_args() -> Args {
    let args: Vec<String> = env::args().collect();
    let mut opt_cc1 = false;
    let mut opt_s = false;
    let mut opt_hash_hash_hash = false;
    let mut opt_o: Option<String> = None;
    let mut input_path: Option<String> = None;
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

        if args[i].starts_with('-') && args[i].len() > 1 {
            eprintln!("unknown argument: {}", args[i]);
            process::exit(1);
        }

        input_path = Some(args[i].clone());
        i += 1;
    }

    let input_path = input_path.unwrap_or_else(|| {
        eprintln!("no input files");
        process::exit(1);
    });

    Args {
        opt_cc1,
        opt_s,
        opt_hash_hash_hash,
        opt_o,
        input_path,
    }
}

fn open_output_file(path: Option<&String>) -> Box<dyn Write> {
    if path.is_none() || path.unwrap().as_str() == "-" {
        return Box::new(io::stdout());
    }

    let file = fs::File::create(path.unwrap()).expect("cannot open output file");
    Box::new(file)
}

fn read_file(path: &str) -> Result<(String, String), String> {
    let filename = if path == "-" {
        String::from("<stdin>")
    } else {
        String::from(path)
    };

    let contents = if path == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("cannot read stdin: {e}"))?;
        buf
    } else {
        fs::read_to_string(path).map_err(|e| format!("cannot open {path}: {e}"))?
    };

    let mut contents = contents;
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }

    Ok((filename, contents))
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
        new_args.push(inp.to_string());
    }

    if let Some(out) = output {
        new_args.push("-o".to_string());
        new_args.push(out.to_string());
    }

    run_subprocess(opt_hash_hash_hash, &new_args)
}

fn cc1(args: &Args) -> Result<(), String> {
    let (filename, src) = read_file(&args.input_path)?;
    let asm = emit_assembly(&filename, &src)?;

    let mut out = open_output_file(args.opt_o.as_ref());
    out.write_all(asm.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn replace_extn(path: &str, extn: &str) -> String {
    let path = Path::new(path);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    format!("{}{}", stem, extn)
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

fn run() -> Result<(), String> {
    let args = parse_args();

    if args.opt_cc1 {
        return cc1(&args);
    }

    let orig_args: Vec<String> = env::args().collect();

    let output = if let Some(ref o) = args.opt_o {
        o.clone()
    } else if args.opt_s {
        replace_extn(&args.input_path, ".s")
    } else {
        replace_extn(&args.input_path, ".o")
    };

    if args.opt_s {
        run_cc1(
            args.opt_hash_hash_hash,
            &orig_args,
            Some(&args.input_path),
            Some(&output),
        )?;
        return Ok(());
    }

    let (_tmpfile, tmpfile_path) = create_tmpfile()?;
    run_cc1(
        args.opt_hash_hash_hash,
        &orig_args,
        Some(&args.input_path),
        Some(&tmpfile_path),
    )?;
    assemble(&tmpfile_path, &output, args.opt_hash_hash_hash)?;

    Ok(())
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("{msg}");
        process::exit(1);
    }
}

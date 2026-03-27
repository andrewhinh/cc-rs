use std::{
    env, fs,
    io::{self, Read, Write},
    process,
};

use cc_rs::codegen::emit_assembly;
use tempfile::NamedTempFile;

struct Args {
    opt_cc1: bool,
    opt_s: bool,
    opt_hash_hash_hash: bool,
    opt_o: Option<String>,
    base_file: Option<String>,
    output_file: Option<String>,
    input_paths: Vec<String>,
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
    let mut base_file: Option<String> = None;
    let mut output_file: Option<String> = None;
    let mut input_paths: Vec<String> = Vec::new();
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
        opt_hash_hash_hash,
        opt_o,
        base_file,
        output_file,
        input_paths,
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
        new_args.push("-cc1-input".to_string());
        new_args.push(inp.to_string());
    }

    if let Some(out) = output {
        new_args.push("-cc1-output".to_string());
        new_args.push(out.to_string());
    }

    run_subprocess(opt_hash_hash_hash, &new_args)
}

fn cc1(args: &Args) -> Result<(), String> {
    let input = args.base_file.as_ref().ok_or("no input file for cc1")?;
    let (filename, src) = read_file(input)?;
    let asm = emit_assembly(&filename, &src)?;

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

fn run() -> Result<(), String> {
    let args = parse_args();

    if args.opt_cc1 {
        return cc1(&args);
    }

    let orig_args: Vec<String> = env::args().collect();

    if args.input_paths.len() > 1 && args.opt_o.is_some() {
        return Err("cannot specify '-o' with multiple files".to_string());
    }

    for input in &args.input_paths {
        let output = if let Some(ref o) = args.opt_o {
            o.clone()
        } else if args.opt_s {
            replace_extn(input, ".s")
        } else {
            replace_extn(input, ".o")
        };

        if args.opt_s {
            run_cc1(
                args.opt_hash_hash_hash,
                &orig_args,
                Some(input),
                Some(&output),
            )?;
            continue;
        }

        let (_tmpfile, tmpfile_path) = create_tmpfile()?;
        run_cc1(
            args.opt_hash_hash_hash,
            &orig_args,
            Some(input),
            Some(&tmpfile_path),
        )?;
        assemble(&tmpfile_path, &output, args.opt_hash_hash_hash)?;
    }

    Ok(())
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("{msg}");
        process::exit(1);
    }
}

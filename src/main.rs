use std::{
    env, fs,
    io::{self, Read, Write},
    process,
};

use cc_rs::codegen::emit_assembly;

struct Args {
    opt_o: Option<String>,
    input_path: String,
}

fn usage(status: i32) {
    eprintln!("Usage: cc-rs [ -o <path> ] <file>");
    process::exit(status);
}

fn parse_args() -> Args {
    let args: Vec<String> = env::args().collect();
    let mut opt_o: Option<String> = None;
    let mut input_path: Option<String> = None;
    let mut i = 1;

    while i < args.len() {
        if args[i] == "--help" {
            usage(0);
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

    Args { opt_o, input_path }
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

fn run() -> Result<(), String> {
    let args = parse_args();

    let (filename, src) = read_file(&args.input_path)?;
    let asm = emit_assembly(&filename, &src)?;

    let mut out = open_output_file(args.opt_o.as_ref());
    out.write_all(asm.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("{msg}");
        process::exit(1);
    }
}

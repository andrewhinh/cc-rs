use std::{env, process};

fn usage(bin: &str) -> String {
    format!("Usage: {bin} <integer>")
}

fn parse_integer(src: &str) -> Result<u8, String> {
    src.trim()
        .parse::<u8>()
        .map_err(|_| format!("Expected 0-255 integer, got: {src}"))
}

fn emit_assembly(value: u8) -> String {
    if !cfg!(target_arch = "x86_64") {
        return String::from("Unsupported target architecture: require x86_64");
    }

    format!(
        ".text\n\
            .globl main\n\
            main:\n\
            mov ${value}, %eax\n\
            ret\n"
    )
}

fn run() -> Result<String, String> {
    let mut args = env::args();
    let bin = args.next().unwrap_or_else(|| String::from("cc-rs"));
    let src = args.next().ok_or_else(|| usage(&bin))?;
    if args.next().is_some() {
        return Err(usage(&bin));
    }

    let value = parse_integer(&src)?;
    Ok(emit_assembly(value))
}

fn main() {
    match run() {
        Ok(asm) => {
            print!("{asm}");
        }
        Err(msg) => {
            eprintln!("{msg}");
            process::exit(1);
        }
    }
}

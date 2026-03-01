use std::{env, process};

fn usage(bin: &str) -> String {
    format!("Usage: {bin} <expression>")
}

fn emit_assembly(src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let mut chars = src.chars().peekable();
    let mut result = String::new();

    result.push_str(".text\n");
    result.push_str(".globl main\n");
    result.push_str("main:\n");

    let first = parse_number(&mut chars)?;
    result.push_str(&format!("  mov ${first}, %rax\n"));

    while let Some(&c) = chars.peek() {
        match c {
            '+' => {
                chars.next();
                let n = parse_number(&mut chars)?;
                result.push_str(&format!("  add ${n}, %rax\n"));
            }
            '-' => {
                chars.next();
                let n = parse_number(&mut chars)?;
                result.push_str(&format!("  sub ${n}, %rax\n"));
            }
            _ => return Err(format!("Unexpected character: '{c}'")),
        }
    }

    result.push_str("  ret\n");
    Ok(result)
}

fn parse_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> Result<i64, String> {
    let mut num = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            num.push(c);
            chars.next();
        } else {
            break;
        }
    }

    if num.is_empty() {
        return Err("Expected number".to_string());
    }

    num.parse::<i64>()
        .map_err(|_| format!("Invalid number: {num}"))
}

fn run() -> Result<String, String> {
    let mut args = env::args();
    let bin = args.next().unwrap_or_else(|| String::from("cc-rs"));
    let src = args.next().ok_or_else(|| usage(&bin))?;
    if args.next().is_some() {
        return Err(usage(&bin));
    }

    emit_assembly(&src)
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

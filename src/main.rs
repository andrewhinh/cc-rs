use std::{env, process};

fn usage(bin: &str) -> String {
    format!("Usage: {bin} <expression>")
}

fn error_at(src: &str, loc: usize, msg: &str) -> String {
    format!("{}\n{:width$}^ {msg}\n", src, "", width = loc)
}

fn error_tok(src: &str, tok: &Token, msg: &str) -> String {
    error_at(src, tok.loc, msg)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Punct,
    Num,
    Eof,
}

#[derive(Debug)]
struct Token {
    kind: TokenKind,
    next: Option<Box<Token>>,
    val: i64,
    loc: usize,
    len: usize,
}

fn new_token(kind: TokenKind, start: usize, end: usize) -> Token {
    Token {
        kind,
        next: None,
        val: 0,
        loc: start,
        len: end - start,
    }
}

fn tokenize(src: &str) -> Result<Token, String> {
    let mut head = Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        loc: 0,
        len: 0,
    };
    let mut cur = &mut head;
    let chars: Vec<char> = src.chars().collect();
    let mut pos = 0;

    while pos < chars.len() {
        if chars[pos].is_whitespace() {
            pos += 1;
            continue;
        }

        if chars[pos].is_ascii_digit() {
            let start = pos;
            let mut num_str = String::new();
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                num_str.push(chars[pos]);
                pos += 1;
            }
            let val = num_str
                .parse::<i64>()
                .map_err(|_| format!("Invalid number: {num_str}"))?;
            let mut tok = new_token(TokenKind::Num, start, pos);
            tok.val = val;
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            continue;
        }

        if chars[pos] == '+' || chars[pos] == '-' {
            let tok = new_token(TokenKind::Punct, pos, pos + 1);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            pos += 1;
            continue;
        }

        return Err(error_at(src, pos, "invalid token"));
    }

    cur.next = Some(Box::new(new_token(TokenKind::Eof, pos, pos)));
    Ok(*head.next.unwrap())
}

fn equal(src: &str, tok: &Token, s: &str) -> bool {
    tok.kind == TokenKind::Punct
        && tok.len == s.len()
        && src.chars().skip(tok.loc).take(tok.len).eq(s.chars())
}

fn get_number(src: &str, tok: &Token) -> Result<i64, String> {
    if tok.kind != TokenKind::Num {
        return Err(error_tok(src, tok, "expected a number"));
    }
    Ok(tok.val)
}

fn emit_assembly(src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let mut tok = tokenize(src)?;
    let mut result = String::new();

    result.push_str(".text\n");
    result.push_str(".globl main\n");
    result.push_str("main:\n");

    let first = get_number(src, &tok)?;
    result.push_str(&format!("  mov ${first}, %rax\n"));
    tok = *tok.next.unwrap();

    while tok.kind != TokenKind::Eof {
        if equal(src, &tok, "+") {
            tok = *tok.next.unwrap();
            let n = get_number(src, &tok)?;
            result.push_str(&format!("  add ${n}, %rax\n"));
            tok = *tok.next.unwrap();
            continue;
        }

        if equal(src, &tok, "-") {
            tok = *tok.next.unwrap();
            let n = get_number(src, &tok)?;
            result.push_str(&format!("  sub ${n}, %rax\n"));
            tok = *tok.next.unwrap();
            continue;
        }

        return Err(error_tok(src, &tok, "unexpected token"));
    }

    result.push_str("  ret\n");
    Ok(result)
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

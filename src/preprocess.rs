use std::path::Path;

use crate::{File, Token, TokenKind, equal, error_tok, get_input_files, tokenize_file, warn_tok};

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "int"
            | "sizeof"
            | "char"
            | "short"
            | "struct"
            | "union"
            | "long"
            | "void"
            | "typedef"
            | "_Bool"
            | "enum"
            | "static"
            | "goto"
            | "break"
            | "continue"
            | "switch"
            | "case"
            | "default"
            | "extern"
            | "_Alignof"
            | "_Alignas"
            | "do"
            | "signed"
            | "unsigned"
            | "const"
            | "volatile"
            | "auto"
            | "register"
            | "restrict"
            | "__restrict"
            | "__restrict__"
            | "_Noreturn"
            | "float"
            | "double"
    )
}

fn is_hash(files: &[File], tok: &Token) -> bool {
    tok.at_bol && equal(files, tok, "#")
}

fn skip_line(files: &[File], mut tok: Token) -> Token {
    if tok.at_bol {
        return tok;
    }
    warn_tok(files, &tok, "extra token");
    while !tok.at_bol {
        tok = *tok.next.unwrap();
    }
    tok
}

fn convert_keywords(files: &[File], tok: &mut Token) {
    let mut cur = tok;
    loop {
        if cur.kind == TokenKind::Ident {
            let file = match files.iter().find(|f| f.file_no == cur.file_no) {
                Some(f) => f,
                None => break,
            };
            let name: String = file.contents.chars().skip(cur.loc).take(cur.len).collect();
            if is_keyword(&name) {
                cur.kind = TokenKind::Keyword;
            }
        }
        if cur.next.is_none() {
            break;
        }
        cur = cur.next.as_mut().unwrap();
    }
}

fn append(tok1: Token, tok2: Token) -> Token {
    if tok1.kind == TokenKind::Eof {
        return tok2;
    }

    let mut head = Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        fval: 0.0,
        loc: 0,
        len: 0,
        ty: None,
        str: None,
        file_no: 0,
        line_no: 0,
        at_bol: false,
    };
    let mut cur = &mut head;

    let mut tok = tok1;
    loop {
        if tok.kind == TokenKind::Eof {
            break;
        }
        let next = tok.next.take();
        cur.next = Some(Box::new(tok));
        cur = cur.next.as_mut().unwrap();
        match next {
            Some(n) => tok = *n,
            None => break,
        }
    }

    cur.next = Some(Box::new(tok2));
    *head.next.unwrap()
}

fn token_str_eq(files: &[File], tok: &Token, s: &str) -> bool {
    let file = match files.iter().find(|f| f.file_no == tok.file_no) {
        Some(f) => f,
        None => return false,
    };
    tok.len == s.len()
        && file
            .contents
            .chars()
            .skip(tok.loc)
            .take(tok.len)
            .eq(s.chars())
}

fn preprocess2(files: &[File], tok: Token) -> Result<Token, String> {
    let mut head = Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        fval: 0.0,
        loc: 0,
        len: 0,
        ty: None,
        str: None,
        file_no: 0,
        line_no: 0,
        at_bol: false,
    };
    let mut cur = &mut head;
    let mut tok = tok;

    loop {
        if tok.kind == TokenKind::Eof {
            break;
        }

        if !is_hash(files, &tok) {
            let next = tok.next.take();
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            tok = *next.unwrap();
            continue;
        }

        tok = *tok.next.unwrap();

        if token_str_eq(files, &tok, "include") {
            tok = *tok.next.unwrap();

            if tok.kind != TokenKind::Str {
                return Err(error_tok(files, &tok, "expected a filename"));
            }

            let filename = String::from_utf8_lossy(&tok.str.clone().unwrap()).into_owned();
            let current_file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
            let current_path = Path::new(&current_file.name);
            let dir = current_path.parent().unwrap_or(Path::new("."));
            let include_path = dir.join(&filename);

            let path_str = include_path.to_string_lossy().into_owned();
            let tok2 = tokenize_file(&path_str).ok_or_else(|| {
                error_tok(
                    files,
                    &tok,
                    &format!(
                        "cannot open {}: {}",
                        path_str,
                        std::io::Error::new(std::io::ErrorKind::NotFound, "file not found")
                    ),
                )
            })?;

            tok = skip_line(files, *tok.next.unwrap());
            tok = append(tok2, tok);
            continue;
        }

        if tok.at_bol {
            continue;
        }

        return Err(error_tok(files, &tok, "invalid preprocessor directive"));
    }

    cur.next = Some(Box::new(Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        fval: 0.0,
        loc: 0,
        len: 0,
        ty: None,
        str: None,
        file_no: 0,
        line_no: 0,
        at_bol: false,
    }));

    Ok(*head.next.unwrap())
}

pub fn preprocess(tok: Token) -> Result<Token, String> {
    let files = get_input_files();
    let tok = preprocess2(&files, tok)?;
    let mut tok = tok;
    let files = get_input_files();
    convert_keywords(&files, &mut tok);
    Ok(tok)
}

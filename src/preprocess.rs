use std::cell::Cell;
use std::path::Path;

use crate::{
    File, Token, TokenKind, const_expr, equal, error_tok, get_input_files, tokenize_file, warn_tok,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum CondInclCtx {
    InThen,
    InElse,
}

#[derive(Debug, Clone)]
struct CondIncl {
    next: Option<Box<CondIncl>>,
    ctx: CondInclCtx,
    tok: Token,
    included: bool,
}

thread_local! {
    static COND_INCL: Cell<Option<Box<CondIncl>>> = const { Cell::new(None) };
}

fn cond_incl_get() -> Option<Box<CondIncl>> {
    COND_INCL.with(|c| c.take())
}

fn cond_incl_set(ci: Option<Box<CondIncl>>) {
    COND_INCL.with(|c| c.set(ci));
}

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

fn copy_token(tok: &Token) -> Token {
    tok.clone()
}

fn new_eof(tok: &Token) -> Token {
    let mut t = copy_token(tok);
    t.kind = TokenKind::Eof;
    t.len = 0;
    t.next = None;
    t
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

fn skip_cond_incl2(files: &[File], mut tok: Token) -> Token {
    while tok.kind != TokenKind::Eof {
        if is_hash(files, &tok)
            && tok
                .next
                .as_ref()
                .is_some_and(|n| token_str_eq(files, n, "if"))
        {
            tok = skip_cond_incl2(files, *tok.next.unwrap().next.unwrap());
            continue;
        }
        if is_hash(files, &tok)
            && tok
                .next
                .as_ref()
                .is_some_and(|n| token_str_eq(files, n, "endif"))
        {
            return *tok.next.unwrap().next.unwrap();
        }
        tok = *tok.next.unwrap();
    }
    tok
}

fn skip_cond_incl(files: &[File], mut tok: Token) -> Token {
    while tok.kind != TokenKind::Eof {
        if is_hash(files, &tok)
            && tok
                .next
                .as_ref()
                .is_some_and(|n| token_str_eq(files, n, "if"))
        {
            tok = skip_cond_incl2(files, *tok.next.unwrap().next.unwrap());
            continue;
        }
        if is_hash(files, &tok)
            && tok
                .next
                .as_ref()
                .is_some_and(|n| token_str_eq(files, n, "else") || token_str_eq(files, n, "endif"))
        {
            break;
        }
        tok = *tok.next.unwrap();
    }
    tok
}

fn copy_line(_files: &[File], tok: &Token) -> (Token, Token) {
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
    let mut tok = tok.clone();

    while !tok.at_bol && tok.kind != TokenKind::Eof {
        let next = tok.next.take();
        cur.next = Some(Box::new(copy_token(&tok)));
        cur = cur.next.as_mut().unwrap();
        match next {
            Some(n) => tok = *n,
            None => break,
        }
    }

    cur.next = Some(Box::new(new_eof(&tok)));
    (*head.next.unwrap(), tok)
}

fn eval_const_expr(files: &[File], tok: &Token) -> Result<(i64, Token), String> {
    let start = tok.clone();
    let (expr, rest) = copy_line(files, tok.next.as_ref().unwrap());

    if expr.kind == TokenKind::Eof {
        return Err(error_tok(files, &start, "no expression"));
    }

    let mut empty_tag_scope_stack: Vec<Vec<crate::TagScope>> = Vec::new();
    let mut empty_scope_stack: Vec<Vec<crate::VarScope>> = Vec::new();
    let (val, rest2) = const_expr(
        files,
        &expr,
        &mut empty_tag_scope_stack,
        &mut empty_scope_stack,
    )?;
    if rest2.kind != TokenKind::Eof {
        return Err(error_tok(files, &rest2, "extra token"));
    }
    Ok((val, rest))
}

fn push_cond_incl(tok: &Token, included: bool) {
    let ci = CondIncl {
        next: cond_incl_get(),
        ctx: CondInclCtx::InThen,
        tok: tok.clone(),
        included,
    };
    cond_incl_set(Some(Box::new(ci)));
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

        let start = tok.clone();
        tok = *tok.next.unwrap();

        if token_str_eq(files, &tok, "include") {
            tok = *tok.next.unwrap();

            if tok.kind != TokenKind::Str {
                return Err(error_tok(files, &tok, "expected a filename"));
            }

            let filename = String::from_utf8_lossy(&tok.str.clone().unwrap()).into_owned();
            let include_path = if filename.starts_with('/') {
                filename
            } else {
                let current_file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
                let current_path = Path::new(&current_file.name);
                let dir = current_path.parent().unwrap_or(Path::new("."));
                dir.join(&filename).to_string_lossy().into_owned()
            };

            let tok2 = tokenize_file(&include_path).ok_or_else(|| {
                error_tok(
                    files,
                    &tok,
                    &format!(
                        "cannot open {}: {}",
                        include_path,
                        std::io::Error::new(std::io::ErrorKind::NotFound, "file not found")
                    ),
                )
            })?;

            tok = skip_line(files, *tok.next.unwrap());
            tok = append(tok2, tok);
            continue;
        }

        if token_str_eq(files, &tok, "if") {
            let (val, new_tok) = eval_const_expr(files, &tok)?;
            tok = new_tok;
            push_cond_incl(&start, val != 0);
            if val == 0 {
                tok = skip_cond_incl(files, tok);
            }
            continue;
        }

        if token_str_eq(files, &tok, "else") {
            let ci = cond_incl_get();
            if ci.is_none() || ci.as_ref().unwrap().ctx == CondInclCtx::InElse {
                return Err(error_tok(files, &start, "stray #else"));
            }
            let mut ci = ci.unwrap();
            ci.ctx = CondInclCtx::InElse;
            cond_incl_set(Some(ci));
            tok = skip_line(files, *tok.next.unwrap());

            let ci = cond_incl_get().unwrap();
            if ci.included {
                tok = skip_cond_incl(files, tok);
            }
            cond_incl_set(Some(ci));
            continue;
        }

        if token_str_eq(files, &tok, "endif") {
            let ci = cond_incl_get();
            if ci.is_none() {
                return Err(error_tok(files, &start, "stray #endif"));
            }
            cond_incl_set(ci.unwrap().next);
            tok = skip_line(files, *tok.next.unwrap());
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
    let ci = cond_incl_get();
    if let Some(c) = ci {
        return Err(error_tok(
            &files,
            &c.tok,
            "unterminated conditional directive",
        ));
    }
    let mut tok = tok;
    let files = get_input_files();
    convert_keywords(&files, &mut tok);
    Ok(tok)
}

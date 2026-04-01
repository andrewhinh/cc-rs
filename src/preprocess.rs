use std::cell::Cell;
use std::collections::HashSet;
use std::path::Path;

use crate::{
    File, Token, TokenKind, const_expr, equal, error_tok, get_input_files, skip, tokenize_file,
    warn_tok,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum CondInclCtx {
    Then,
    Elif,
    Else,
}

#[derive(Debug, Clone)]
struct CondIncl {
    next: Option<Box<CondIncl>>,
    ctx: CondInclCtx,
    tok: Token,
    included: bool,
}

#[derive(Debug, Clone)]
struct MacroParam {
    next: Option<Box<MacroParam>>,
    name: String,
}

#[derive(Debug, Clone)]
struct MacroArg {
    next: Option<Box<MacroArg>>,
    name: String,
    tok: Token,
}

#[derive(Debug, Clone)]
struct Macro {
    next: Option<Box<Macro>>,
    name: String,
    is_objlike: bool,
    params: Option<Box<MacroParam>>,
    body: Token,
    deleted: bool,
}

thread_local! {
    static COND_INCL: Cell<Option<Box<CondIncl>>> = const { Cell::new(None) };
    static MACROS: Cell<Option<Box<Macro>>> = const { Cell::new(None) };
}

fn cond_incl_get() -> Option<Box<CondIncl>> {
    COND_INCL.with(|c| c.take())
}

fn cond_incl_set(ci: Option<Box<CondIncl>>) {
    COND_INCL.with(|c| c.set(ci));
}

fn macros_get() -> Option<Box<Macro>> {
    MACROS.with(|m| m.take())
}

fn macros_set(m: Option<Box<Macro>>) {
    MACROS.with(|cell| cell.set(m));
}

fn find_macro(files: &[File], tok: &Token) -> Option<Macro> {
    if tok.kind != TokenKind::Ident {
        return None;
    }
    let file = files.iter().find(|f| f.file_no == tok.file_no)?;
    let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();

    let macros = macros_get();
    let mut result: Option<Macro> = None;
    let mut m = &macros;
    while let Some(current) = m {
        if current.name == name {
            if !current.deleted {
                result = Some((**current).clone());
            }
            break;
        }
        m = &current.next;
    }
    macros_set(macros);
    result
}

fn add_macro(
    name: String,
    is_objlike: bool,
    params: Option<Box<MacroParam>>,
    body: Token,
    deleted: bool,
) {
    let m = Macro {
        next: macros_get(),
        name,
        is_objlike,
        params,
        body,
        deleted,
    };
    macros_set(Some(Box::new(m)));
}

fn read_macro_params(
    files: &[File],
    tok: &Token,
) -> Result<(Option<Box<MacroParam>>, Token), String> {
    let mut head: Option<Box<MacroParam>> = None;
    let mut cur: &mut Option<Box<MacroParam>> = &mut head;
    let mut tok = tok.clone();

    while !equal(files, &tok, ")") {
        if cur.is_some() {
            tok = skip(files, &tok, ",")?;
        }

        if tok.kind != TokenKind::Ident {
            return Err(error_tok(files, &tok, "expected an identifier"));
        }

        let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
        let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();

        let param = Box::new(MacroParam { next: None, name });

        if cur.is_none() {
            head = Some(param);
            cur = &mut head;
        } else {
            cur.as_mut().unwrap().next = Some(param);
            cur = &mut cur.as_mut().unwrap().next;
        }

        tok = *tok.next.unwrap();
    }

    Ok((head, *tok.next.unwrap()))
}

fn read_macro_arg_one(files: &[File], tok: &Token) -> Result<(MacroArg, Token), String> {
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
        has_space: false,
        hideset: HashSet::new(),
    };
    let mut cur = &mut head;
    let mut tok = tok.clone();
    let mut level: i32 = 0;

    while level > 0 || (!equal(files, &tok, ",") && !equal(files, &tok, ")")) {
        if tok.kind == TokenKind::Eof {
            return Err(error_tok(files, &tok, "premature end of input"));
        }
        if equal(files, &tok, "(") {
            level += 1;
        } else if equal(files, &tok, ")") {
            level -= 1;
        }
        let next = tok.next.take();
        cur.next = Some(Box::new(copy_token(&tok)));
        cur = cur.next.as_mut().unwrap();
        match next {
            Some(n) => tok = *n,
            None => break,
        }
    }

    cur.next = Some(Box::new(new_eof(&tok)));

    let arg = MacroArg {
        next: None,
        name: String::new(),
        tok: *head.next.unwrap(),
    };

    Ok((arg, tok))
}

fn read_macro_args(
    files: &[File],
    tok: &Token,
    params: &Option<Box<MacroParam>>,
) -> Result<(Option<Box<MacroArg>>, Token), String> {
    let start = tok.clone();
    let mut tok = tok
        .next
        .as_ref()
        .unwrap()
        .next
        .as_ref()
        .unwrap()
        .as_ref()
        .clone();

    let mut head: Option<Box<MacroArg>> = None;
    let mut cur: &mut Option<Box<MacroArg>> = &mut head;

    let mut pp = params.as_ref();
    while let Some(p) = pp {
        if cur.is_some() {
            tok = skip(files, &tok, ",")?;
        }

        let (arg, new_tok) = read_macro_arg_one(files, &tok)?;
        tok = new_tok;

        let mut arg = arg;
        arg.name = p.name.clone();

        let arg_box = Box::new(arg);
        if cur.is_none() {
            head = Some(arg_box);
            cur = &mut head;
        } else {
            cur.as_mut().unwrap().next = Some(arg_box);
            cur = &mut cur.as_mut().unwrap().next;
        }

        pp = p.next.as_ref();
    }

    if pp.is_some() {
        return Err(error_tok(files, &start, "too many arguments"));
    }

    skip(files, &tok, ")")?;
    Ok((head, tok))
}

fn find_arg<'a>(
    args: &'a Option<Box<MacroArg>>,
    files: &[File],
    tok: &Token,
) -> Option<&'a MacroArg> {
    let mut ap = args.as_ref();
    while let Some(arg) = ap {
        let file = files.iter().find(|f| f.file_no == tok.file_no)?;
        let tok_str: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();
        if tok_str == arg.name {
            return Some(arg);
        }
        ap = arg.next.as_ref();
    }
    None
}

fn subst(files: &[File], tok: &Token, args: &Option<Box<MacroArg>>) -> Result<Token, String> {
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
        has_space: false,
        hideset: HashSet::new(),
    };
    let mut cur = &mut head;
    let mut tok = tok.clone();

    while tok.kind != TokenKind::Eof {
        if let Some(arg) = find_arg(args, files, &tok) {
            let t = preprocess2(files, arg.tok.clone())?;
            let mut t = t;
            while t.kind != TokenKind::Eof {
                let next = t.next.take();
                cur.next = Some(Box::new(copy_token(&t)));
                cur = cur.next.as_mut().unwrap();
                match next {
                    Some(n) => t = *n,
                    None => break,
                }
            }
            tok = *tok.next.unwrap();
            continue;
        }

        let next = tok.next.take();
        cur.next = Some(Box::new(copy_token(&tok)));
        cur = cur.next.as_mut().unwrap();
        match next {
            Some(n) => tok = *n,
            None => break,
        }
    }

    cur.next = Some(Box::new(tok));
    Ok(*head.next.unwrap())
}

fn read_macro_definition(files: &[File], tok: &Token) -> Result<Token, String> {
    if tok.kind != TokenKind::Ident {
        return Err(error_tok(files, tok, "macro name must be an identifier"));
    }
    let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
    let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();
    let next_tok = tok.next.as_ref().unwrap();

    if !next_tok.has_space && equal(files, next_tok, "(") {
        let (params, tok) = read_macro_params(files, next_tok.next.as_ref().unwrap())?;
        let (body, rest) = copy_line(files, &tok);
        add_macro(name, false, params, body, false);
        return Ok(rest);
    }

    let (body, rest) = copy_line(files, next_tok);
    add_macro(name, true, None, body, false);
    Ok(rest)
}

fn expand_macro(files: &[File], tok: &Token) -> Option<Token> {
    let m = find_macro(files, tok)?;

    let file = files.iter().find(|f| f.file_no == tok.file_no)?;
    let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();

    if hideset_contains(&tok.hideset, &name) {
        return None;
    }

    if m.is_objlike {
        let mut hs = tok.hideset.clone();
        hs.insert(m.name.clone());
        let body = add_hideset(m.body, &hs);
        return Some(append(body, *tok.next.as_ref().unwrap().clone()));
    }

    if !equal(files, tok.next.as_ref().unwrap(), "(") {
        return None;
    }

    let macro_token = tok;
    let (args, rparen) = read_macro_args(files, tok, &m.params).ok()?;
    let hs = hideset_intersection(&macro_token.hideset, &rparen.hideset);
    let hs = hideset_union(&hs, &HashSet::from([m.name.clone()]));
    let body = subst(files, &m.body, &args).ok()?;
    let body = add_hideset(body, &hs);
    Some(append(body, *rparen.next.unwrap()))
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

fn hideset_contains(hs: &HashSet<String>, name: &str) -> bool {
    hs.contains(name)
}

fn hideset_intersection(hs1: &HashSet<String>, hs2: &HashSet<String>) -> HashSet<String> {
    hs1.intersection(hs2).cloned().collect()
}

fn hideset_union(hs1: &HashSet<String>, hs2: &HashSet<String>) -> HashSet<String> {
    hs1.union(hs2).cloned().collect()
}

fn add_hideset(tok: Token, hs: &HashSet<String>) -> Token {
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
        has_space: false,
        hideset: HashSet::new(),
    };
    let mut cur = &mut head;
    let mut tok = tok;

    while tok.kind != TokenKind::Eof {
        let next = tok.next.take();
        let mut t = copy_token(&tok);
        t.hideset = hideset_union(&t.hideset, hs);
        cur.next = Some(Box::new(t));
        cur = cur.next.as_mut().unwrap();
        match next {
            Some(n) => tok = *n,
            None => break,
        }
    }

    cur.next = Some(Box::new(new_eof(&tok)));
    *head.next.unwrap()
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
            && tok.next.as_ref().is_some_and(|n| {
                token_str_eq(files, n, "if")
                    || token_str_eq(files, n, "ifdef")
                    || token_str_eq(files, n, "ifndef")
            })
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
            && tok.next.as_ref().is_some_and(|n| {
                token_str_eq(files, n, "if")
                    || token_str_eq(files, n, "ifdef")
                    || token_str_eq(files, n, "ifndef")
            })
        {
            tok = skip_cond_incl2(files, *tok.next.unwrap().next.unwrap());
            continue;
        }
        if is_hash(files, &tok)
            && tok.next.as_ref().is_some_and(|n| {
                token_str_eq(files, n, "elif")
                    || token_str_eq(files, n, "else")
                    || token_str_eq(files, n, "endif")
            })
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
        has_space: false,
        hideset: HashSet::new(),
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
    let expr = preprocess2(files, expr)?;

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
        ctx: CondInclCtx::Then,
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
        has_space: false,
        hideset: HashSet::new(),
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
        has_space: false,
        hideset: HashSet::new(),
    };
    let mut cur = &mut head;
    let mut tok = tok;

    loop {
        if tok.kind == TokenKind::Eof {
            break;
        }

        if let Some(expanded) = expand_macro(files, &tok) {
            tok = expanded;
            continue;
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

        if token_str_eq(files, &tok, "ifdef") {
            let defined = find_macro(files, tok.next.as_ref().unwrap()).is_some();
            push_cond_incl(&start, defined);
            tok = skip_line(files, *tok.next.unwrap().next.unwrap());
            if !defined {
                tok = skip_cond_incl(files, tok);
            }
            continue;
        }

        if token_str_eq(files, &tok, "ifndef") {
            let defined = find_macro(files, tok.next.as_ref().unwrap()).is_some();
            push_cond_incl(&start, !defined);
            tok = skip_line(files, *tok.next.unwrap().next.unwrap());
            if defined {
                tok = skip_cond_incl(files, tok);
            }
            continue;
        }

        if token_str_eq(files, &tok, "elif") {
            let ci = cond_incl_get();
            if ci.is_none() || ci.as_ref().unwrap().ctx == CondInclCtx::Else {
                return Err(error_tok(files, &start, "stray #elif"));
            }
            let mut ci = ci.unwrap();
            ci.ctx = CondInclCtx::Elif;
            cond_incl_set(Some(ci.clone()));

            if !ci.included {
                let (val, new_tok) = eval_const_expr(files, &tok)?;
                tok = new_tok;
                if val != 0 {
                    ci.included = true;
                    cond_incl_set(Some(ci));
                } else {
                    tok = skip_cond_incl(files, tok);
                }
            } else {
                tok = skip_cond_incl(files, tok);
            }
            continue;
        }

        if token_str_eq(files, &tok, "else") {
            let ci = cond_incl_get();
            if ci.is_none() || ci.as_ref().unwrap().ctx == CondInclCtx::Else {
                return Err(error_tok(files, &start, "stray #else"));
            }
            let mut ci = ci.unwrap();
            ci.ctx = CondInclCtx::Else;
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

        if token_str_eq(files, &tok, "define") {
            tok = read_macro_definition(files, tok.next.as_ref().unwrap())?;
            continue;
        }

        if token_str_eq(files, &tok, "undef") {
            tok = *tok.next.unwrap();
            if tok.kind != TokenKind::Ident {
                return Err(error_tok(files, &tok, "macro name must be an identifier"));
            }
            let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
            let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();
            tok = skip_line(files, *tok.next.unwrap());
            add_macro(name, true, None, new_eof(&tok), true);
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
        has_space: false,
        hideset: HashSet::new(),
    }));

    Ok(*head.next.unwrap())
}

pub fn preprocess(tok: Token) -> Result<Token, String> {
    macros_set(None);
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

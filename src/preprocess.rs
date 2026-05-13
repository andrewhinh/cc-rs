use std::cell::Cell;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};

use chrono::Local;

use crate::tokenize::{line_delta_for_file, update_file_line_marker};
use crate::{
    File, Token, TokenKind, add_input_file, const_expr, consume, convert_pp_number, equal,
    error_tok, get_file_no, get_include_paths, get_input_files, is_integer, new_file, skip,
    tokenize, tokenize_file, tokenize_string_literal, warn_tok,
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

type MacroHandler = fn(&Token) -> Token;

static COUNTER: AtomicI32 = AtomicI32::new(0);

pub fn reset_counter() {
    COUNTER.store(0, Ordering::Relaxed);
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
    is_variadic: bool,
    body: Token,
    deleted: bool,
    handler: Option<MacroHandler>,
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
    is_variadic: bool,
    body: Token,
    deleted: bool,
    handler: Option<MacroHandler>,
) {
    let m = Macro {
        next: macros_get(),
        name,
        is_objlike,
        params,
        is_variadic,
        body,
        deleted,
        handler,
    };
    macros_set(Some(Box::new(m)));
}

pub fn define_macro(name: &str, body: &str) {
    let file_no = get_file_no();
    let file = new_file("<built-in>".to_string(), file_no, body.to_string());
    add_input_file(file.clone());
    let tok = tokenize(&file);
    add_macro(name.to_string(), true, None, false, tok, false, None);
}

pub fn undef_macro(name: &str) {
    let file_no = get_file_no();
    let file = new_file("<built-in>".to_string(), file_no, String::new());
    add_input_file(file.clone());
    let tok = tokenize(&file);
    add_macro(name.to_string(), true, None, false, tok, true, None);
}

fn add_builtin(name: &str, handler: MacroHandler) {
    let file_no = get_file_no();
    let file = new_file("<built-in>".to_string(), file_no, String::new());
    add_input_file(file.clone());
    let tok = tokenize(&file);
    add_macro(
        name.to_string(),
        true,
        None,
        false,
        tok,
        false,
        Some(handler),
    );
}

fn detached_token(tok: &Token) -> Token {
    Token {
        kind: tok.kind,
        next: None,
        val: tok.val,
        fval: tok.fval,
        loc: tok.loc,
        len: tok.len,
        ty: tok.ty.clone(),
        str: tok.str.clone(),
        file_no: tok.file_no,
        line_no: tok.line_no,
        line_delta: tok.line_delta,
        at_bol: tok.at_bol,
        has_space: tok.has_space,
        hideset: tok.hideset.clone(),
        origin: None,
    }
}

fn root_origin(tok: &Token) -> Token {
    detached_token(macro_origin(tok))
}

fn macro_origin(mut tmpl: &Token) -> &Token {
    while let Some(origin) = tmpl.origin.as_deref() {
        tmpl = origin;
    }
    tmpl
}

fn builtin_spelling_and_file<'a>(files: &'a [File], tmpl: &'a Token) -> (&'a Token, &'a File) {
    let tmpl = macro_origin(tmpl);
    let file = files.iter().find(|f| f.file_no == tmpl.file_no).unwrap();
    (tmpl, file)
}

fn file_macro(tmpl: &Token) -> Token {
    let files = get_input_files();
    let (tmpl, file) = builtin_spelling_and_file(&files, tmpl);
    new_str_token(&files, &file.display_name, tmpl)
}

fn line_macro(tmpl: &Token) -> Token {
    let files = get_input_files();
    let (tmpl, file) = builtin_spelling_and_file(&files, tmpl);
    new_num_token(&files, tmpl.line_no as i64 + file.line_delta, tmpl)
}

fn counter_macro(tmpl: &Token) -> Token {
    let files = get_input_files();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) as i64;
    new_num_token(&files, n, tmpl)
}

pub fn init_macros() {
    define_macro("_LP64", "1");
    define_macro("__C99_MACRO_WITH_VA_ARGS", "1");
    define_macro("__ELF__", "1");
    define_macro("__LP64__", "1");
    define_macro("__SIZEOF_DOUBLE__", "8");
    define_macro("__SIZEOF_FLOAT__", "4");
    define_macro("__SIZEOF_INT__", "4");
    define_macro("__SIZEOF_LONG_DOUBLE__", "8");
    define_macro("__SIZEOF_LONG_LONG__", "8");
    define_macro("__SIZEOF_LONG__", "8");
    define_macro("__SIZEOF_POINTER__", "8");
    define_macro("__SIZEOF_PTRDIFF_T__", "8");
    define_macro("__SIZEOF_SHORT__", "2");
    define_macro("__SIZEOF_SIZE_T__", "8");
    define_macro("__SIZE_TYPE__", "unsigned long");
    define_macro("__STDC_HOSTED__", "1");
    define_macro("__STDC_NO_ATOMICS__", "1");
    define_macro("__STDC_NO_COMPLEX__", "1");
    define_macro("__STDC_NO_THREADS__", "1");
    define_macro("__STDC_NO_VLA__", "1");
    define_macro("__STDC_UTF_16__", "1");
    define_macro("__STDC_UTF_32__", "1");
    define_macro("__STDC_VERSION__", "201112L");
    define_macro("__STDC__", "1");
    define_macro("__USER_LABEL_PREFIX__", "");
    define_macro("__alignof__", "_Alignof");
    define_macro("__amd64", "1");
    define_macro("__amd64__", "1");
    define_macro("__cc_rs__", "1");
    define_macro("__const__", "const");
    define_macro("__gnu_linux__", "1");
    define_macro("__inline__", "inline");
    define_macro("__linux", "1");
    define_macro("__linux__", "1");
    define_macro("__signed__", "signed");
    define_macro("__typeof__", "typeof");
    define_macro("__unix", "1");
    define_macro("__unix__", "1");
    define_macro("__volatile__", "volatile");
    define_macro("__x86_64", "1");
    define_macro("__x86_64__", "1");
    define_macro("linux", "1");
    define_macro("unix", "1");

    add_builtin("__FILE__", file_macro);
    add_builtin("__LINE__", line_macro);
    add_builtin("__COUNTER__", counter_macro);

    let now = Local::now();
    let date_s = now.format("%b %e %Y").to_string();
    let time_s = now.format("%H:%M:%S").to_string();
    define_macro("__DATE__", &format!("\"{date_s}\""));
    define_macro("__TIME__", &format!("\"{time_s}\""));
}

fn read_macro_params(
    files: &[File],
    tok: &Token,
    is_variadic: &mut bool,
) -> Result<(Option<Box<MacroParam>>, Token), String> {
    let mut head: Option<Box<MacroParam>> = None;
    let mut cur: &mut Option<Box<MacroParam>> = &mut head;
    let mut tok = tok.clone();

    while !equal(files, &tok, ")") {
        if cur.is_some() {
            tok = skip(files, &tok, ",")?;
        }

        if equal(files, &tok, "...") {
            *is_variadic = true;
            let tok = skip(files, tok.next.as_ref().unwrap(), ")")?;
            return Ok((head, tok));
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

fn read_macro_arg_one(
    files: &[File],
    tok: &Token,
    read_rest: bool,
) -> Result<(MacroArg, Token), String> {
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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
    };
    let mut cur = &mut head;
    let mut tok = tok.clone();
    let mut level: i32 = 0;

    loop {
        if level == 0 && equal(files, &tok, ")") {
            break;
        }
        if level == 0 && !read_rest && equal(files, &tok, ",") {
            break;
        }

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
    is_variadic: bool,
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

        let (arg, new_tok) = read_macro_arg_one(files, &tok, false)?;
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

    if is_variadic {
        let arg = if equal(files, &tok, ")") {
            MacroArg {
                next: None,
                name: "__VA_ARGS__".to_string(),
                tok: new_eof(&tok),
            }
        } else {
            if params.is_some() {
                tok = skip(files, &tok, ",")?;
            }
            let (mut arg, new_tok) = read_macro_arg_one(files, &tok, true)?;
            tok = new_tok;
            arg.name = "__VA_ARGS__".to_string();
            arg
        };
        let arg_box = Box::new(arg);
        if cur.is_none() {
            head = Some(arg_box);
        } else {
            cur.as_mut().unwrap().next = Some(arg_box);
        }
    } else if pp.is_some() {
        return Err(error_tok(files, &start, "too many arguments"));
    }

    let rparen = tok.clone();
    skip(files, &tok, ")")?;
    Ok((head, rparen))
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

fn quote_string(str: &str) -> String {
    let mut bufsize = 3;
    for c in str.chars() {
        if c == '\\' || c == '"' {
            bufsize += 1;
        }
        bufsize += 1;
    }

    let mut buf = String::with_capacity(bufsize);
    buf.push('"');
    for c in str.chars() {
        if c == '\\' || c == '"' {
            buf.push('\\');
        }
        buf.push(c);
    }
    buf.push('"');
    buf
}

fn new_str_token(files: &[File], s: &str, tmpl: &Token) -> Token {
    let quoted = quote_string(s);
    let file = files.iter().find(|f| f.file_no == tmpl.file_no).unwrap();
    let file_no = get_file_no();
    let new_file = new_file(file.name.clone(), file_no, quoted);
    add_input_file(new_file.clone());
    tokenize(&new_file)
}

fn new_num_token(files: &[File], val: i64, tmpl: &Token) -> Token {
    let buf = format!("{}\n", val);
    let file = files.iter().find(|f| f.file_no == tmpl.file_no).unwrap();
    let file_no = get_file_no();
    let new_file = new_file(file.name.clone(), file_no, buf);
    add_input_file(new_file.clone());
    tokenize(&new_file)
}

fn join_tokens(files: &[File], tok: &Token, end: Option<&Token>) -> String {
    let mut len = 1;
    let mut t = tok.clone();
    while t.kind != TokenKind::Eof {
        if let Some(e) = end
            && t.loc == e.loc
            && t.file_no == e.file_no
        {
            break;
        }
        if t.has_space && t.loc != tok.loc {
            len += 1;
        }
        len += t.len;
        t = *t.next.unwrap();
    }

    let mut buf = String::with_capacity(len);
    let mut t = tok.clone();
    while t.kind != TokenKind::Eof {
        if let Some(e) = end
            && t.loc == e.loc
            && t.file_no == e.file_no
        {
            break;
        }
        if t.has_space && t.loc != tok.loc {
            buf.push(' ');
        }
        let file = files.iter().find(|f| f.file_no == t.file_no).unwrap();
        let token_str: String = file.contents.chars().skip(t.loc).take(t.len).collect();
        buf.push_str(&token_str);
        t = *t.next.unwrap();
    }
    buf
}

fn stringize(files: &[File], hash: &Token, arg: &Token) -> Token {
    let s = join_tokens(files, arg, None);
    new_str_token(files, &s, hash)
}

fn paste(lhs: &Token, rhs: &Token) -> Result<Token, String> {
    let files = get_input_files();
    let lhs_file = files.iter().find(|f| f.file_no == lhs.file_no).unwrap();
    let rhs_file = files.iter().find(|f| f.file_no == rhs.file_no).unwrap();
    let lhs_str: String = lhs_file
        .contents
        .chars()
        .skip(lhs.loc)
        .take(lhs.len)
        .collect();
    let rhs_str: String = rhs_file
        .contents
        .chars()
        .skip(rhs.loc)
        .take(rhs.len)
        .collect();
    let buf = format!("{}{}", lhs_str, rhs_str);

    let file_no = get_file_no();
    let new_file = new_file(lhs_file.name.clone(), file_no, buf.clone());
    add_input_file(new_file.clone());
    let tok = tokenize(&new_file);

    if tok.next.as_ref().is_some_and(|n| n.kind != TokenKind::Eof) {
        return Err(error_tok(
            &files,
            lhs,
            &format!("pasting forms '{}', an invalid token", buf),
        ));
    }

    Ok(tok)
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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
    };
    let mut cur = &mut head;
    let mut tok = tok.clone();
    let mut is_start = true;

    while tok.kind != TokenKind::Eof {
        if equal(files, &tok, "#") {
            let arg = find_arg(args, files, tok.next.as_ref().unwrap());
            if arg.is_none() {
                return Err(error_tok(
                    files,
                    tok.next.as_ref().unwrap(),
                    "'#' is not followed by a macro parameter",
                ));
            }
            let arg = arg.unwrap();
            cur.next = Some(Box::new(stringize(files, &tok, &arg.tok)));
            cur = cur.next.as_mut().unwrap();
            is_start = false;
            tok = *tok.next.take().unwrap().next.take().unwrap();
            continue;
        }

        if equal(files, &tok, "##") {
            if is_start {
                return Err(error_tok(
                    files,
                    &tok,
                    "'##' cannot appear at start of macro expansion",
                ));
            }

            if tok.next.as_ref().is_none_or(|n| n.kind == TokenKind::Eof) {
                return Err(error_tok(
                    files,
                    &tok,
                    "'##' cannot appear at end of macro expansion",
                ));
            }

            let arg = find_arg(args, files, tok.next.as_ref().unwrap());
            if let Some(arg) = arg {
                if arg.tok.kind != TokenKind::Eof {
                    let cur_tok = (*cur).clone();
                    *cur = paste(&cur_tok, &arg.tok)?;
                    let mut t = arg.tok.next.clone();
                    while let Some(token) = t {
                        if token.kind == TokenKind::Eof {
                            break;
                        }
                        let next = token.next.clone();
                        cur.next = Some(Box::new(copy_token(&token)));
                        cur = cur.next.as_mut().unwrap();
                        t = next;
                    }
                }
                tok = *tok.next.take().unwrap().next.take().unwrap();
                continue;
            }

            let cur_tok = (*cur).clone();
            *cur = paste(&cur_tok, tok.next.as_ref().unwrap())?;
            tok = *tok.next.take().unwrap().next.take().unwrap();
            continue;
        }

        if let Some(arg) = find_arg(args, files, &tok) {
            if equal(files, tok.next.as_ref().unwrap(), "##") {
                if arg.tok.kind == TokenKind::Eof {
                    let rhs = tok.next.as_ref().unwrap().next.as_ref().unwrap();
                    let arg2 = find_arg(args, files, rhs);
                    if let Some(arg2) = arg2 {
                        let mut t: Option<Token> = Some(arg2.tok.clone());
                        while let Some(token) = t {
                            if token.kind == TokenKind::Eof {
                                break;
                            }
                            let next = token.next.clone();
                            cur.next = Some(Box::new(copy_token(&token)));
                            cur = cur.next.as_mut().unwrap();
                            is_start = false;
                            t = next.map(|n| *n);
                        }
                    } else {
                        cur.next = Some(Box::new(copy_token(rhs)));
                        cur = cur.next.as_mut().unwrap();
                        is_start = false;
                    }
                    tok = *rhs.next.clone().unwrap();
                    continue;
                }

                let mut t: Option<Token> = Some(arg.tok.clone());
                while let Some(token) = t {
                    if token.kind == TokenKind::Eof {
                        break;
                    }
                    let next = token.next.clone();
                    cur.next = Some(Box::new(copy_token(&token)));
                    cur = cur.next.as_mut().unwrap();
                    is_start = false;
                    t = next.map(|n| *n);
                }
                tok = *tok.next.take().unwrap();
                continue;
            }

            let mut t = preprocess2(&[], arg.tok.clone())?;
            t.at_bol = tok.at_bol;
            t.has_space = tok.has_space;
            while t.kind != TokenKind::Eof {
                let next = t.next.take();
                cur.next = Some(Box::new(copy_token(&t)));
                cur = cur.next.as_mut().unwrap();
                is_start = false;
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
        is_start = false;
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
        let mut is_variadic = false;
        let (params, tok) =
            read_macro_params(files, next_tok.next.as_ref().unwrap(), &mut is_variadic)?;
        let (body, rest) = copy_line(files, &tok);
        add_macro(name, false, params, is_variadic, body, false, None);
        return Ok(rest);
    }

    let (body, rest) = copy_line(files, next_tok);
    add_macro(name, true, None, false, body, false, None);
    Ok(rest)
}

fn expand_macro(files: &[File], tok: &Token) -> Option<Token> {
    let m = find_macro(files, tok)?;

    let file = files.iter().find(|f| f.file_no == tok.file_no)?;
    let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();

    if hideset_contains(&tok.hideset, &name) {
        return None;
    }

    if let Some(handler) = m.handler {
        let mut result = handler(tok);
        result.next = tok.next.clone();
        return Some(result);
    }

    if m.is_objlike {
        let mut hs = tok.hideset.clone();
        hs.insert(m.name.clone());
        let mut body = add_hideset(m.body, &hs);
        set_origin(&mut body, tok);
        let mut result = append(body, *tok.next.as_ref().unwrap().clone());
        result.at_bol = tok.at_bol;
        result.has_space = tok.has_space;
        return Some(result);
    }

    if !equal(files, tok.next.as_ref().unwrap(), "(") {
        return None;
    }

    let macro_token = tok;
    let (args, rparen) = read_macro_args(files, tok, &m.params, m.is_variadic).ok()?;
    let hs = hideset_intersection(&macro_token.hideset, &rparen.hideset);
    let hs = hideset_union(&hs, &HashSet::from([m.name.clone()]));
    let mut body = subst(files, &m.body, &args).ok()?;
    body = add_hideset(body, &hs);
    set_origin(&mut body, macro_token);
    let next_tok = rparen
        .next
        .clone()
        .unwrap_or_else(|| Box::new(new_eof(&rparen)));
    let mut result = append(body, *next_tok);
    result.at_bol = macro_token.at_bol;
    result.has_space = macro_token.has_space;
    Some(result)
}

fn set_origin(tok: &mut Token, origin: &Token) {
    let origin_box = Box::new(root_origin(origin));
    let mut cur = tok;
    while cur.kind != TokenKind::Eof {
        cur.origin = Some(origin_box.clone());
        if let Some(ref mut next) = cur.next {
            cur = next;
        } else {
            break;
        }
    }
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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
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

fn read_const_expr(files: &[File], tok: &Token) -> Result<(Token, Token), String> {
    let (line, rest) = copy_line(files, tok);

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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
    };
    let mut cur = &mut head;
    let mut tok = line;

    while tok.kind != TokenKind::Eof {
        if token_str_eq(files, &tok, "defined") {
            let start = tok.clone();
            tok = *tok.next.unwrap();
            let (has_paren, new_tok) = consume(files, &tok, "(");
            tok = new_tok;

            if tok.kind != TokenKind::Ident {
                return Err(error_tok(files, &start, "macro name must be an identifier"));
            }

            let m = find_macro(files, &tok);
            tok = *tok.next.unwrap();

            if has_paren {
                tok = skip(files, &tok, ")")?;
            }

            cur.next = Some(Box::new(new_num_token(
                files,
                if m.is_some() { 1 } else { 0 },
                &start,
            )));
            cur = cur.next.as_mut().unwrap();
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
    Ok((*head.next.unwrap(), rest))
}

fn replace_idents_with_zero(mut tok: Token) -> Token {
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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
    };
    let mut cur = &mut head;

    while tok.kind != TokenKind::Eof {
        let next = tok.next.take();
        if tok.kind == TokenKind::Ident {
            tok.kind = TokenKind::Num;
            tok.val = 0;
            tok.ty = Some(crate::Type::new_int());
        }
        cur.next = Some(Box::new(tok));
        cur = cur.next.as_mut().unwrap();
        tok = *next.unwrap();
    }

    cur.next = Some(Box::new(tok));
    *head.next.unwrap()
}

fn eval_const_expr(files: &[File], tok: &Token) -> Result<(i64, Token), String> {
    let start = tok.clone();
    let (expr, rest) = read_const_expr(files, tok.next.as_ref().unwrap())?;
    let mut expr = preprocess2(&[], expr)?;

    if expr.kind == TokenKind::Eof {
        return Err(error_tok(files, &start, "no expression"));
    }

    let files = get_input_files();
    convert_pp_tokens(&files, &mut expr);

    let expr = replace_idents_with_zero(expr);

    let mut empty_tag_scope_stack: Vec<Vec<crate::TagScope>> = Vec::new();
    let mut empty_scope_stack: Vec<Vec<crate::VarScope>> = Vec::new();
    let (val, rest2) = const_expr(
        &files,
        &expr,
        &mut empty_tag_scope_stack,
        &mut empty_scope_stack,
    )?;
    if rest2.kind != TokenKind::Eof {
        return Err(error_tok(&files, &rest2, "extra token"));
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

fn convert_pp_tokens(files: &[File], tok: &mut Token) {
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
        } else if cur.kind == TokenKind::PpNum
            && let Err(e) = convert_pp_number(files, cur)
        {
            eprintln!("{}", e);
            std::process::exit(1);
        }
        if cur.next.is_none() {
            break;
        }
        cur = cur.next.as_mut().unwrap();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConcatLitKind {
    Plain,
    Utf8,
    Utf16,
    Utf32,
    Wide,
}

fn concat_literal_kind(chars: &[char], loc: usize) -> ConcatLitKind {
    if loc + 2 < chars.len() && chars[loc] == 'u' && chars[loc + 1] == '8' && chars[loc + 2] == '"'
    {
        ConcatLitKind::Utf8
    } else if chars.get(loc).is_some_and(|c| *c == '"') {
        ConcatLitKind::Plain
    } else if loc + 1 < chars.len() && chars[loc] == 'u' && chars[loc + 1] == '"' {
        ConcatLitKind::Utf16
    } else if loc + 1 < chars.len() && chars[loc] == 'U' && chars[loc + 1] == '"' {
        ConcatLitKind::Utf32
    } else if loc + 1 < chars.len() && chars[loc] == 'L' && chars[loc + 1] == '"' {
        ConcatLitKind::Wide
    } else {
        ConcatLitKind::Plain
    }
}

fn array_base(ty: &crate::Type) -> crate::Type {
    ty.base.as_ref().unwrap().borrow().clone()
}

fn file_chars_cached(files: &[File], file_no: usize, cf: &mut usize, cv: &mut Vec<char>) {
    if *cf != file_no {
        let f = files.iter().find(|f| f.file_no == file_no).unwrap();
        cv.clear();
        cv.extend(f.contents.chars());
        *cf = file_no;
    }
}

fn widen_adjacent_string_literals(files: &[File], head: &mut Token) -> Result<(), String> {
    use std::ptr;

    let mut curr: *mut Token = ptr::from_mut(head);
    unsafe {
        loop {
            if (*curr).kind == TokenKind::Eof {
                break;
            }

            let next_is_str = (*curr)
                .next
                .as_ref()
                .is_some_and(|n| n.kind == TokenKind::Str);
            let has_adjacent = (*curr).kind == TokenKind::Str && next_is_str;

            if !has_adjacent {
                match (*curr).next.as_deref_mut() {
                    Some(n) => curr = ptr::from_mut(n),
                    None => break,
                }
                continue;
            }

            let (basety, needs_widen) = classify_concat_run(&*curr, files)?;

            let mut w = curr;
            loop {
                if (*w).kind != TokenKind::Str {
                    curr = w;
                    break;
                }
                if needs_widen && array_base((*w).ty.as_ref().unwrap()).size == 1 {
                    tokenize_string_literal(files, &mut *w, &basety)?;
                }
                match (*w).next.as_deref_mut() {
                    Some(n) => w = ptr::from_mut(n),
                    None => {
                        curr = w;
                        break;
                    }
                }
            }
        }
    }
    Ok(())
}

fn classify_concat_run(first_str: &Token, files: &[File]) -> Result<(crate::Type, bool), String> {
    let mut cf = usize::MAX;
    let mut cv: Vec<char> = Vec::new();

    file_chars_cached(files, first_str.file_no, &mut cf, &mut cv);
    let mut kind = concat_literal_kind(cv.as_slice(), first_str.loc);
    let mut basety = array_base(first_str.ty.as_ref().unwrap());

    let mut narrow_piece = array_base(first_str.ty.as_ref().unwrap()).size == 1;

    let mut scan = first_str.next.as_deref();
    while let Some(t_ref) = scan {
        if t_ref.kind != TokenKind::Str {
            break;
        }
        file_chars_cached(files, t_ref.file_no, &mut cf, &mut cv);
        let k = concat_literal_kind(cv.as_slice(), t_ref.loc);
        if kind == ConcatLitKind::Plain {
            kind = k;
            basety = array_base(t_ref.ty.as_ref().unwrap());
        } else if k != ConcatLitKind::Plain && k != kind {
            return Err(error_tok(
                files,
                t_ref,
                "unsupported non-standard concatenation of string literals",
            ));
        }
        narrow_piece |= array_base(t_ref.ty.as_ref().unwrap()).size == 1;

        scan = t_ref.next.as_deref();
    }

    let needs_widen = basety.size > 1 && narrow_piece;

    Ok((basety, needs_widen))
}

fn merge_adjacent_string_literals(tok: &mut Token) {
    let mut tok1 = tok;
    loop {
        if tok1.kind == TokenKind::Eof {
            break;
        }
        if tok1.kind != TokenKind::Str
            || tok1.next.is_none()
            || tok1.next.as_ref().unwrap().kind != TokenKind::Str
        {
            tok1 = tok1.next.as_mut().unwrap();
            continue;
        }

        let tok2_loc;
        let tok2_file_no;
        {
            let mut tok2 = tok1.next.as_mut().unwrap();
            while tok2.kind == TokenKind::Str {
                if tok2.next.is_none() {
                    break;
                }
                tok2 = tok2.next.as_mut().unwrap();
            }
            tok2_loc = tok2.loc;
            tok2_file_no = tok2.file_no;
        }

        let base_ty = array_base(tok1.ty.as_ref().unwrap());

        let mut len = tok1.ty.as_ref().unwrap().array_len;
        let mut t = tok1.next.as_ref().unwrap();
        while t.kind == TokenKind::Str && (t.loc != tok2_loc || t.file_no != tok2_file_no) {
            len += t.ty.as_ref().unwrap().array_len - 1;
            t = t.next.as_ref().unwrap();
        }

        let mut buf: Vec<u8> = Vec::new();
        if let Some(ref s) = tok1.str {
            buf.extend_from_slice(s);
        }
        let mut t = *tok1.next.as_ref().unwrap().clone();
        while t.kind == TokenKind::Str && (t.loc != tok2_loc || t.file_no != tok2_file_no) {
            if let Some(ref s) = t.str {
                buf.extend_from_slice(s);
            }
            t = *t.next.as_ref().unwrap().clone();
        }

        tok1.str = Some(buf);
        tok1.ty = Some(crate::Type::new_array(base_ty, len));
        tok1.next = Some(Box::new(t));
    }
}

fn join_adjacent_string_literals(files: &[File], tok: &mut Token) -> Result<(), String> {
    widen_adjacent_string_literals(files, tok)?;
    merge_adjacent_string_literals(tok);
    Ok(())
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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
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

fn search_include_paths(filename: &str) -> Option<String> {
    if filename.starts_with('/') {
        return Some(filename.to_string());
    }

    let paths = get_include_paths();
    for dir in &paths {
        let path = Path::new(dir).join(filename);
        if path.exists() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

fn read_include_filename(tok: &Token, is_dquote: &mut bool) -> Result<(String, Token), String> {
    let files = get_input_files();

    // Pattern 1: #include "foo.h"
    if tok.kind == TokenKind::Str {
        let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
        let filename: String = file
            .contents
            .chars()
            .skip(tok.loc + 1)
            .take(tok.len - 2)
            .collect();
        *is_dquote = true;
        let rest = skip_line(&files, *tok.next.as_ref().unwrap().clone());
        return Ok((filename, rest));
    }

    // Pattern 2: #include <foo.h>
    if equal(&files, tok, "<") {
        let start = tok.clone();
        let mut tok = *tok.next.as_ref().unwrap().clone();

        // Find closing ">"
        while !equal(&files, &tok, ">") {
            if tok.at_bol || tok.kind == TokenKind::Eof {
                return Err(error_tok(&files, &tok, "expected '>'"));
            }
            tok = *tok.next.as_ref().unwrap().clone();
        }

        let end = &tok;
        *is_dquote = false;
        let files = get_input_files();
        let filename = join_tokens(&files, start.next.as_ref().unwrap(), Some(end));
        let rest = skip_line(&files, *tok.next.as_ref().unwrap().clone());
        return Ok((filename, rest));
    }

    // Pattern 3: #include FOO
    // In this case FOO must be macro-expanded to either
    // a single string token or a sequence of "<" ... ">".
    if tok.kind == TokenKind::Ident {
        let files = get_input_files();
        let (line, rest) = copy_line(&files, tok);
        let tok2 = preprocess2(&[], line)?;
        return read_include_filename(&tok2, is_dquote).map(|(f, _)| (f, rest));
    }

    let files = get_input_files();
    Err(error_tok(&files, tok, "expected a filename"))
}

fn str_payload_from_literal(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn apply_line_marker_adjustments(tok: &mut Token) {
    let mut cur = tok;
    loop {
        cur.line_no = (cur.line_no as i64 + cur.line_delta) as usize;
        if cur.next.is_none() {
            break;
        }
        cur = cur.next.as_mut().unwrap();
    }
}

fn read_line_marker(files: &[File], first_arg: &Token) -> Result<Token, String> {
    let start = first_arg.clone();
    let (line_body, rest) = copy_line(files, first_arg);

    let mut expanded = preprocess2(&[], line_body)?;
    let files_now = get_input_files();
    convert_pp_tokens(&files_now, &mut expanded);
    join_adjacent_string_literals(&files_now, &mut expanded)?;

    if expanded.kind != TokenKind::Num {
        return Err(error_tok(files, &start, "invalid line marker"));
    }
    if !matches!(expanded.ty.as_ref(), Some(ty) if is_integer(ty)) {
        return Err(error_tok(files, &expanded, "invalid line marker"));
    }

    let delta = expanded.val - start.line_no as i64;
    let after_num = expanded.next.as_deref();

    match after_num {
        Some(next) if next.kind != TokenKind::Eof => {
            let bytes = match (next.kind, next.str.as_deref()) {
                (TokenKind::Str, Some(b)) => b,
                _ => return Err(error_tok(files, next, "filename expected")),
            };
            update_file_line_marker(start.file_no, delta, Some(str_payload_from_literal(bytes)));
        }
        _ => update_file_line_marker(start.file_no, delta, None),
    }

    Ok(rest)
}

fn include_file(tok: Token, path: &str, filename_tok: &Token) -> Result<Token, String> {
    match tokenize_file(path) {
        Some(tok2) => Ok(append(tok2, tok)),
        None => Err(error_tok(
            &get_input_files(),
            filename_tok,
            &format!(
                "{}: cannot open file: {}",
                path,
                std::io::Error::new(std::io::ErrorKind::NotFound, "file not found")
            ),
        )),
    }
}

fn preprocess2(_files: &[File], tok: Token) -> Result<Token, String> {
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
        line_delta: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
    };
    let mut cur = &mut head;
    let mut tok = tok;

    loop {
        let files = get_input_files();

        if tok.kind == TokenKind::Eof {
            break;
        }

        if let Some(expanded) = expand_macro(&files, &tok) {
            tok = expanded;
            continue;
        }

        if !is_hash(&files, &tok) {
            tok.line_delta = line_delta_for_file(tok.file_no);
            let next = tok.next.take();
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            tok = *next.unwrap();
            continue;
        }

        let start = tok.clone();
        tok = *tok.next.unwrap();

        if token_str_eq(&files, &tok, "include") {
            let mut is_dquote = false;
            let (filename, new_tok) =
                read_include_filename(tok.next.as_ref().unwrap(), &mut is_dquote)?;
            tok = new_tok;

            let filename_tok = start.next.as_ref().unwrap().clone();

            if !filename.starts_with('/') && is_dquote {
                let current_file = files.iter().find(|f| f.file_no == start.file_no).unwrap();
                let current_path = Path::new(&current_file.name);
                let dir = current_path.parent().unwrap_or(Path::new("."));
                let path = dir.join(&filename).to_string_lossy().into_owned();
                if Path::new(&path).exists() {
                    tok = include_file(tok, &path, &filename_tok)?;
                    continue;
                }
            }

            let path = search_include_paths(&filename);
            tok = include_file(tok, path.as_deref().unwrap_or(&filename), &filename_tok)?;
            continue;
        }

        if token_str_eq(&files, &tok, "if") {
            let (val, new_tok) = eval_const_expr(&files, &tok)?;
            tok = new_tok;
            push_cond_incl(&start, val != 0);
            if val == 0 {
                tok = skip_cond_incl(&get_input_files(), tok);
            }
            continue;
        }

        if token_str_eq(&files, &tok, "ifdef") {
            let defined = find_macro(&get_input_files(), tok.next.as_ref().unwrap()).is_some();
            push_cond_incl(&start, defined);
            tok = skip_line(&get_input_files(), *tok.next.unwrap().next.unwrap());
            if !defined {
                tok = skip_cond_incl(&get_input_files(), tok);
            }
            continue;
        }

        if token_str_eq(&files, &tok, "ifndef") {
            let defined = find_macro(&get_input_files(), tok.next.as_ref().unwrap()).is_some();
            push_cond_incl(&start, !defined);
            tok = skip_line(&get_input_files(), *tok.next.unwrap().next.unwrap());
            if defined {
                tok = skip_cond_incl(&get_input_files(), tok);
            }
            continue;
        }

        if token_str_eq(&files, &tok, "elif") {
            let ci = cond_incl_get();
            if ci.is_none() || ci.as_ref().unwrap().ctx == CondInclCtx::Else {
                return Err(error_tok(&get_input_files(), &start, "stray #elif"));
            }
            let mut ci = ci.unwrap();
            ci.ctx = CondInclCtx::Elif;
            cond_incl_set(Some(ci.clone()));

            if !ci.included {
                let (val, new_tok) = eval_const_expr(&get_input_files(), &tok)?;
                tok = new_tok;
                if val != 0 {
                    ci.included = true;
                    cond_incl_set(Some(ci));
                } else {
                    tok = skip_cond_incl(&get_input_files(), tok);
                }
            } else {
                tok = skip_cond_incl(&get_input_files(), tok);
            }
            continue;
        }

        if token_str_eq(&files, &tok, "else") {
            let ci = cond_incl_get();
            if ci.is_none() || ci.as_ref().unwrap().ctx == CondInclCtx::Else {
                return Err(error_tok(&get_input_files(), &start, "stray #else"));
            }
            let mut ci = ci.unwrap();
            ci.ctx = CondInclCtx::Else;
            cond_incl_set(Some(ci));
            tok = skip_line(&get_input_files(), *tok.next.unwrap());

            let ci = cond_incl_get().unwrap();
            if ci.included {
                tok = skip_cond_incl(&get_input_files(), tok);
            }
            cond_incl_set(Some(ci));
            continue;
        }

        if token_str_eq(&files, &tok, "endif") {
            let ci = cond_incl_get();
            if ci.is_none() {
                return Err(error_tok(&get_input_files(), &start, "stray #endif"));
            }
            cond_incl_set(ci.unwrap().next);
            tok = skip_line(&get_input_files(), *tok.next.unwrap());
            continue;
        }

        if token_str_eq(&files, &tok, "define") {
            tok = read_macro_definition(&get_input_files(), tok.next.as_ref().unwrap())?;
            continue;
        }

        if token_str_eq(&files, &tok, "undef") {
            tok = *tok.next.unwrap();
            if tok.kind != TokenKind::Ident {
                return Err(error_tok(
                    &get_input_files(),
                    &tok,
                    "macro name must be an identifier",
                ));
            }
            let files = get_input_files();
            let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
            let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();
            tok = skip_line(&files, *tok.next.unwrap());
            undef_macro(&name);
            continue;
        }

        if token_str_eq(&files, &tok, "line") {
            let after_line = tok.next.as_ref().unwrap();
            tok = read_line_marker(&files, after_line)?;
            continue;
        }

        if token_str_eq(&files, &tok, "error") {
            return Err(error_tok(&get_input_files(), &tok, "error"));
        }

        if tok.at_bol {
            continue;
        }

        return Err(error_tok(
            &get_input_files(),
            &tok,
            "invalid preprocessor directive",
        ));
    }

    cur.next = Some(Box::new(new_eof(&tok)));

    Ok(*head.next.unwrap())
}

pub fn preprocess(tok: Token) -> Result<Token, String> {
    let files = get_input_files();
    let tok = preprocess2(&[], tok)?;
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
    convert_pp_tokens(&files, &mut tok);
    join_adjacent_string_literals(&files, &mut tok)?;
    apply_line_marker_adjustments(&mut tok);
    Ok(tok)
}

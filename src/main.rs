use std::{
    collections::HashMap,
    env, fs,
    io::{self, Read},
    process,
};

fn usage(bin: &str) -> String {
    format!("Usage: {bin} <filename>")
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

fn error_at(filename: &str, src: &str, loc: usize, msg: &str) -> String {
    let mut line_start = loc;
    while line_start > 0 && src.as_bytes()[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    let mut line_end = loc;
    while line_end < src.len() && src.as_bytes()[line_end] != b'\n' {
        line_end += 1;
    }

    let line_no = src[..loc].matches('\n').count() + 1;
    let line = &src[line_start..line_end];

    let indent = format!("{filename}:{line_no}: ").len();
    let pos = loc - line_start + indent;

    format!(
        "{filename}:{line_no}: {line}\n{:width$}^ {msg}\n",
        "",
        width = pos
    )
}

fn error_tok(filename: &str, src: &str, tok: &Token, msg: &str) -> String {
    error_at(filename, src, tok.loc, msg)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Ident,
    Punct,
    Keyword,
    Num,
    Eof,
}

#[derive(Debug, Clone)]
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

fn read_punct(chars: &[char], pos: usize) -> Option<usize> {
    let remaining: String = chars[pos..].iter().collect();
    if remaining.starts_with("==")
        || remaining.starts_with("!=")
        || remaining.starts_with("<=")
        || remaining.starts_with(">=")
    {
        return Some(2);
    }
    if chars[pos].is_ascii_punctuation() {
        return Some(1);
    }
    None
}

fn is_keyword(name: &str) -> bool {
    matches!(name, "return" | "if" | "else" | "for" | "while")
}

fn convert_keywords(src: &str, tok: &mut Token) {
    let mut cur = tok;
    loop {
        if cur.kind == TokenKind::Ident {
            let name: String = src.chars().skip(cur.loc).take(cur.len).collect();
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

fn tokenize(filename: &str, src: &str) -> Result<Token, String> {
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

        if chars[pos].is_ascii_alphabetic() || chars[pos] == '_' {
            let start = pos;
            while pos < chars.len() && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_') {
                pos += 1;
            }
            let tok = new_token(TokenKind::Ident, start, pos);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            continue;
        }

        if let Some(len) = read_punct(&chars, pos) {
            let tok = new_token(TokenKind::Punct, pos, pos + len);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            pos += len;
            continue;
        }

        return Err(error_at(filename, src, pos, "invalid token"));
    }

    cur.next = Some(Box::new(new_token(TokenKind::Eof, pos, pos)));
    let mut tok = head.next.unwrap();
    convert_keywords(src, &mut tok);
    Ok(*tok)
}

fn equal(src: &str, tok: &Token, s: &str) -> bool {
    (tok.kind == TokenKind::Punct || tok.kind == TokenKind::Keyword)
        && tok.len == s.len()
        && src.chars().skip(tok.loc).take(tok.len).eq(s.chars())
}

fn skip(filename: &str, src: &str, tok: &Token, s: &str) -> Result<Token, String> {
    if equal(src, tok, s) {
        return Ok(*tok.next.as_ref().unwrap().clone());
    }
    Err(error_tok(filename, src, tok, &format!("expected '{s}'")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Eq,
    Ne,
    Lt,
    Le,
    Assign,
    Addr,
    Deref,
    Return,
    If,
    For,
    While,
    Block,
    ExprStmt,
    Var,
    Num,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeKind {
    Int,
    Ptr,
}

#[derive(Debug, Clone)]
struct Type {
    kind: TypeKind,
    base: Option<Box<Type>>,
}

impl Type {
    fn new_int() -> Type {
        Type {
            kind: TypeKind::Int,
            base: None,
        }
    }

    fn new_ptr(base: Type) -> Type {
        Type {
            kind: TypeKind::Ptr,
            base: Some(Box::new(base)),
        }
    }
}

fn is_integer(ty: &Type) -> bool {
    ty.kind == TypeKind::Int
}

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    tok_loc: usize,
    ty: Option<Type>,
    next: Option<Box<Node>>,
    lhs: Option<Box<Node>>,
    rhs: Option<Box<Node>>,
    cond: Option<Box<Node>>,
    then: Option<Box<Node>>,
    els: Option<Box<Node>>,
    init: Option<Box<Node>>,
    inc: Option<Box<Node>>,
    body: Option<Box<Node>>,
    varname: String,
    val: i64,
}

fn new_node(kind: NodeKind, tok_loc: usize) -> Node {
    Node {
        kind,
        tok_loc,
        ty: None,
        next: None,
        lhs: None,
        rhs: None,
        cond: None,
        then: None,
        els: None,
        init: None,
        inc: None,
        body: None,
        varname: String::new(),
        val: 0,
    }
}

fn new_binary(kind: NodeKind, lhs: Node, rhs: Node, tok_loc: usize) -> Node {
    let mut node = new_node(kind, tok_loc);
    node.lhs = Some(Box::new(lhs));
    node.rhs = Some(Box::new(rhs));
    node
}

fn new_unary(kind: NodeKind, expr: Node, tok_loc: usize) -> Node {
    let mut node = new_node(kind, tok_loc);
    node.lhs = Some(Box::new(expr));
    node
}

fn new_num(val: i64, tok_loc: usize) -> Node {
    let mut node = new_node(NodeKind::Num, tok_loc);
    node.val = val;
    node
}

fn new_var_node(varname: String, tok_loc: usize) -> Node {
    let mut node = new_node(NodeKind::Var, tok_loc);
    node.varname = varname;
    node
}

fn compound_stmt(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let tok_loc = tok.loc;
    let mut head = Node {
        kind: NodeKind::Num,
        tok_loc,
        ty: None,
        next: None,
        lhs: None,
        rhs: None,
        cond: None,
        then: None,
        els: None,
        init: None,
        inc: None,
        body: None,
        varname: String::new(),
        val: 0,
    };
    let mut cur = &mut head;

    let mut tok = tok.clone();
    while !equal(src, &tok, "}") {
        let (node, new_tok) = stmt(filename, src, &tok)?;
        tok = new_tok;
        cur.next = Some(Box::new(node));
        cur = cur.next.as_mut().unwrap();
    }

    let mut node = new_node(NodeKind::Block, tok_loc);
    node.body = head.next;
    Ok((node, *tok.next.as_ref().unwrap().clone()))
}

fn stmt(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    if equal(src, tok, "return") {
        let tok_loc = tok.loc;
        let (expr_node, tok) = expr(filename, src, tok.next.as_ref().unwrap())?;
        let tok = skip(filename, src, &tok, ";")?;
        let node = new_unary(NodeKind::Return, expr_node, tok_loc);
        return Ok((node, tok));
    }
    if equal(src, tok, "if") {
        let tok_loc = tok.loc;
        let mut node = new_node(NodeKind::If, tok_loc);
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(filename, src, &tok)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;
        let (then, tok) = stmt(filename, src, &tok)?;
        node.then = Some(Box::new(then));
        let mut tok = tok;
        if equal(src, &tok, "else") {
            let (els, new_tok) = stmt(filename, src, tok.next.as_ref().unwrap())?;
            node.els = Some(Box::new(els));
            tok = new_tok;
        }
        return Ok((node, tok));
    }
    if equal(src, tok, "for") {
        let tok_loc = tok.loc;
        let mut node = new_node(NodeKind::For, tok_loc);
        let mut tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;

        let (init, new_tok) = expr_stmt(filename, src, &tok)?;
        node.init = Some(Box::new(init));
        tok = new_tok;

        if !equal(src, &tok, ";") {
            let (cond, new_tok) = expr(filename, src, &tok)?;
            node.cond = Some(Box::new(cond));
            tok = new_tok;
        }
        tok = skip(filename, src, &tok, ";")?;

        if !equal(src, &tok, ")") {
            let (inc, new_tok) = expr(filename, src, &tok)?;
            node.inc = Some(Box::new(inc));
            tok = new_tok;
        }
        tok = skip(filename, src, &tok, ")")?;

        let (then, tok) = stmt(filename, src, &tok)?;
        node.then = Some(Box::new(then));
        return Ok((node, tok));
    }
    if equal(src, tok, "while") {
        let tok_loc = tok.loc;
        let mut node = new_node(NodeKind::While, tok_loc);
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(filename, src, &tok)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;
        let (then, tok) = stmt(filename, src, &tok)?;
        node.then = Some(Box::new(then));
        return Ok((node, tok));
    }
    if equal(src, tok, "{") {
        return compound_stmt(filename, src, tok.next.as_ref().unwrap());
    }
    expr_stmt(filename, src, tok)
}

fn expr_stmt(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    if equal(src, tok, ";") {
        let tok_loc = tok.loc;
        let tok = *tok.next.as_ref().unwrap().clone();
        return Ok((new_node(NodeKind::Block, tok_loc), tok));
    }
    let tok_loc = tok.loc;
    let (expr_node, tok) = expr(filename, src, tok)?;
    let tok = skip(filename, src, &tok, ";")?;
    let node = new_unary(NodeKind::ExprStmt, expr_node, tok_loc);
    Ok((node, tok))
}

fn expr(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    assign(filename, src, tok)
}

fn assign(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, tok) = equality(filename, src, tok)?;
    if equal(src, &tok, "=") {
        let tok_loc = tok.loc;
        let (rhs, tok) = assign(filename, src, tok.next.as_ref().unwrap())?;
        node = new_binary(NodeKind::Assign, node, rhs, tok_loc);
        return Ok((node, tok));
    }
    Ok((node, tok))
}

fn equality(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = relational(filename, src, tok)?;

    loop {
        if equal(src, &tok, "==") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = relational(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Eq, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "!=") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = relational(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Ne, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn relational(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = add(filename, src, tok)?;

    loop {
        if equal(src, &tok, "<") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Lt, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "<=") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Le, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">") {
            let tok_loc = tok.loc;
            let (lhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Lt, lhs, node, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">=") {
            let tok_loc = tok.loc;
            let (lhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Le, lhs, node, tok_loc);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn add(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = mul(filename, src, tok)?;

    loop {
        if equal(src, &tok, "+") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = mul(filename, src, tok.next.as_ref().unwrap())?;
            node = new_add(node, rhs, tok_loc, filename, src)?;
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "-") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = mul(filename, src, tok.next.as_ref().unwrap())?;
            node = new_sub(node, rhs, tok_loc, filename, src)?;
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn mul(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = unary(filename, src, tok)?;

    loop {
        if equal(src, &tok, "*") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Mul, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "/") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Div, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn unary(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    if equal(src, tok, "+") {
        return unary(filename, src, tok.next.as_ref().unwrap());
    }

    if equal(src, tok, "-") {
        let tok_loc = tok.loc;
        let (node, tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
        return Ok((new_unary(NodeKind::Neg, node, tok_loc), tok));
    }

    if equal(src, tok, "&") {
        let tok_loc = tok.loc;
        let (node, tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
        return Ok((new_unary(NodeKind::Addr, node, tok_loc), tok));
    }

    if equal(src, tok, "*") {
        let tok_loc = tok.loc;
        let (node, tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
        return Ok((new_unary(NodeKind::Deref, node, tok_loc), tok));
    }

    primary(filename, src, tok)
}

fn primary(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    if equal(src, tok, "(") {
        let (node, tok) = expr(filename, src, tok.next.as_ref().unwrap())?;
        let tok = skip(filename, src, &tok, ")")?;
        return Ok((node, tok));
    }

    if tok.kind == TokenKind::Ident {
        let tok_loc = tok.loc;
        let varname: String = src.chars().skip(tok.loc).take(tok.len).collect();
        let node = new_var_node(varname, tok_loc);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    if tok.kind == TokenKind::Num {
        let tok_loc = tok.loc;
        let node = new_num(tok.val, tok_loc);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    Err(error_tok(filename, src, tok, "expected an expression"))
}

fn gen_addr(
    node: &Node,
    var_offsets: &HashMap<String, i64>,
    result: &mut String,
    filename: &str,
    src: &str,
) -> Result<(), String> {
    match node.kind {
        NodeKind::Var => {
            let offset = var_offsets.get(&node.varname).unwrap();
            result.push_str(&format!("  lea -{}(%rbp), %rax\n", offset));
        }
        NodeKind::Deref => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
        }
        _ => return Err(error_at(filename, src, node.tok_loc, "not an lvalue")),
    }
    Ok(())
}

fn gen_expr(
    node: &Node,
    var_offsets: &HashMap<String, i64>,
    result: &mut String,
    filename: &str,
    src: &str,
) -> Result<(), String> {
    match node.kind {
        NodeKind::Num => {
            result.push_str(&format!("  mov ${}, %rax\n", node.val));
            return Ok(());
        }
        NodeKind::Neg => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str("  neg %rax\n");
            return Ok(());
        }
        NodeKind::Var => {
            gen_addr(node, var_offsets, result, filename, src)?;
            result.push_str("  mov (%rax), %rax\n");
            return Ok(());
        }
        NodeKind::Addr => {
            gen_addr(
                node.lhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            return Ok(());
        }
        NodeKind::Deref => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str("  mov (%rax), %rax\n");
            return Ok(());
        }
        NodeKind::Assign => {
            gen_addr(
                node.lhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str("  push %rax\n");
            gen_expr(
                node.rhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str("  pop %rdi\n");
            result.push_str("  mov %rax, (%rdi)\n");
            return Ok(());
        }
        _ => {}
    }

    gen_expr(
        node.rhs.as_ref().unwrap(),
        var_offsets,
        result,
        filename,
        src,
    )?;
    result.push_str("  push %rax\n");
    gen_expr(
        node.lhs.as_ref().unwrap(),
        var_offsets,
        result,
        filename,
        src,
    )?;
    result.push_str("  pop %rdi\n");

    match node.kind {
        NodeKind::Add => result.push_str("  add %rdi, %rax\n"),
        NodeKind::Sub => result.push_str("  sub %rdi, %rax\n"),
        NodeKind::Mul => result.push_str("  imul %rdi, %rax\n"),
        NodeKind::Div => {
            result.push_str("  cqo\n");
            result.push_str("  idiv %rdi\n");
        }
        NodeKind::Eq | NodeKind::Ne | NodeKind::Lt | NodeKind::Le => {
            result.push_str("  cmp %rdi, %rax\n");
            match node.kind {
                NodeKind::Eq => result.push_str("  sete %al\n"),
                NodeKind::Ne => result.push_str("  setne %al\n"),
                NodeKind::Lt => result.push_str("  setl %al\n"),
                NodeKind::Le => result.push_str("  setle %al\n"),
                _ => unreachable!(),
            }
            result.push_str("  movzb %al, %rax\n");
        }
        NodeKind::Neg
        | NodeKind::Num
        | NodeKind::ExprStmt
        | NodeKind::Var
        | NodeKind::Assign
        | NodeKind::Addr
        | NodeKind::Deref
        | NodeKind::Return
        | NodeKind::Block
        | NodeKind::If
        | NodeKind::For
        | NodeKind::While => unreachable!(),
    }
    Ok(())
}

static mut LABEL_COUNT: i32 = 0;

fn count() -> i32 {
    unsafe {
        LABEL_COUNT += 1;
        LABEL_COUNT
    }
}

fn gen_stmt(
    node: &Node,
    var_offsets: &HashMap<String, i64>,
    result: &mut String,
    filename: &str,
    src: &str,
) -> Result<(), String> {
    match node.kind {
        NodeKind::If => {
            let c = count();
            gen_expr(
                node.cond.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str("  cmp $0, %rax\n");
            result.push_str(&format!("  je .L.else.{}\n", c));
            gen_stmt(
                node.then.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str(&format!("  jmp .L.end.{}\n", c));
            result.push_str(&format!(".L.else.{}:\n", c));
            if let Some(els) = node.els.as_ref() {
                gen_stmt(els, var_offsets, result, filename, src)?;
            }
            result.push_str(&format!(".L.end.{}:\n", c));
        }
        NodeKind::For => {
            let c = count();
            gen_stmt(
                node.init.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str(&format!(".L.begin.{}:\n", c));
            if let Some(cond) = node.cond.as_ref() {
                gen_expr(cond, var_offsets, result, filename, src)?;
                result.push_str("  cmp $0, %rax\n");
                result.push_str(&format!("  je .L.end.{}\n", c));
            }
            gen_stmt(
                node.then.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            if let Some(inc) = node.inc.as_ref() {
                gen_expr(inc, var_offsets, result, filename, src)?;
            }
            result.push_str(&format!("  jmp .L.begin.{}\n", c));
            result.push_str(&format!(".L.end.{}:\n", c));
        }
        NodeKind::While => {
            let c = count();
            result.push_str(&format!(".L.begin.{}:\n", c));
            gen_expr(
                node.cond.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str("  cmp $0, %rax\n");
            result.push_str(&format!("  je .L.end.{}\n", c));
            gen_stmt(
                node.then.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str(&format!("  jmp .L.begin.{}\n", c));
            result.push_str(&format!(".L.end.{}:\n", c));
        }
        NodeKind::Block => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                gen_stmt(stmt_node, var_offsets, result, filename, src)?;
                n = stmt_node.next.as_ref();
            }
        }
        NodeKind::Return => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
            result.push_str("  jmp .L.return\n");
        }
        NodeKind::ExprStmt => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                var_offsets,
                result,
                filename,
                src,
            )?;
        }
        _ => return Err(error_at(filename, src, node.tok_loc, "invalid statement")),
    }
    Ok(())
}

fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
}

fn collect_var_names(node: &Node, var_names: &mut Vec<String>) {
    match node.kind {
        NodeKind::Var => {
            if !var_names.contains(&node.varname) {
                var_names.push(node.varname.clone());
            }
        }
        NodeKind::Num => {}
        NodeKind::If => {
            collect_var_names(node.cond.as_ref().unwrap(), var_names);
            collect_var_names(node.then.as_ref().unwrap(), var_names);
            if let Some(els) = node.els.as_ref() {
                collect_var_names(els, var_names);
            }
        }
        NodeKind::For => {
            collect_var_names(node.init.as_ref().unwrap(), var_names);
            if let Some(cond) = node.cond.as_ref() {
                collect_var_names(cond, var_names);
            }
            collect_var_names(node.then.as_ref().unwrap(), var_names);
            if let Some(inc) = node.inc.as_ref() {
                collect_var_names(inc, var_names);
            }
        }
        NodeKind::While => {
            collect_var_names(node.cond.as_ref().unwrap(), var_names);
            collect_var_names(node.then.as_ref().unwrap(), var_names);
        }
        NodeKind::Block => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                collect_var_names(stmt_node, var_names);
                n = stmt_node.next.as_ref();
            }
        }
        NodeKind::Neg
        | NodeKind::ExprStmt
        | NodeKind::Return
        | NodeKind::Addr
        | NodeKind::Deref => {
            collect_var_names(node.lhs.as_ref().unwrap(), var_names);
        }
        NodeKind::Assign
        | NodeKind::Add
        | NodeKind::Sub
        | NodeKind::Mul
        | NodeKind::Div
        | NodeKind::Eq
        | NodeKind::Ne
        | NodeKind::Lt
        | NodeKind::Le => {
            collect_var_names(node.lhs.as_ref().unwrap(), var_names);
            collect_var_names(node.rhs.as_ref().unwrap(), var_names);
        }
    }
}

fn add_type(node: &mut Node) {
    if node.ty.is_some() {
        return;
    }

    if let Some(lhs) = &mut node.lhs {
        add_type(lhs);
    }
    if let Some(rhs) = &mut node.rhs {
        add_type(rhs);
    }
    if let Some(cond) = &mut node.cond {
        add_type(cond);
    }
    if let Some(then) = &mut node.then {
        add_type(then);
    }
    if let Some(els) = &mut node.els {
        add_type(els);
    }
    if let Some(init) = &mut node.init {
        add_type(init);
    }
    if let Some(inc) = &mut node.inc {
        add_type(inc);
    }

    if let Some(body) = &mut node.body {
        let mut n = body;
        loop {
            add_type(n);
            if let Some(next) = &mut n.next {
                n = next;
            } else {
                break;
            }
        }
    }

    match node.kind {
        NodeKind::Add | NodeKind::Sub | NodeKind::Mul | NodeKind::Div | NodeKind::Neg => {
            node.ty = node.lhs.as_ref().unwrap().ty.clone();
        }
        NodeKind::Assign => {
            node.ty = node.lhs.as_ref().unwrap().ty.clone();
        }
        NodeKind::Eq
        | NodeKind::Ne
        | NodeKind::Lt
        | NodeKind::Le
        | NodeKind::Num
        | NodeKind::Var => {
            node.ty = Some(Type::new_int());
        }
        NodeKind::Addr => {
            if let Some(lhs_ty) = &node.lhs.as_ref().unwrap().ty {
                node.ty = Some(Type::new_ptr(lhs_ty.clone()));
            } else {
                node.ty = Some(Type::new_int());
            }
        }
        NodeKind::Deref => {
            if let Some(lhs_ty) = &node.lhs.as_ref().unwrap().ty {
                if lhs_ty.kind == TypeKind::Ptr {
                    node.ty = Some(lhs_ty.base.as_ref().unwrap().as_ref().clone());
                } else {
                    node.ty = Some(Type::new_int());
                }
            } else {
                node.ty = Some(Type::new_int());
            }
        }
        NodeKind::Return
        | NodeKind::If
        | NodeKind::For
        | NodeKind::While
        | NodeKind::Block
        | NodeKind::ExprStmt => {}
    }
}

fn new_add(
    lhs: Node,
    rhs: Node,
    tok_loc: usize,
    filename: &str,
    src: &str,
) -> Result<Node, String> {
    let mut lhs = lhs;
    let mut rhs = rhs;
    add_type(&mut lhs);
    add_type(&mut rhs);

    let lhs_ty = lhs.ty.as_ref().unwrap();
    let rhs_ty = rhs.ty.as_ref().unwrap();

    if is_integer(lhs_ty) && is_integer(rhs_ty) {
        return Ok(new_binary(NodeKind::Add, lhs, rhs, tok_loc));
    }

    if lhs_ty.kind == TypeKind::Ptr && rhs_ty.kind == TypeKind::Ptr {
        return Err(error_at(filename, src, tok_loc, "invalid operands"));
    }

    if lhs_ty.kind != TypeKind::Ptr && rhs_ty.kind == TypeKind::Ptr {
        std::mem::swap(&mut lhs, &mut rhs);
    }

    let rhs = new_binary(NodeKind::Mul, rhs, new_num(8, tok_loc), tok_loc);
    Ok(new_binary(NodeKind::Add, lhs, rhs, tok_loc))
}

fn new_sub(
    lhs: Node,
    rhs: Node,
    tok_loc: usize,
    filename: &str,
    src: &str,
) -> Result<Node, String> {
    let mut lhs = lhs;
    let mut rhs = rhs;
    add_type(&mut lhs);
    add_type(&mut rhs);

    let lhs_ty = lhs.ty.as_ref().unwrap();
    let rhs_ty = rhs.ty.as_ref().unwrap();

    if is_integer(lhs_ty) && is_integer(rhs_ty) {
        return Ok(new_binary(NodeKind::Sub, lhs, rhs, tok_loc));
    }

    if lhs_ty.kind == TypeKind::Ptr && is_integer(rhs_ty) {
        let lhs_ty = lhs.ty.clone();
        let rhs = new_binary(NodeKind::Mul, rhs, new_num(8, tok_loc), tok_loc);
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc);
        node.ty = lhs_ty;
        return Ok(node);
    }

    if lhs_ty.kind == TypeKind::Ptr && rhs_ty.kind == TypeKind::Ptr {
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc);
        node.ty = Some(Type::new_int());
        let mut result = new_binary(NodeKind::Div, node, new_num(8, tok_loc), tok_loc);
        result.ty = Some(Type::new_int());
        return Ok(result);
    }

    Err(error_at(filename, src, tok_loc, "invalid operands"))
}

fn emit_assembly(filename: &str, src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let tok = tokenize(filename, src)?;
    let tok = skip(filename, src, &tok, "{")?;
    let (mut prog, tok) = compound_stmt(filename, src, &tok)?;

    if tok.kind != TokenKind::Eof {
        return Err(error_tok(filename, src, &tok, "extra token"));
    }

    add_type(&mut prog);

    let mut var_names: Vec<String> = Vec::new();
    collect_var_names(&prog, &mut var_names);
    var_names.reverse();

    let mut var_offsets: HashMap<String, i64> = HashMap::new();
    for (i, name) in var_names.iter().enumerate() {
        let offset = ((i + 1) * 8) as i64;
        var_offsets.insert(name.clone(), offset);
    }

    let stack_size = align_to((var_offsets.len() * 8) as i64, 16);

    let mut result = String::new();
    result.push_str(".text\n");
    result.push_str(".globl main\n");
    result.push_str("main:\n");

    result.push_str("  push %rbp\n");
    result.push_str("  mov %rsp, %rbp\n");
    result.push_str(&format!("  sub ${}, %rsp\n", stack_size));

    gen_stmt(&prog, &var_offsets, &mut result, filename, src)?;

    result.push_str(".L.return:\n");
    result.push_str("  mov %rbp, %rsp\n");
    result.push_str("  pop %rbp\n");
    result.push_str("  ret\n");
    Ok(result)
}

fn run() -> Result<String, String> {
    let mut args = env::args();
    let bin = args.next().unwrap_or_else(|| String::from("cc-rs"));
    let path = args.next().ok_or_else(|| usage(&bin))?;
    if args.next().is_some() {
        return Err(usage(&bin));
    }

    let (filename, src) = read_file(&path)?;
    emit_assembly(&filename, &src)
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

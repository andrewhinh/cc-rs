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

        if chars[pos].is_ascii_punctuation() {
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

fn skip(src: &str, tok: &Token, s: &str) -> Result<Token, String> {
    if equal(src, tok, s) {
        return Ok(*tok.next.as_ref().unwrap().clone());
    }
    Err(error_tok(src, tok, &format!("expected '{s}'")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Num,
}

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    lhs: Option<Box<Node>>,
    rhs: Option<Box<Node>>,
    val: i64,
}

fn new_node(kind: NodeKind) -> Node {
    Node {
        kind,
        lhs: None,
        rhs: None,
        val: 0,
    }
}

fn new_binary(kind: NodeKind, lhs: Node, rhs: Node) -> Node {
    let mut node = new_node(kind);
    node.lhs = Some(Box::new(lhs));
    node.rhs = Some(Box::new(rhs));
    node
}

fn new_unary(kind: NodeKind, expr: Node) -> Node {
    let mut node = new_node(kind);
    node.lhs = Some(Box::new(expr));
    node
}

fn new_num(val: i64) -> Node {
    let mut node = new_node(NodeKind::Num);
    node.val = val;
    node
}

fn expr(src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = mul(src, tok)?;

    loop {
        if equal(src, &tok, "+") {
            let (rhs, new_tok) = mul(src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Add, node, rhs);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "-") {
            let (rhs, new_tok) = mul(src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Sub, node, rhs);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn mul(src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = unary(src, tok)?;

    loop {
        if equal(src, &tok, "*") {
            let (rhs, new_tok) = unary(src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Mul, node, rhs);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "/") {
            let (rhs, new_tok) = unary(src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Div, node, rhs);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn unary(src: &str, tok: &Token) -> Result<(Node, Token), String> {
    if equal(src, tok, "+") {
        return unary(src, tok.next.as_ref().unwrap());
    }

    if equal(src, tok, "-") {
        let (node, tok) = unary(src, tok.next.as_ref().unwrap())?;
        return Ok((new_unary(NodeKind::Neg, node), tok));
    }

    primary(src, tok)
}

fn primary(src: &str, tok: &Token) -> Result<(Node, Token), String> {
    if equal(src, tok, "(") {
        let (node, tok) = expr(src, tok.next.as_ref().unwrap())?;
        let tok = skip(src, &tok, ")")?;
        return Ok((node, tok));
    }

    if tok.kind == TokenKind::Num {
        let node = new_num(tok.val);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    Err(error_tok(src, tok, "expected an expression"))
}

fn gen_expr(node: &Node, result: &mut String) {
    match node.kind {
        NodeKind::Num => {
            result.push_str(&format!("  mov ${}, %rax\n", node.val));
            return;
        }
        NodeKind::Neg => {
            gen_expr(node.lhs.as_ref().unwrap(), result);
            result.push_str("  neg %rax\n");
            return;
        }
        _ => {}
    }

    gen_expr(node.rhs.as_ref().unwrap(), result);
    result.push_str("  push %rax\n");
    gen_expr(node.lhs.as_ref().unwrap(), result);
    result.push_str("  pop %rdi\n");

    match node.kind {
        NodeKind::Add => result.push_str("  add %rdi, %rax\n"),
        NodeKind::Sub => result.push_str("  sub %rdi, %rax\n"),
        NodeKind::Mul => result.push_str("  imul %rdi, %rax\n"),
        NodeKind::Div => {
            result.push_str("  cqo\n");
            result.push_str("  idiv %rdi\n");
        }
        NodeKind::Neg | NodeKind::Num => unreachable!(),
    }
}

fn emit_assembly(src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let tok = tokenize(src)?;
    let (node, tok) = expr(src, &tok)?;

    if tok.kind != TokenKind::Eof {
        return Err(error_tok(src, &tok, "extra token"));
    }

    let mut result = String::new();
    result.push_str(".text\n");
    result.push_str(".globl main\n");
    result.push_str("main:\n");
    gen_expr(&node, &mut result);
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

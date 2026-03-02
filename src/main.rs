use std::{
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
    Ok(*head.next.unwrap())
}

fn equal(src: &str, tok: &Token, s: &str) -> bool {
    tok.kind == TokenKind::Punct
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
    ExprStmt,
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

fn stmt(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    expr_stmt(filename, src, tok)
}

fn expr_stmt(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (expr_node, tok) = expr(filename, src, tok)?;
    let tok = skip(filename, src, &tok, ";")?;
    let node = new_unary(NodeKind::ExprStmt, expr_node);
    Ok((node, tok))
}

fn expr(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    equality(filename, src, tok)
}

fn equality(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = relational(filename, src, tok)?;

    loop {
        if equal(src, &tok, "==") {
            let (rhs, new_tok) = relational(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Eq, node, rhs);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "!=") {
            let (rhs, new_tok) = relational(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Ne, node, rhs);
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
            let (rhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Lt, node, rhs);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "<=") {
            let (rhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Le, node, rhs);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">") {
            let (lhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Lt, lhs, node);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">=") {
            let (lhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Le, lhs, node);
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
            let (rhs, new_tok) = mul(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Add, node, rhs);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "-") {
            let (rhs, new_tok) = mul(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Sub, node, rhs);
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
            let (rhs, new_tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Mul, node, rhs);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "/") {
            let (rhs, new_tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
            node = new_binary(NodeKind::Div, node, rhs);
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
        let (node, tok) = unary(filename, src, tok.next.as_ref().unwrap())?;
        return Ok((new_unary(NodeKind::Neg, node), tok));
    }

    primary(filename, src, tok)
}

fn primary(filename: &str, src: &str, tok: &Token) -> Result<(Node, Token), String> {
    if equal(src, tok, "(") {
        let (node, tok) = expr(filename, src, tok.next.as_ref().unwrap())?;
        let tok = skip(filename, src, &tok, ")")?;
        return Ok((node, tok));
    }

    if tok.kind == TokenKind::Num {
        let node = new_num(tok.val);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    Err(error_tok(filename, src, tok, "expected an expression"))
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
        NodeKind::Neg | NodeKind::Num | NodeKind::ExprStmt => unreachable!(),
    }
}

fn gen_stmt(node: &Node, result: &mut String) {
    if node.kind == NodeKind::ExprStmt {
        gen_expr(node.lhs.as_ref().unwrap(), result);
        return;
    }
    panic!("invalid statement");
}

fn emit_assembly(filename: &str, src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let tok = tokenize(filename, src)?;

    let mut stmts: Vec<Node> = Vec::new();
    let mut tok = tok;

    while tok.kind != TokenKind::Eof {
        let (node, new_tok) = stmt(filename, src, &tok)?;
        tok = new_tok;
        stmts.push(node);
    }

    let mut result = String::new();
    result.push_str(".text\n");
    result.push_str(".globl main\n");
    result.push_str("main:\n");

    for node in &stmts {
        gen_stmt(node, &mut result);
    }

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

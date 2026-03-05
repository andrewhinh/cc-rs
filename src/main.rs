use std::{
    env, fs,
    io::{self, Read, Write},
    process,
};

struct Args {
    opt_o: Option<String>,
    input_path: String,
}

fn usage(status: i32) {
    eprintln!("Usage: cc-rs [ -o <path> ] <file>");
    process::exit(status);
}

fn parse_args() -> Args {
    let args: Vec<String> = env::args().collect();
    let mut opt_o: Option<String> = None;
    let mut input_path: Option<String> = None;
    let mut i = 1;

    while i < args.len() {
        if args[i] == "--help" {
            usage(0);
        }

        if args[i] == "-o" {
            i += 1;
            if i >= args.len() {
                usage(1);
            }
            opt_o = Some(args[i].clone());
            i += 1;
            continue;
        }

        if args[i].starts_with("-o") {
            opt_o = Some(args[i][2..].to_string());
            i += 1;
            continue;
        }

        if args[i].starts_with('-') && args[i].len() > 1 {
            eprintln!("unknown argument: {}", args[i]);
            process::exit(1);
        }

        input_path = Some(args[i].clone());
        i += 1;
    }

    let input_path = input_path.unwrap_or_else(|| {
        eprintln!("no input files");
        process::exit(1);
    });

    Args { opt_o, input_path }
}

fn open_output_file(path: Option<&String>) -> Box<dyn Write> {
    if path.is_none() || path.unwrap().as_str() == "-" {
        return Box::new(io::stdout());
    }

    let file = fs::File::create(path.unwrap()).expect("cannot open output file");
    Box::new(file)
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
    matches!(
        name,
        "return" | "if" | "else" | "for" | "while" | "int" | "sizeof"
    )
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

fn consume(src: &str, tok: &Token, s: &str) -> (bool, Token) {
    if equal(src, tok, s) {
        (true, *tok.next.as_ref().unwrap().clone())
    } else {
        (false, tok.clone())
    }
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
    FuncCall,
    ExprStmt,
    Var,
    Num,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeKind {
    Int,
    Ptr,
    Func,
    Array,
}

#[derive(Debug, Clone)]
struct Type {
    kind: TypeKind,
    size: i64,
    base: Option<Box<Type>>,
    name: Option<Box<Token>>,
    #[allow(unused)]
    return_ty: Option<Box<Type>>,
    params: Option<Box<Type>>,
    next: Option<Box<Type>>,
    #[allow(dead_code)]
    array_len: i64,
}

impl Type {
    fn new_int() -> Type {
        Type {
            kind: TypeKind::Int,
            size: 8,
            base: None,
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
        }
    }

    fn new_ptr(base: Type) -> Type {
        Type {
            kind: TypeKind::Ptr,
            size: 8,
            base: Some(Box::new(base)),
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
        }
    }

    fn new_array(base: Type, len: i64) -> Type {
        Type {
            kind: TypeKind::Array,
            size: base.size * len,
            base: Some(Box::new(base)),
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: len,
        }
    }
}

fn pointer_to(base: Type) -> Type {
    Type::new_ptr(base)
}

fn func_type(return_ty: Type) -> Type {
    Type {
        kind: TypeKind::Func,
        size: 0,
        base: None,
        name: None,
        return_ty: Some(Box::new(return_ty)),
        params: None,
        next: None,
        array_len: 0,
    }
}

fn is_integer(ty: &Type) -> bool {
    ty.kind == TypeKind::Int
}

fn copy_type(ty: &Type) -> Type {
    ty.clone()
}

#[derive(Debug, Clone)]
struct Obj {
    next: Option<Box<Obj>>,
    name: String,
    ty: Type,
    is_local: bool,
    offset: i64,
    is_function: bool,
    params: Vec<Obj>,
    body: Option<Box<Node>>,
    locals: Vec<Obj>,
    #[allow(dead_code)]
    stack_size: i64,
}

#[derive(Debug, Clone)]
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
    funcname: Option<String>,
    args: Option<Box<Node>>,
    var: Option<Box<Obj>>,
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
        funcname: None,
        args: None,
        var: None,
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

fn new_var_node(var: Obj, tok_loc: usize) -> Node {
    let mut node = new_node(NodeKind::Var, tok_loc);
    node.var = Some(Box::new(var.clone()));
    node.ty = Some(var.ty);
    node
}

fn find_var(locals: &[Obj], name: &str) -> Option<Obj> {
    for var in locals.iter().rev() {
        if var.name == name {
            return Some(var.clone());
        }
    }
    None
}

fn new_var(name: String, ty: Type) -> Obj {
    Obj {
        next: None,
        name,
        ty,
        is_local: false,
        offset: 0,
        is_function: false,
        params: Vec::new(),
        body: None,
        locals: Vec::new(),
        stack_size: 0,
    }
}

fn new_lvar(name: String, ty: Type, locals: &mut Vec<Obj>) -> Obj {
    let mut var = new_var(name, ty);
    var.is_local = true;
    let mut offset = 0;
    for v in locals.iter() {
        offset += v.ty.size;
    }
    offset += var.ty.size;
    var.offset = offset;
    locals.push(var.clone());
    var
}

fn new_gvar(name: String, ty: Type) -> Obj {
    let mut var = new_var(name, ty);
    var.is_local = false;
    var
}

fn get_ident(src: &str, tok: &Token) -> Result<String, String> {
    if tok.kind != TokenKind::Ident {
        return Err(error_tok("<stdin>", src, tok, "expected an identifier"));
    }
    let name: String = src.chars().skip(tok.loc).take(tok.len).collect();
    Ok(name)
}

fn declspec(filename: &str, src: &str, tok: &Token) -> Result<(Type, Token), String> {
    let tok = skip(filename, src, tok, "int")?;
    Ok((Type::new_int(), tok))
}

fn get_number(tok: &Token) -> Result<i64, String> {
    if tok.kind != TokenKind::Num {
        return Err("expected a number".to_string());
    }
    Ok(tok.val)
}

fn func_params(filename: &str, src: &str, tok: &Token, ty: Type) -> Result<(Type, Token), String> {
    let mut tok = tok.clone();

    let mut head = Type {
        kind: TypeKind::Int,
        size: 0,
        base: None,
        name: None,
        return_ty: None,
        params: None,
        next: None,
        array_len: 0,
    };
    let mut cur = &mut head;
    let mut first = true;

    while !equal(src, &tok, ")") {
        if !first {
            tok = skip(filename, src, &tok, ",")?;
        }
        first = false;

        let (basety, new_tok) = declspec(filename, src, &tok)?;
        tok = new_tok;
        let (param_ty, new_tok) = declarator(filename, src, &tok, basety)?;
        tok = new_tok;
        let param_copy = copy_type(&param_ty);
        cur.next = Some(Box::new(param_copy));
        cur = cur.next.as_mut().unwrap();
    }

    let mut func_ty = func_type(ty);
    func_ty.params = head.next;
    let rest = tok.next.as_ref().unwrap().as_ref().clone();
    Ok((func_ty, rest))
}

fn type_suffix(filename: &str, src: &str, tok: &Token, ty: Type) -> Result<(Type, Token), String> {
    if equal(src, tok, "(") {
        return func_params(filename, src, tok.next.as_ref().unwrap(), ty);
    }

    if equal(src, tok, "[") {
        let sz = get_number(tok.next.as_ref().unwrap())?;
        let tok = skip(
            filename,
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            "]",
        )?;
        let (ty, rest) = type_suffix(filename, src, &tok, ty)?;
        return Ok((Type::new_array(ty, sz), rest));
    }

    Ok((ty, tok.clone()))
}

fn declarator(
    filename: &str,
    src: &str,
    tok: &Token,
    mut ty: Type,
) -> Result<(Type, Token), String> {
    let mut tok = tok.clone();
    loop {
        let (consumed, new_tok) = consume(src, &tok, "*");
        if !consumed {
            break;
        }
        tok = new_tok;
        ty = pointer_to(ty);
    }

    if tok.kind != TokenKind::Ident {
        return Err(error_tok(filename, src, &tok, "expected a variable name"));
    }

    let name_tok = tok.clone();
    let (ty, tok) = type_suffix(filename, src, tok.next.as_ref().unwrap(), ty)?;
    let mut ty = ty;
    ty.name = Some(Box::new(name_tok));
    Ok((ty, tok))
}

fn declaration(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let (basety, mut tok) = declspec(filename, src, tok)?;

    let mut head = Node {
        kind: NodeKind::Num,
        tok_loc: tok.loc,
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
        funcname: None,
        args: None,
        var: None,
        val: 0,
    };
    let mut cur = &mut head;
    let mut i = 0;

    while !equal(src, &tok, ";") {
        if i > 0 {
            tok = skip(filename, src, &tok, ",")?;
        }
        i += 1;

        let (ty, new_tok) = declarator(filename, src, &tok, basety.clone())?;
        tok = new_tok;
        let name = get_ident(src, ty.name.as_ref().unwrap())?;
        let var = new_lvar(name, ty.clone(), locals);

        if !equal(src, &tok, "=") {
            continue;
        }

        let tok_loc = tok.loc;
        let tok_next = tok.next.as_ref().unwrap().clone();
        let (rhs, new_tok) = assign(filename, src, &tok_next, locals)?;
        tok = new_tok;
        let lhs = new_var_node(var, ty.name.as_ref().unwrap().loc);
        let node = new_binary(NodeKind::Assign, lhs, rhs, tok_loc);
        cur.next = Some(Box::new(new_unary(NodeKind::ExprStmt, node, tok_loc)));
        cur = cur.next.as_mut().unwrap();
    }

    let tok_loc = tok.loc;
    let mut node = new_node(NodeKind::Block, tok_loc);
    node.body = head.next;
    Ok((node, *tok.next.as_ref().unwrap().clone()))
}

fn create_param_lvars(src: &str, param: &Type, locals: &mut Vec<Obj>) {
    let mut current = Some(param);

    while let Some(p) = current {
        if let Some(name_tok) = &p.name {
            let name = get_ident(src, name_tok).unwrap();
            new_lvar(name, p.clone(), locals);
        }
        current = p.next.as_ref().map(|b| b.as_ref());
    }
}

fn function(filename: &str, src: &str, tok: &Token, basety: Type) -> Result<(Obj, Token), String> {
    let (ty, tok) = declarator(filename, src, tok, basety)?;
    let name = get_ident(src, ty.name.as_ref().unwrap())?;

    let mut fn_obj = new_gvar(name, ty.clone());
    fn_obj.is_function = true;

    let mut locals: Vec<Obj> = Vec::new();

    if let Some(params) = &ty.params {
        create_param_lvars(src, params, &mut locals);
    }

    fn_obj.params = locals.clone();

    let tok = skip(filename, src, &tok, "{")?;
    let (mut body, tok) = compound_stmt(filename, src, &tok, &mut locals)?;

    add_type(&mut body);

    fn_obj.body = Some(Box::new(body));
    fn_obj.locals = locals;

    Ok((fn_obj, tok))
}

fn compound_stmt(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
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
        funcname: None,
        args: None,
        var: None,
        val: 0,
    };
    let mut cur = &mut head;

    let mut tok = tok.clone();
    while !equal(src, &tok, "}") {
        if equal(src, &tok, "int") {
            let (node, new_tok) = declaration(filename, src, &tok, locals)?;
            tok = new_tok;
            cur.next = Some(Box::new(node));
            cur = cur.next.as_mut().unwrap();
        } else {
            let (node, new_tok) = stmt(filename, src, &tok, locals)?;
            tok = new_tok;
            cur.next = Some(Box::new(node));
            cur = cur.next.as_mut().unwrap();
        }
    }

    let mut node = new_node(NodeKind::Block, tok_loc);
    node.body = head.next;
    Ok((node, *tok.next.as_ref().unwrap().clone()))
}

fn stmt(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "return") {
        let tok_loc = tok.loc;
        let (expr_node, tok) = expr(filename, src, tok.next.as_ref().unwrap(), locals)?;
        let tok = skip(filename, src, &tok, ";")?;
        let node = new_unary(NodeKind::Return, expr_node, tok_loc);
        return Ok((node, tok));
    }
    if equal(src, tok, "if") {
        let tok_loc = tok.loc;
        let mut node = new_node(NodeKind::If, tok_loc);
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(filename, src, &tok, locals)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;
        let (then, tok) = stmt(filename, src, &tok, locals)?;
        node.then = Some(Box::new(then));
        let mut tok = tok;
        if equal(src, &tok, "else") {
            let (els, new_tok) = stmt(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node.els = Some(Box::new(els));
            tok = new_tok;
        }
        return Ok((node, tok));
    }
    if equal(src, tok, "for") {
        let tok_loc = tok.loc;
        let mut node = new_node(NodeKind::For, tok_loc);
        let mut tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;

        let (init, new_tok) = expr_stmt(filename, src, &tok, locals)?;
        node.init = Some(Box::new(init));
        tok = new_tok;

        if !equal(src, &tok, ";") {
            let (cond, new_tok) = expr(filename, src, &tok, locals)?;
            node.cond = Some(Box::new(cond));
            tok = new_tok;
        }
        tok = skip(filename, src, &tok, ";")?;

        if !equal(src, &tok, ")") {
            let (inc, new_tok) = expr(filename, src, &tok, locals)?;
            node.inc = Some(Box::new(inc));
            tok = new_tok;
        }
        tok = skip(filename, src, &tok, ")")?;

        let (then, tok) = stmt(filename, src, &tok, locals)?;
        node.then = Some(Box::new(then));
        return Ok((node, tok));
    }
    if equal(src, tok, "while") {
        let tok_loc = tok.loc;
        let mut node = new_node(NodeKind::While, tok_loc);
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(filename, src, &tok, locals)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;
        let (then, tok) = stmt(filename, src, &tok, locals)?;
        node.then = Some(Box::new(then));
        return Ok((node, tok));
    }
    if equal(src, tok, "{") {
        return compound_stmt(filename, src, tok.next.as_ref().unwrap(), locals);
    }
    expr_stmt(filename, src, tok, locals)
}

fn expr_stmt(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, ";") {
        let tok_loc = tok.loc;
        let tok = *tok.next.as_ref().unwrap().clone();
        return Ok((new_node(NodeKind::Block, tok_loc), tok));
    }
    let tok_loc = tok.loc;
    let (expr_node, tok) = expr(filename, src, tok, locals)?;
    let tok = skip(filename, src, &tok, ";")?;
    let node = new_unary(NodeKind::ExprStmt, expr_node, tok_loc);
    Ok((node, tok))
}

fn expr(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    assign(filename, src, tok, locals)
}

fn assign(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let (mut node, tok) = equality(filename, src, tok, locals)?;
    if equal(src, &tok, "=") {
        let tok_loc = tok.loc;
        let (rhs, tok) = assign(filename, src, tok.next.as_ref().unwrap(), locals)?;
        node = new_binary(NodeKind::Assign, node, rhs, tok_loc);
        return Ok((node, tok));
    }
    Ok((node, tok))
}

fn equality(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = relational(filename, src, tok, locals)?;

    loop {
        if equal(src, &tok, "==") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = relational(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Eq, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "!=") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = relational(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Ne, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn relational(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = add(filename, src, tok, locals)?;

    loop {
        if equal(src, &tok, "<") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Lt, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "<=") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Le, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">") {
            let tok_loc = tok.loc;
            let (lhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Lt, lhs, node, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">=") {
            let tok_loc = tok.loc;
            let (lhs, new_tok) = add(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Le, lhs, node, tok_loc);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn add(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = mul(filename, src, tok, locals)?;

    loop {
        if equal(src, &tok, "+") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = mul(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_add(node, rhs, tok_loc, filename, src)?;
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "-") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = mul(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_sub(node, rhs, tok_loc, filename, src)?;
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn mul(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = unary(filename, src, tok, locals)?;

    loop {
        if equal(src, &tok, "*") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = unary(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Mul, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "/") {
            let tok_loc = tok.loc;
            let (rhs, new_tok) = unary(filename, src, tok.next.as_ref().unwrap(), locals)?;
            node = new_binary(NodeKind::Div, node, rhs, tok_loc);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

fn unary(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "+") {
        return unary(filename, src, tok.next.as_ref().unwrap(), locals);
    }

    if equal(src, tok, "-") {
        let tok_loc = tok.loc;
        let (node, tok) = unary(filename, src, tok.next.as_ref().unwrap(), locals)?;
        return Ok((new_unary(NodeKind::Neg, node, tok_loc), tok));
    }

    if equal(src, tok, "&") {
        let tok_loc = tok.loc;
        let (node, tok) = unary(filename, src, tok.next.as_ref().unwrap(), locals)?;
        return Ok((new_unary(NodeKind::Addr, node, tok_loc), tok));
    }

    if equal(src, tok, "*") {
        let tok_loc = tok.loc;
        let (node, tok) = unary(filename, src, tok.next.as_ref().unwrap(), locals)?;
        return Ok((new_unary(NodeKind::Deref, node, tok_loc), tok));
    }

    postfix(filename, src, tok, locals)
}

fn postfix(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = primary(filename, src, tok, locals)?;

    while equal(src, &tok, "[") {
        let tok_loc = tok.loc;
        let (idx, new_tok) = expr(filename, src, tok.next.as_ref().unwrap(), locals)?;
        tok = skip(filename, src, &new_tok, "]")?;
        node = new_unary(
            NodeKind::Deref,
            new_add(node, idx, tok_loc, filename, src)?,
            tok_loc,
        );
    }

    Ok((node, tok))
}

fn funcall(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    let tok_loc = tok.loc;
    let funcname: String = src.chars().skip(tok.loc).take(tok.len).collect();
    let mut tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;

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
        funcname: None,
        args: None,
        var: None,
        val: 0,
    };
    let mut cur = &mut head;

    while !equal(src, &tok, ")") {
        if cur.tok_loc != tok_loc || cur.kind != NodeKind::Num {
            tok = skip(filename, src, &tok, ",")?;
        }
        let (arg, new_tok) = assign(filename, src, &tok, locals)?;
        tok = new_tok;
        cur.next = Some(Box::new(arg));
        cur = cur.next.as_mut().unwrap();
    }

    let tok = skip(filename, src, &tok, ")")?;

    let mut node = new_node(NodeKind::FuncCall, tok_loc);
    node.funcname = Some(funcname);
    node.args = head.next;
    Ok((node, tok))
}

fn primary(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "(") {
        let (node, tok) = expr(filename, src, tok.next.as_ref().unwrap(), locals)?;
        let tok = skip(filename, src, &tok, ")")?;
        return Ok((node, tok));
    }

    if equal(src, tok, "sizeof") {
        let tok_loc = tok.loc;
        let (mut node, tok) = unary(filename, src, tok.next.as_ref().unwrap(), locals)?;
        add_type(&mut node);
        let size = node.ty.as_ref().unwrap().size;
        return Ok((new_num(size, tok_loc), tok));
    }

    if tok.kind == TokenKind::Ident {
        if equal(src, tok.next.as_ref().unwrap(), "(") {
            return funcall(filename, src, tok, locals);
        }

        let tok_loc = tok.loc;
        let funcname: String = src.chars().skip(tok.loc).take(tok.len).collect();

        let var = find_var(locals, &funcname)
            .ok_or_else(|| error_tok(filename, src, tok, "undefined variable"))?;
        let node = new_var_node(var, tok_loc);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    if tok.kind == TokenKind::Num {
        let tok_loc = tok.loc;
        let node = new_num(tok.val, tok_loc);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    Err(error_tok(filename, src, tok, "expected an expression"))
}

fn gen_addr(node: &Node, result: &mut String, filename: &str, src: &str) -> Result<(), String> {
    match node.kind {
        NodeKind::Var => {
            let offset = node.var.as_ref().unwrap().offset;
            result.push_str(&format!("  lea -{}(%rbp), %rax\n", offset));
        }
        NodeKind::Deref => {
            gen_expr(node.lhs.as_ref().unwrap(), result, filename, src)?;
        }
        _ => return Err(error_at(filename, src, node.tok_loc, "not an lvalue")),
    }
    Ok(())
}

fn load(ty: &Type, result: &mut String) {
    if ty.kind == TypeKind::Array {
        return;
    }
    result.push_str("  mov (%rax), %rax\n");
}

fn store(result: &mut String) {
    result.push_str("  pop %rdi\n");
    result.push_str("  mov %rax, (%rdi)\n");
}

fn gen_expr(node: &Node, result: &mut String, filename: &str, src: &str) -> Result<(), String> {
    match node.kind {
        NodeKind::Num => {
            result.push_str(&format!("  mov ${}, %rax\n", node.val));
            return Ok(());
        }
        NodeKind::Neg => {
            gen_expr(node.lhs.as_ref().unwrap(), result, filename, src)?;
            result.push_str("  neg %rax\n");
            return Ok(());
        }
        NodeKind::Var => {
            gen_addr(node, result, filename, src)?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Addr => {
            gen_addr(node.lhs.as_ref().unwrap(), result, filename, src)?;
            return Ok(());
        }
        NodeKind::Deref => {
            gen_expr(node.lhs.as_ref().unwrap(), result, filename, src)?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Assign => {
            gen_addr(node.lhs.as_ref().unwrap(), result, filename, src)?;
            result.push_str("  push %rax\n");
            gen_expr(node.rhs.as_ref().unwrap(), result, filename, src)?;
            store(result);
            return Ok(());
        }
        NodeKind::FuncCall => {
            let argreg = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
            let mut nargs = 0;
            let mut arg = node.args.as_ref();
            while let Some(arg_node) = arg {
                gen_expr(arg_node, result, filename, src)?;
                result.push_str("  push %rax\n");
                nargs += 1;
                arg = arg_node.next.as_ref();
            }

            for i in (0..nargs).rev() {
                result.push_str(&format!("  pop {}\n", argreg[i]));
            }

            result.push_str("  mov $0, %rax\n");
            result.push_str(&format!("  call {}\n", node.funcname.as_ref().unwrap()));
            return Ok(());
        }
        _ => {}
    }

    gen_expr(node.rhs.as_ref().unwrap(), result, filename, src)?;
    result.push_str("  push %rax\n");
    gen_expr(node.lhs.as_ref().unwrap(), result, filename, src)?;
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
        | NodeKind::FuncCall
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
    result: &mut String,
    filename: &str,
    src: &str,
    current_fn: &str,
) -> Result<(), String> {
    match node.kind {
        NodeKind::If => {
            let c = count();
            gen_expr(node.cond.as_ref().unwrap(), result, filename, src)?;
            result.push_str("  cmp $0, %rax\n");
            result.push_str(&format!("  je .L.else.{}\n", c));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            result.push_str(&format!("  jmp .L.end.{}\n", c));
            result.push_str(&format!(".L.else.{}:\n", c));
            if let Some(els) = node.els.as_ref() {
                gen_stmt(els, result, filename, src, current_fn)?;
            }
            result.push_str(&format!(".L.end.{}:\n", c));
        }
        NodeKind::For => {
            let c = count();
            gen_stmt(
                node.init.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            result.push_str(&format!(".L.begin.{}:\n", c));
            if let Some(cond) = node.cond.as_ref() {
                gen_expr(cond, result, filename, src)?;
                result.push_str("  cmp $0, %rax\n");
                result.push_str(&format!("  je .L.end.{}\n", c));
            }
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            if let Some(inc) = node.inc.as_ref() {
                gen_expr(inc, result, filename, src)?;
            }
            result.push_str(&format!("  jmp .L.begin.{}\n", c));
            result.push_str(&format!(".L.end.{}:\n", c));
        }
        NodeKind::While => {
            let c = count();
            result.push_str(&format!(".L.begin.{}:\n", c));
            gen_expr(node.cond.as_ref().unwrap(), result, filename, src)?;
            result.push_str("  cmp $0, %rax\n");
            result.push_str(&format!("  je .L.end.{}\n", c));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            result.push_str(&format!("  jmp .L.begin.{}\n", c));
            result.push_str(&format!(".L.end.{}:\n", c));
        }
        NodeKind::Block => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                gen_stmt(stmt_node, result, filename, src, current_fn)?;
                n = stmt_node.next.as_ref();
            }
        }
        NodeKind::Return => {
            gen_expr(node.lhs.as_ref().unwrap(), result, filename, src)?;
            result.push_str(&format!("  jmp .L.return.{}\n", current_fn));
        }
        NodeKind::ExprStmt => {
            gen_expr(node.lhs.as_ref().unwrap(), result, filename, src)?;
        }
        _ => return Err(error_at(filename, src, node.tok_loc, "invalid statement")),
    }
    Ok(())
}

fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
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

    if let Some(args) = &mut node.args {
        let mut n = args;
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
            let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
            if lhs_ty.kind == TypeKind::Array {
                node.ty = Some(Type::new_int());
            } else {
                node.ty = Some(lhs_ty.clone());
            }
        }
        NodeKind::Eq
        | NodeKind::Ne
        | NodeKind::Lt
        | NodeKind::Le
        | NodeKind::Num
        | NodeKind::FuncCall => {
            node.ty = Some(Type::new_int());
        }
        NodeKind::Var => {
            node.ty = Some(node.var.as_ref().unwrap().ty.clone());
        }
        NodeKind::Addr => {
            let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
            if lhs_ty.kind == TypeKind::Array {
                node.ty = Some(Type::new_ptr(
                    lhs_ty.base.as_ref().unwrap().as_ref().clone(),
                ));
            } else {
                node.ty = Some(Type::new_ptr(lhs_ty.clone()));
            }
        }
        NodeKind::Deref => {
            let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
            if lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array {
                node.ty = Some(lhs_ty.base.as_ref().unwrap().as_ref().clone());
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

    if lhs_ty.kind == TypeKind::Array && rhs_ty.kind == TypeKind::Array {
        return Err(error_at(filename, src, tok_loc, "invalid operands"));
    }

    if !is_integer(lhs_ty) && !is_integer(rhs_ty) {
        return Err(error_at(filename, src, tok_loc, "invalid operands"));
    }

    if is_integer(lhs_ty) && (rhs_ty.kind == TypeKind::Ptr || rhs_ty.kind == TypeKind::Array) {
        std::mem::swap(&mut lhs, &mut rhs);
    }

    let base_size = lhs.ty.as_ref().unwrap().base.as_ref().unwrap().size;
    let rhs = new_binary(NodeKind::Mul, rhs, new_num(base_size, tok_loc), tok_loc);
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

    if (lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array) && is_integer(rhs_ty) {
        let lhs_ty_clone = lhs.ty.clone();
        let base_size = lhs.ty.as_ref().unwrap().base.as_ref().unwrap().size;
        let rhs = new_binary(NodeKind::Mul, rhs, new_num(base_size, tok_loc), tok_loc);
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc);
        node.ty = Some(Type::new_ptr(
            lhs_ty_clone
                .unwrap()
                .base
                .as_ref()
                .unwrap()
                .as_ref()
                .clone(),
        ));
        return Ok(node);
    }

    if (lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array)
        && (rhs_ty.kind == TypeKind::Ptr || rhs_ty.kind == TypeKind::Array)
    {
        let base_size = lhs.ty.as_ref().unwrap().base.as_ref().unwrap().size;
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc);
        node.ty = Some(Type::new_int());
        let mut result = new_binary(NodeKind::Div, node, new_num(base_size, tok_loc), tok_loc);
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

    let mut head = Obj {
        next: None,
        name: String::new(),
        ty: Type::new_int(),
        is_local: false,
        offset: 0,
        is_function: false,
        params: Vec::new(),
        body: None,
        locals: Vec::new(),
        stack_size: 0,
    };
    let mut cur = &mut head;

    let mut tok = tok;
    while tok.kind != TokenKind::Eof {
        let (basety, new_tok) = declspec(filename, src, &tok)?;
        tok = new_tok;
        let (func, new_tok) = function(filename, src, &tok, basety)?;
        tok = new_tok;
        cur.next = Some(Box::new(func));
        cur = cur.next.as_mut().unwrap();
    }

    let prog = head.next.unwrap();

    let mut result = String::new();

    let mut func = &prog;
    loop {
        if !func.is_function {
            if let Some(next) = &func.next {
                func = next;
                continue;
            } else {
                break;
            }
        }

        let mut stack_size = 0;
        for var in func.locals.iter() {
            stack_size += var.ty.size;
        }
        let stack_size = align_to(stack_size, 16);

        result.push_str("  .text\n");
        result.push_str(&format!("  .globl {}\n", func.name));
        result.push_str(&format!("{}:\n", func.name));

        result.push_str("  push %rbp\n");
        result.push_str("  mov %rsp, %rbp\n");
        result.push_str(&format!("  sub ${}, %rsp\n", stack_size));

        let argreg = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
        for (i, var) in func.params.iter().enumerate() {
            result.push_str(&format!("  mov {}, -{}(%rbp)\n", argreg[i], var.offset));
        }

        gen_stmt(
            func.body.as_ref().unwrap(),
            &mut result,
            filename,
            src,
            &func.name,
        )?;

        result.push_str(&format!(".L.return.{}:\n", func.name));
        result.push_str("  mov %rbp, %rsp\n");
        result.push_str("  pop %rbp\n");
        result.push_str("  ret\n");

        if let Some(next) = &func.next {
            func = next;
        } else {
            break;
        }
    }

    Ok(result)
}

fn run() -> Result<(), String> {
    let args = parse_args();

    let (filename, src) = read_file(&args.input_path)?;
    let asm = emit_assembly(&filename, &src)?;

    let mut out = open_output_file(args.opt_o.as_ref());
    out.write_all(asm.as_bytes())
        .map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

fn main() {
    if let Err(msg) = run() {
        eprintln!("{msg}");
        process::exit(1);
    }
}

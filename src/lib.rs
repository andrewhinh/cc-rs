pub mod codegen;
pub mod parse;
pub mod tokenize;

pub use parse::{
    add_type, declspec, find_tag, function, global_variable, is_function, is_typename,
    push_tag_scope,
};
pub use tokenize::{consume, equal, skip, tokenize};

use std::sync::atomic::{AtomicI32, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Ident,
    Punct,
    Keyword,
    Str,
    Num,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub next: Option<Box<Token>>,
    pub val: i64,
    pub loc: usize,
    pub len: usize,
    pub ty: Option<Type>,
    pub str: Option<Vec<u8>>,
    pub line_no: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
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
    Comma,
    Member,
    Addr,
    Deref,
    Return,
    If,
    For,
    While,
    Block,
    FuncCall,
    ExprStmt,
    StmtExpr,
    Var,
    Num,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Void,
    Char,
    Short,
    Int,
    Long,
    Ptr,
    Func,
    Array,
    Struct,
    Union,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub next: Option<Box<Member>>,
    pub ty: Type,
    pub name: Option<Box<Token>>,
    pub offset: i64,
}

#[derive(Debug, Clone)]
pub struct Type {
    pub kind: TypeKind,
    pub size: i64,
    pub align: i64,
    pub base: Option<Box<Type>>,
    pub name: Option<Box<Token>>,
    #[allow(unused)]
    pub return_ty: Option<Box<Type>>,
    pub params: Option<Box<Type>>,
    pub next: Option<Box<Type>>,
    #[allow(dead_code)]
    pub array_len: i64,
    pub members: Option<Box<Member>>,
}

impl Type {
    pub fn new_void() -> Type {
        Type {
            kind: TypeKind::Void,
            size: 1,
            align: 1,
            base: None,
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
        }
    }

    pub fn new_char() -> Type {
        Type {
            kind: TypeKind::Char,
            size: 1,
            align: 1,
            base: None,
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
        }
    }

    pub fn new_short() -> Type {
        Type {
            kind: TypeKind::Short,
            size: 2,
            align: 2,
            base: None,
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
        }
    }

    pub fn new_int() -> Type {
        Type {
            kind: TypeKind::Int,
            size: 4,
            align: 4,
            base: None,
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
        }
    }

    pub fn new_long() -> Type {
        Type {
            kind: TypeKind::Long,
            size: 8,
            align: 8,
            base: None,
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
        }
    }

    pub fn new_ptr(base: Type) -> Type {
        Type {
            kind: TypeKind::Ptr,
            size: 8,
            align: 8,
            base: Some(Box::new(base)),
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
        }
    }

    pub fn new_array(base: Type, len: i64) -> Type {
        Type {
            kind: TypeKind::Array,
            size: base.size * len,
            align: base.align,
            base: Some(Box::new(base)),
            name: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: len,
            members: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Obj {
    pub name: String,
    pub ty: Type,
    pub is_local: bool,
    pub offset: i64,
    pub is_function: bool,
    pub is_definition: bool,
    pub init_data: Option<Vec<u8>>,
    pub params: Vec<Obj>,
    pub body: Option<Box<Node>>,
    pub locals: Vec<Obj>,
    #[allow(dead_code)]
    pub stack_size: i64,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub tok_loc: usize,
    pub line_no: usize,
    pub ty: Option<Type>,
    pub next: Option<Box<Node>>,
    pub lhs: Option<Box<Node>>,
    pub rhs: Option<Box<Node>>,
    pub cond: Option<Box<Node>>,
    pub then: Option<Box<Node>>,
    pub els: Option<Box<Node>>,
    pub init: Option<Box<Node>>,
    pub inc: Option<Box<Node>>,
    pub body: Option<Box<Node>>,
    pub funcname: Option<String>,
    pub args: Option<Box<Node>>,
    pub var: Option<Box<Obj>>,
    pub val: i64,
    pub member: Option<Box<Member>>,
}

pub fn error_at(filename: &str, src: &str, loc: usize, msg: &str) -> String {
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

fn verror_at(filename: &str, src: &str, loc: usize, line_no: usize, msg: &str) -> String {
    let mut line_start = loc;
    while line_start > 0 && src.as_bytes()[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    let mut line_end = loc;
    while line_end < src.len() && src.as_bytes()[line_end] != b'\n' {
        line_end += 1;
    }

    let line = &src[line_start..line_end];

    let indent = format!("{filename}:{line_no}: ").len();
    let pos = loc - line_start + indent;

    format!(
        "{filename}:{line_no}: {line}\n{:width$}^ {msg}\n",
        "",
        width = pos
    )
}

pub fn error_tok(filename: &str, src: &str, tok: &Token, msg: &str) -> String {
    verror_at(filename, src, tok.loc, tok.line_no, msg)
}

pub fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
}

static UNIQUE_ID: AtomicI32 = AtomicI32::new(0);

pub fn new_unique_name() -> String {
    let id = UNIQUE_ID.fetch_add(1, Ordering::SeqCst);
    format!(".L..{}", id)
}

#[derive(Debug, Clone)]
pub struct VarScope {
    pub name: String,
    pub var: Obj,
}

#[derive(Debug, Clone)]
pub struct TagScope {
    pub name: String,
    pub ty: Type,
}

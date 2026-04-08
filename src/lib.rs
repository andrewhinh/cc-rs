pub mod codegen;
pub mod parse;
pub mod preprocess;
pub mod tokenize;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

pub use parse::{
    add_type, const_expr, declspec, find_tag, find_typedef, function, global_variable, is_function,
    is_typename, parse_typedef, push_tag_scope,
};
pub use preprocess::preprocess;
pub use tokenize::{
    add_input_file, consume, equal, error_at, error_tok, get_file_no, get_input_files, new_file,
    skip, tokenize, tokenize_file, warn_tok,
};

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
pub struct File {
    pub name: String,
    pub file_no: usize,
    pub contents: String,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub next: Option<Box<Token>>,
    pub val: i64,
    pub fval: f64,
    pub loc: usize,
    pub len: usize,
    pub ty: Option<Type>,
    pub str: Option<Vec<u8>>,
    pub file_no: usize,
    pub line_no: usize,
    pub at_bol: bool,
    pub has_space: bool,
    pub hideset: HashSet<String>,
    pub origin: Option<Box<Token>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Assign,
    Cond,
    Comma,
    Member,
    Addr,
    Deref,
    Not,
    BitNot,
    LogAnd,
    LogOr,
    Return,
    If,
    For,
    While,
    Do,
    Block,
    FuncCall,
    ExprStmt,
    StmtExpr,
    Var,
    Num,
    Cast,
    Goto,
    Label,
    Switch,
    Case,
    NullExpr,
    Memzero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Void,
    Bool,
    Char,
    Short,
    Int,
    Long,
    Enum,
    Ptr,
    Func,
    Array,
    Struct,
    Union,
    Float,
    Double,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub next: Option<Box<Member>>,
    pub ty: Type,
    pub tok: Option<Box<Token>>,
    pub name: Option<Box<Token>>,
    pub idx: i64,
    pub align: i64,
    pub offset: i64,
}

#[derive(Debug, Clone)]
pub struct Type {
    pub kind: TypeKind,
    pub size: i64,
    pub align: i64,
    pub is_unsigned: bool,
    pub base: Option<Rc<RefCell<Type>>>,
    pub name: Option<Box<Token>>,
    pub name_pos: Option<Box<Token>>,
    #[allow(unused)]
    pub return_ty: Option<Box<Type>>,
    pub params: Option<Box<Type>>,
    pub next: Option<Box<Type>>,
    #[allow(dead_code)]
    pub array_len: i64,
    pub members: Option<Box<Member>>,
    pub origin: Option<Rc<RefCell<Type>>>,
    pub is_flexible: bool,
    pub is_variadic: bool,
}

impl Type {
    pub fn new_void() -> Type {
        Type {
            kind: TypeKind::Void,
            size: 1,
            align: 1,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_bool() -> Type {
        Type {
            kind: TypeKind::Bool,
            size: 1,
            align: 1,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_char() -> Type {
        Type {
            kind: TypeKind::Char,
            size: 1,
            align: 1,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_short() -> Type {
        Type {
            kind: TypeKind::Short,
            size: 2,
            align: 2,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_int() -> Type {
        Type {
            kind: TypeKind::Int,
            size: 4,
            align: 4,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_long() -> Type {
        Type {
            kind: TypeKind::Long,
            size: 8,
            align: 8,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_uchar() -> Type {
        Type {
            kind: TypeKind::Char,
            size: 1,
            align: 1,
            is_unsigned: true,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_ushort() -> Type {
        Type {
            kind: TypeKind::Short,
            size: 2,
            align: 2,
            is_unsigned: true,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_uint() -> Type {
        Type {
            kind: TypeKind::Int,
            size: 4,
            align: 4,
            is_unsigned: true,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_ulong() -> Type {
        Type {
            kind: TypeKind::Long,
            size: 8,
            align: 8,
            is_unsigned: true,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_ptr(base: Type) -> Type {
        Type {
            kind: TypeKind::Ptr,
            size: 8,
            align: 8,
            is_unsigned: true,
            base: Some(Rc::new(RefCell::new(base))),
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_ptr_shared(base: Rc<RefCell<Type>>) -> Type {
        Type {
            kind: TypeKind::Ptr,
            size: 8,
            align: 8,
            is_unsigned: true,
            base: Some(base),
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_array(base: Type, len: i64) -> Type {
        let size = if len < 0 { 0 } else { base.size * len };
        Type {
            kind: TypeKind::Array,
            size,
            align: base.align,
            is_unsigned: false,
            base: Some(Rc::new(RefCell::new(base))),
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: len,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_struct() -> Type {
        Type {
            kind: TypeKind::Struct,
            size: 0,
            align: 1,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_enum() -> Type {
        Type {
            kind: TypeKind::Enum,
            size: 4,
            align: 4,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_float() -> Type {
        Type {
            kind: TypeKind::Float,
            size: 4,
            align: 4,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }

    pub fn new_double() -> Type {
        Type {
            kind: TypeKind::Double,
            size: 8,
            align: 8,
            is_unsigned: false,
            base: None,
            name: None,
            name_pos: None,
            return_ty: None,
            params: None,
            next: None,
            array_len: 0,
            members: None,
            origin: None,
            is_flexible: false,
            is_variadic: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Relocation {
    pub next: Option<Box<Relocation>>,
    pub offset: i64,
    pub label: String,
    pub addend: i64,
}

#[derive(Debug, Clone)]
pub struct Obj {
    pub name: String,
    pub ty: Type,
    pub is_local: bool,
    pub align: i64,
    pub offset: i64,
    pub is_function: bool,
    pub is_definition: bool,
    pub is_static: bool,
    pub init_data: Option<Vec<u8>>,
    pub rel: Option<Box<Relocation>>,
    pub params: Vec<Obj>,
    pub body: Option<Box<Node>>,
    pub locals: Vec<Obj>,
    pub va_area: Option<Box<Obj>>,
    #[allow(dead_code)]
    pub stack_size: i64,
    pub unique_id: u64,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub kind: NodeKind,
    pub tok_loc: usize,
    pub file_no: usize,
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
    pub func_ty: Option<Type>,
    pub args: Option<Box<Node>>,
    pub pass_by_stack: bool,
    pub ret_buffer: Option<Box<Obj>>,
    pub var: Option<Box<Obj>>,
    pub val: i64,
    pub fval: f64,
    pub member: Option<Box<Member>>,
    pub label: Option<String>,
    pub unique_label: Option<String>,
    pub goto_next: Option<Box<Node>>,
    pub brk_label: Option<String>,
    pub cont_label: Option<String>,
    pub case_next: Option<Box<Node>>,
    pub default_case: Option<Box<Node>>,
}

pub fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
}

pub fn is_flonum(ty: &Type) -> bool {
    ty.kind == TypeKind::Float || ty.kind == TypeKind::Double
}

pub fn is_integer(ty: &Type) -> bool {
    ty.kind == TypeKind::Bool
        || ty.kind == TypeKind::Char
        || ty.kind == TypeKind::Short
        || ty.kind == TypeKind::Int
        || ty.kind == TypeKind::Long
        || ty.kind == TypeKind::Enum
}

pub fn is_numeric(ty: &Type) -> bool {
    is_integer(ty) || is_flonum(ty)
}

static UNIQUE_ID: AtomicI32 = AtomicI32::new(0);
static VAR_UNIQUE_ID: AtomicU64 = AtomicU64::new(0);

pub fn new_unique_name() -> String {
    let id = UNIQUE_ID.fetch_add(1, Ordering::SeqCst);
    format!(".L..{}", id)
}

pub fn new_var_unique_id() -> u64 {
    VAR_UNIQUE_ID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug, Clone)]
pub struct VarScope {
    pub name: String,
    pub var: Option<Obj>,
    pub type_def: Option<Rc<RefCell<Type>>>,
    pub enum_ty: Option<Type>,
    pub enum_val: i64,
}

#[derive(Debug, Clone)]
pub struct TagScope {
    pub name: String,
    pub ty: Rc<RefCell<Type>>,
}

#[derive(Debug, Clone, Default)]
pub struct VarAttr {
    pub is_typedef: bool,
    pub is_static: bool,
    pub is_extern: bool,
    pub align: i64,
}

static INCLUDE_PATHS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn get_include_paths() -> Vec<String> {
    INCLUDE_PATHS.lock().unwrap().clone()
}

pub fn set_include_paths(paths: Vec<String>) {
    *INCLUDE_PATHS.lock().unwrap() = paths;
}

pub fn add_include_path(path: String) {
    INCLUDE_PATHS.lock().unwrap().push(path);
}

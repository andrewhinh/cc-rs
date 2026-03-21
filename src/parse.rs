use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    Node, NodeKind, Obj, TagScope, Token, TokenKind, Type, TypeKind, VarAttr, VarScope, align_to,
    error_at, error_tok, new_unique_name, new_var_unique_id,
};
use crate::{consume, equal, skip};

thread_local! {
    static GOTOS: Cell<Option<Box<Node>>> = const { Cell::new(None) };
    static LABELS: Cell<Option<Box<Node>>> = const { Cell::new(None) };
    static BRK_LABEL: Cell<Option<String>> = const { Cell::new(None) };
    static CONT_LABEL: Cell<Option<String>> = const { Cell::new(None) };
    static CURRENT_SWITCH: Cell<Option<Box<Node>>> = const { Cell::new(None) };
}

fn gotos_get() -> Option<Box<Node>> {
    GOTOS.with(|g| g.take())
}

fn gotos_set(node: Option<Box<Node>>) {
    GOTOS.with(|g| g.set(node));
}

fn labels_get() -> Option<Box<Node>> {
    LABELS.with(|l| l.take())
}

fn labels_set(node: Option<Box<Node>>) {
    LABELS.with(|l| l.set(node));
}

fn brk_label_get() -> Option<String> {
    BRK_LABEL.with(|b| b.take())
}

fn brk_label_set(label: Option<String>) {
    BRK_LABEL.with(|b| b.set(label));
}

fn cont_label_get() -> Option<String> {
    CONT_LABEL.with(|c| c.take())
}

fn cont_label_set(label: Option<String>) {
    CONT_LABEL.with(|c| c.set(label));
}

fn current_switch_get() -> Option<Box<Node>> {
    CURRENT_SWITCH.with(|c| c.take())
}

fn current_switch_set(node: Option<Box<Node>>) {
    CURRENT_SWITCH.with(|c| c.set(node));
}

fn is_end(src: &str, tok: &Token) -> bool {
    equal(src, tok, "}")
        || (equal(src, tok, ",") && tok.next.as_ref().is_some_and(|n| equal(src, n, "}")))
}

fn consume_end(src: &str, tok: &Token) -> (bool, Token) {
    if equal(src, tok, "}") {
        return (true, tok.next.as_ref().unwrap().as_ref().clone());
    }
    if equal(src, tok, ",") && tok.next.as_ref().is_some_and(|n| equal(src, n, "}")) {
        return (
            true,
            tok.next
                .as_ref()
                .unwrap()
                .next
                .as_ref()
                .unwrap()
                .as_ref()
                .clone(),
        );
    }
    (false, tok.clone())
}

#[derive(Debug, Clone)]
struct Initializer {
    ty: Type,
    expr: Option<Node>,
    children: Vec<Initializer>,
    is_flexible: bool,
}

#[derive(Debug, Clone)]
struct InitDesg {
    next: Option<Box<InitDesg>>,
    idx: i64,
    member: Option<crate::Member>,
    var: Option<Obj>,
}

fn new_initializer(ty: &Type, is_flexible: bool) -> Initializer {
    if ty.kind == TypeKind::Array {
        if is_flexible && ty.array_len < 0 {
            return Initializer {
                ty: ty.clone(),
                expr: None,
                children: Vec::new(),
                is_flexible: true,
            };
        }
        let mut children = Vec::with_capacity(ty.array_len as usize);
        for _ in 0..ty.array_len {
            let base_ty = ty.base.as_ref().unwrap().borrow().clone();
            children.push(new_initializer(&base_ty, false));
        }
        return Initializer {
            ty: ty.clone(),
            expr: None,
            children,
            is_flexible: false,
        };
    }

    if ty.kind == TypeKind::Struct || ty.kind == TypeKind::Union {
        let mut len = 0;
        let mut current = ty.members.as_ref();
        while let Some(mem) = current {
            len += 1;
            current = mem.next.as_ref();
        }

        let mut children = vec![
            Initializer {
                ty: Type::new_int(),
                expr: None,
                children: Vec::new(),
                is_flexible: false,
            };
            len
        ];

        let mut current = ty.members.as_ref();
        while let Some(mem) = current {
            if is_flexible && ty.is_flexible && mem.next.is_none() {
                children[mem.idx as usize] = Initializer {
                    ty: mem.ty.clone(),
                    expr: None,
                    children: Vec::new(),
                    is_flexible: true,
                };
            } else {
                children[mem.idx as usize] = new_initializer(&mem.ty, false);
            }
            current = mem.next.as_ref();
        }

        return Initializer {
            ty: ty.clone(),
            expr: None,
            children,
            is_flexible: false,
        };
    }

    Initializer {
        ty: ty.clone(),
        expr: None,
        children: Vec::new(),
        is_flexible: false,
    }
}

pub fn new_node(kind: NodeKind, tok_loc: usize, line_no: usize) -> Node {
    Node {
        kind,
        tok_loc,
        line_no,
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
        func_ty: None,
        args: None,
        var: None,
        val: 0,
        member: None,
        label: None,
        unique_label: None,
        goto_next: None,
        brk_label: None,
        cont_label: None,
        case_next: None,
        default_case: None,
    }
}

fn new_case_link(node: &Node) -> Node {
    let mut link = new_node(NodeKind::Case, node.tok_loc, node.file_no, node.line_no);
    link.label = node.label.clone();
    link.val = node.val;
    link
}

fn token_snapshot(tok: &Token) -> Token {
    Token {
        kind: tok.kind,
        next: None,
        val: tok.val,
        loc: tok.loc,
        len: tok.len,
        ty: tok.ty.clone(),
        str: tok.str.clone(),
        line_no: tok.line_no,
    }
}

pub fn new_binary(kind: NodeKind, lhs: Node, rhs: Node, tok_loc: usize, line_no: usize) -> Node {
    let mut node = new_node(kind, tok_loc, line_no);
    node.lhs = Some(Box::new(lhs));
    node.rhs = Some(Box::new(rhs));
    node
}

pub fn new_unary(kind: NodeKind, expr: Node, tok_loc: usize, line_no: usize) -> Node {
    let mut node = new_node(kind, tok_loc, line_no);
    node.lhs = Some(Box::new(expr));
    node
}

pub fn new_num(val: i64, tok_loc: usize, line_no: usize) -> Node {
    let mut node = new_node(NodeKind::Num, tok_loc, line_no);
    node.val = val;
    node
}

pub fn new_long(val: i64, tok_loc: usize, line_no: usize) -> Node {
    let mut node = new_node(NodeKind::Num, tok_loc, line_no);
    node.val = val;
    node.ty = Some(Type::new_long());
    node
}

pub fn new_var_node(var: Obj, tok_loc: usize, line_no: usize) -> Node {
    let mut node = new_node(NodeKind::Var, tok_loc, line_no);
    node.var = Some(Box::new(var.clone()));
    node.ty = Some(var.ty);
    node
}

pub fn new_cast(expr: Node, ty: Type) -> Node {
    let mut expr = expr;
    add_type(&mut expr);
    let mut node = new_node(NodeKind::Cast, expr.tok_loc, expr.line_no);
    node.lhs = Some(Box::new(expr));
    node.ty = Some(ty);
    node
}

pub fn find_var(scope_stack: &[Vec<VarScope>], globals: &[Obj], name: &str) -> Option<VarScope> {
    for scope in scope_stack.iter().rev() {
        for vs in scope.iter().rev() {
            if vs.name == name {
                return Some(vs.clone());
            }
        }
    }
    for var in globals.iter() {
        if var.name == name {
            return Some(VarScope {
                name: var.name.clone(),
                var: Some(var.clone()),
                type_def: None,
                enum_ty: None,
                enum_val: 0,
            });
        }
    }
    None
}

pub fn find_typedef(
    scope_stack: &[Vec<VarScope>],
    tok: &Token,
    src: &str,
) -> Option<Rc<RefCell<Type>>> {
    if tok.kind != TokenKind::Ident {
        return None;
    }
    let name: String = src.chars().skip(tok.loc).take(tok.len).collect();
    for scope in scope_stack.iter().rev() {
        for vs in scope.iter().rev() {
            if vs.name == name {
                return vs.type_def.clone();
            }
        }
    }
    None
}

pub fn find_tag(tag_scope_stack: &[Vec<TagScope>], name: &str) -> Option<Rc<RefCell<Type>>> {
    for scope in tag_scope_stack.iter().rev() {
        for ts in scope.iter().rev() {
            if ts.name == name {
                return Some(ts.ty.clone());
            }
        }
    }
    None
}

fn find_tag_in_current_scope(
    tag_scope_stack: &[Vec<TagScope>],
    name: &str,
) -> Option<Rc<RefCell<Type>>> {
    for ts in tag_scope_stack.last()? {
        if ts.name == name {
            return Some(ts.ty.clone());
        }
    }
    None
}

pub fn push_tag_scope(tag_scope_stack: &mut [Vec<TagScope>], name: String, ty: Rc<RefCell<Type>>) {
    tag_scope_stack
        .last_mut()
        .unwrap()
        .push(TagScope { name, ty });
}

pub fn new_var(name: String, ty: Type) -> Obj {
    Obj {
        name,
        ty: ty.clone(),
        is_local: false,
        align: ty.align,
        offset: 0,
        is_function: false,
        is_definition: false,
        is_static: false,
        init_data: None,
        rel: None,
        params: Vec::new(),
        body: None,
        locals: Vec::new(),
        stack_size: 0,
        unique_id: new_var_unique_id(),
    }
}

pub fn new_anon_gvar(ty: Type) -> Obj {
    let mut var = new_var(new_unique_name(), ty);
    var.is_definition = true;
    var
}

pub fn new_string_literal(str_content: &[u8], ty: Type) -> Obj {
    let mut var = new_anon_gvar(ty);
    let mut init_data: Vec<u8> = str_content.to_vec();
    init_data.push(0);
    var.init_data = Some(init_data);
    var
}

#[allow(clippy::ptr_arg)]
pub fn new_lvar(
    name: String,
    ty: Type,
    locals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
) -> Obj {
    let mut var = new_var(name.clone(), ty);
    var.is_local = true;
    locals.push(var.clone());
    scope_stack.last_mut().unwrap().push(VarScope {
        name,
        var: Some(var.clone()),
        type_def: None,
        enum_ty: None,
        enum_val: 0,
    });
    var
}

pub fn new_gvar(name: String, ty: Type) -> Obj {
    let mut var = new_var(name, ty);
    var.is_local = false;
    var.is_definition = true;
    var.is_static = true;
    var
}

fn skip_excess_element(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    if equal(src, tok, "{") {
        let tok = skip(filename, src, tok, "{")?;
        let tok = skip_excess_element(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return skip(filename, src, &tok, "}");
    }

    let (_, tok) = assign(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
fn count_array_init_elements(
    filename: &str,
    src: &str,
    tok: &Token,
    ty: &Type,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<i64, String> {
    let base_ty = ty.base.as_ref().unwrap().borrow().clone();
    let dummy = new_initializer(&base_ty, false);
    let mut tok = tok.clone();
    let mut i = 0;

    loop {
        let (is_end, _) = consume_end(src, &tok);
        if is_end {
            break;
        }
        if i > 0 {
            tok = skip(filename, src, &tok, ",")?;
        }
        let mut dummy = dummy.clone();
        tok = initializer2(
            filename,
            src,
            &tok,
            &mut dummy,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        i += 1;
    }
    Ok(i)
}

fn string_initializer(tok: &Token, init: &mut Initializer) -> Token {
    if init.is_flexible {
        let base_ty = init.ty.base.as_ref().unwrap().borrow().clone();
        let str_len = tok.ty.as_ref().unwrap().array_len;
        let new_ty = Type::new_array(base_ty, str_len);
        *init = new_initializer(&new_ty, false);
    }

    let str_content = tok.str.as_ref().unwrap();
    let str_len = tok.ty.as_ref().unwrap().array_len as usize;
    let array_len = init.ty.array_len as usize;
    let len = array_len.min(str_len);
    for (i, &c) in str_content.iter().take(len).enumerate() {
        init.children[i].expr = Some(new_num(c as i64, tok.loc, tok.line_no));
    }
    tok.next.as_ref().unwrap().as_ref().clone()
}

#[allow(clippy::too_many_arguments)]
fn array_initializer1(
    filename: &str,
    src: &str,
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    let mut tok = skip(filename, src, tok, "{")?;

    if init.is_flexible {
        let len = count_array_init_elements(
            filename,
            src,
            &tok,
            &init.ty,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let base_ty = init.ty.base.as_ref().unwrap().borrow().clone();
        let new_ty = Type::new_array(base_ty, len);
        *init = new_initializer(&new_ty, false);
    }

    let mut i = 0;
    loop {
        let (is_end, new_tok) = consume_end(src, &tok);
        if is_end {
            return Ok(new_tok);
        }
        if i > 0 {
            tok = skip(filename, src, &tok, ",")?;
        }

        if i < init.ty.array_len as usize {
            tok = initializer2(
                filename,
                src,
                &tok,
                &mut init.children[i],
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
        } else {
            tok = skip_excess_element(
                filename,
                src,
                &tok,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
        }
        i += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn array_initializer2(
    filename: &str,
    src: &str,
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    if init.is_flexible {
        let len = count_array_init_elements(
            filename,
            src,
            tok,
            &init.ty,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let base_ty = init.ty.base.as_ref().unwrap().borrow().clone();
        let new_ty = Type::new_array(base_ty, len);
        *init = new_initializer(&new_ty, false);
    }

    let mut tok = tok.clone();
    for i in 0..init.ty.array_len as usize {
        if is_end(src, &tok) {
            break;
        }
        if i > 0 {
            tok = skip(filename, src, &tok, ",")?;
        }
        tok = initializer2(
            filename,
            src,
            &tok,
            &mut init.children[i],
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
    }
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
fn struct_initializer1(
    filename: &str,
    src: &str,
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    let mut tok = skip(filename, src, tok, "{")?;

    let mut mem = init.ty.members.as_ref();
    let mut first = true;

    loop {
        let (is_end, new_tok) = consume_end(src, &tok);
        if is_end {
            return Ok(new_tok);
        }

        if !first {
            tok = skip(filename, src, &tok, ",")?;
        }
        first = false;

        if let Some(m) = mem {
            tok = initializer2(
                filename,
                src,
                &tok,
                &mut init.children[m.idx as usize],
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            mem = m.next.as_ref();
        } else {
            tok = skip_excess_element(
                filename,
                src,
                &tok,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn struct_initializer2(
    filename: &str,
    src: &str,
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    let mut tok = tok.clone();
    let mut first = true;

    let mut mem = init.ty.members.as_ref();
    while let Some(m) = mem {
        if is_end(src, &tok) {
            break;
        }
        if !first {
            tok = skip(filename, src, &tok, ",")?;
        }
        first = false;
        tok = initializer2(
            filename,
            src,
            &tok,
            &mut init.children[m.idx as usize],
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        mem = m.next.as_ref();
    }
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
fn union_initializer(
    filename: &str,
    src: &str,
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    if equal(src, tok, "{") {
        let tok = skip(filename, src, tok, "{")?;
        let tok = initializer2(
            filename,
            src,
            &tok,
            &mut init.children[0],
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let (_, tok) = consume(src, &tok, ",");
        return skip(filename, src, &tok, "}");
    }
    initializer2(
        filename,
        src,
        tok,
        &mut init.children[0],
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )
}

#[allow(clippy::too_many_arguments)]
fn initializer2(
    filename: &str,
    src: &str,
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    if init.ty.kind == TypeKind::Array && tok.kind == TokenKind::Str {
        return Ok(string_initializer(tok, init));
    }

    if init.ty.kind == TypeKind::Array {
        if equal(src, tok, "{") {
            return array_initializer1(
                filename,
                src,
                tok,
                init,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            );
        }
        return array_initializer2(
            filename,
            src,
            tok,
            init,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        );
    }

    if init.ty.kind == TypeKind::Struct {
        if equal(src, tok, "{") {
            return struct_initializer1(
                filename,
                src,
                tok,
                init,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            );
        }

        let (expr_node, new_tok) = assign(
            filename,
            src,
            tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let mut expr_node = expr_node;
        add_type(&mut expr_node);
        if expr_node.ty.as_ref().unwrap().kind == TypeKind::Struct {
            init.expr = Some(expr_node);
            return Ok(new_tok);
        }

        return struct_initializer2(
            filename,
            src,
            tok,
            init,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        );
    }

    if init.ty.kind == TypeKind::Union {
        return union_initializer(
            filename,
            src,
            tok,
            init,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        );
    }

    if equal(src, tok, "{") {
        let tok = initializer2(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            init,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return skip(filename, src, &tok, "}");
    }

    let (expr_node, tok) = assign(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;
    init.expr = Some(expr_node);
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
fn initializer(
    filename: &str,
    src: &str,
    tok: &Token,
    ty: &Type,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Initializer, Type, Token), String> {
    let mut init = new_initializer(ty, true);
    let tok = initializer2(
        filename,
        src,
        tok,
        &mut init,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    if (ty.kind == TypeKind::Struct || ty.kind == TypeKind::Union) && ty.is_flexible {
        let mut new_ty = copy_struct_type(ty);

        let mut mem = new_ty.members.as_mut();
        while let Some(m) = mem {
            if m.next.is_none() {
                m.ty = init.children[m.idx as usize].ty.clone();
                new_ty.size += m.ty.size;
                break;
            }
            mem = m.next.as_mut();
        }

        return Ok((init, new_ty, tok));
    }

    let new_ty = init.ty.clone();
    Ok((init, new_ty, tok))
}

fn init_desg_expr(
    desg: &InitDesg,
    tok_loc: usize,
    line_no: usize,
    filename: &str,
    src: &str,
) -> Result<Node, String> {
    if let Some(var) = &desg.var {
        return Ok(new_var_node(var.clone(), tok_loc, line_no));
    }

    if let Some(member) = &desg.member {
        let node = init_desg_expr(
            desg.next.as_ref().unwrap().as_ref(),
            tok_loc,
            line_no,
            filename,
            src,
        )?;
        let mut node = new_unary(NodeKind::Member, node, tok_loc, line_no);
        node.member = Some(Box::new(member.clone()));
        return Ok(node);
    }

    let lhs = init_desg_expr(
        desg.next.as_ref().unwrap().as_ref(),
        tok_loc,
        line_no,
        filename,
        src,
    )?;
    let rhs = new_num(desg.idx, tok_loc, line_no);
    let add_node = new_add(lhs, rhs, tok_loc, line_no, filename, src)?;
    Ok(new_unary(NodeKind::Deref, add_node, tok_loc, line_no))
}

fn create_lvar_init(
    init: &Initializer,
    ty: &Type,
    desg: &InitDesg,
    tok_loc: usize,
    line_no: usize,
    filename: &str,
    src: &str,
) -> Result<Node, String> {
    if ty.kind == TypeKind::Array {
        let mut node = new_node(NodeKind::NullExpr, tok_loc, line_no);
        for i in 0..ty.array_len as usize {
            let desg2 = InitDesg {
                next: Some(Box::new(desg.clone())),
                idx: i as i64,
                member: None,
                var: None,
            };
            let base_ty = ty.base.as_ref().unwrap().borrow().clone();
            let rhs = create_lvar_init(
                &init.children[i],
                &base_ty,
                &desg2,
                tok_loc,
                line_no,
                filename,
                src,
            )?;
            node = new_binary(NodeKind::Comma, node, rhs, tok_loc, line_no);
        }
        return Ok(node);
    }

    if ty.kind == TypeKind::Struct {
        if let Some(rhs) = &init.expr {
            let lhs = init_desg_expr(desg, tok_loc, line_no, filename, src)?;
            return Ok(new_binary(
                NodeKind::Assign,
                lhs,
                rhs.clone(),
                tok_loc,
                line_no,
            ));
        }

        let mut node = new_node(NodeKind::NullExpr, tok_loc, line_no);

        let mut current = ty.members.as_ref();
        while let Some(mem) = current {
            let desg2 = InitDesg {
                next: Some(Box::new(desg.clone())),
                idx: 0,
                member: Some(mem.as_ref().clone()),
                var: None,
            };
            let rhs = create_lvar_init(
                &init.children[mem.idx as usize],
                &mem.ty,
                &desg2,
                tok_loc,
                line_no,
                filename,
                src,
            )?;
            node = new_binary(NodeKind::Comma, node, rhs, tok_loc, line_no);
            current = mem.next.as_ref();
        }
        return Ok(node);
    }

    if ty.kind == TypeKind::Union {
        let desg2 = InitDesg {
            next: Some(Box::new(desg.clone())),
            idx: 0,
            member: Some(ty.members.as_ref().unwrap().as_ref().clone()),
            var: None,
        };
        return create_lvar_init(
            &init.children[0],
            &ty.members.as_ref().unwrap().ty,
            &desg2,
            tok_loc,
            line_no,
            filename,
            src,
        );
    }

    let lhs = init_desg_expr(desg, tok_loc, line_no, filename, src)?;
    if init.expr.is_none() {
        return Ok(new_node(NodeKind::NullExpr, tok_loc, line_no));
    }
    let rhs = init.expr.as_ref().unwrap().clone();
    Ok(new_binary(NodeKind::Assign, lhs, rhs, tok_loc, line_no))
}

#[allow(clippy::too_many_arguments)]
fn lvar_initializer(
    filename: &str,
    src: &str,
    tok: &Token,
    var_name: &str,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let tok_loc = tok.loc;
    let line_no = tok.line_no;

    let var_idx = locals
        .iter()
        .position(|v| v.name == var_name)
        .ok_or_else(|| format!("variable not found: {}", var_name))?;
    let old_ty = locals[var_idx].ty.clone();
    let (init, new_ty, tok) = initializer(
        filename,
        src,
        tok,
        &old_ty,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;
    locals[var_idx].ty = new_ty.clone();

    for scope in scope_stack.iter_mut().rev() {
        for vs in scope.iter_mut().rev() {
            if vs.name == var_name {
                if let Some(ref mut var) = vs.var {
                    var.ty = new_ty.clone();
                }
                break;
            }
        }
    }

    let var = locals[var_idx].clone();

    let mut lhs = new_node(NodeKind::Memzero, tok_loc, line_no);
    lhs.var = Some(Box::new(var.clone()));

    let desg = InitDesg {
        next: None,
        idx: 0,
        member: None,
        var: Some(var.clone()),
    };
    let rhs = create_lvar_init(&init, &new_ty, &desg, tok_loc, line_no, filename, src)?;
    Ok((new_binary(NodeKind::Comma, lhs, rhs, tok_loc, line_no), tok))
}

pub fn get_ident(src: &str, tok: &Token) -> Result<String, String> {
    if tok.kind != TokenKind::Ident {
        return Err(error_tok("<stdin>", src, tok, "expected an identifier"));
    }
    let name: String = src.chars().skip(tok.loc).take(tok.len).collect();
    Ok(name)
}

pub fn struct_members(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Option<Box<crate::Member>>, bool, Token), String> {
    let mut tok = tok.clone();
    let mut members: Vec<crate::Member> = Vec::new();
    let mut idx: i64 = 0;

    while !equal(src, &tok, "}") {
        let mut attr = VarAttr::default();
        let mut empty_scope: Vec<Vec<VarScope>> = vec![];
        let (basety, new_tok) = declspec(
            filename,
            src,
            &tok,
            tag_scope_stack,
            &mut empty_scope,
            Some(&mut attr),
        )?;
        tok = new_tok;
        let mut first = true;

        while !equal(src, &tok, ";") {
            if !first {
                tok = skip(filename, src, &tok, ",")?;
            }
            first = false;

            let (mem_ty, new_tok) = {
                let mut empty_scope: Vec<Vec<VarScope>> = vec![];
                declarator(
                    filename,
                    src,
                    &tok,
                    basety.clone(),
                    tag_scope_stack,
                    &mut empty_scope,
                )?
            };
            tok = new_tok;
            let mem_align = if attr.align > 0 {
                attr.align
            } else {
                mem_ty.align
            };
            let mem = crate::Member {
                next: None,
                ty: mem_ty.clone(),
                tok: Some(Box::new(token_snapshot(&tok))),
                name: mem_ty.name.clone(),
                idx,
                align: mem_align,
                offset: 0,
            };
            idx += 1;
            members.push(mem);
        }
        tok = skip(filename, src, &tok, ";")?;
    }

    let rest = tok.next.as_ref().unwrap().as_ref().clone();

    let mut is_flexible = false;
    if let Some(last) = members.last_mut()
        && last.ty.kind == TypeKind::Array
        && last.ty.array_len < 0
    {
        last.ty.array_len = 0;
        last.ty.size = 0;
        is_flexible = true;
    }

    if members.is_empty() {
        Ok((None, is_flexible, rest))
    } else {
        let mut current: Option<Box<crate::Member>> = None;
        for mem in members.into_iter().rev() {
            let mut m = mem;
            m.next = current;
            current = Some(Box::new(m));
        }
        Ok((current, is_flexible, rest))
    }
}

pub fn struct_union_decl(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Rc<RefCell<Type>>, Token), String> {
    let mut tok = tok.clone();

    let tag = if tok.kind == TokenKind::Ident {
        let tag_tok = tok.clone();
        tok = tok.next.as_ref().unwrap().as_ref().clone();
        Some(tag_tok)
    } else {
        None
    };

    if let Some(tag_tok) = &tag
        && !equal(src, &tok, "{")
    {
        let tag_name: String = src.chars().skip(tag_tok.loc).take(tag_tok.len).collect();
        if let Some(ty) = find_tag(tag_scope_stack, &tag_name) {
            return Ok((ty, tok));
        }

        let ty = Rc::new(RefCell::new(Type::new_struct()));
        ty.borrow_mut().size = -1;
        push_tag_scope(tag_scope_stack, tag_name, ty.clone());
        return Ok((ty, tok));
    }

    tok = skip(filename, src, &tok, "{")?;

    let ty_rc = if let Some(tag_tok) = &tag {
        let tag_name: String = src.chars().skip(tag_tok.loc).take(tag_tok.len).collect();
        if let Some(existing_ty) = find_tag_in_current_scope(tag_scope_stack, &tag_name) {
            existing_ty.clone()
        } else {
            let ty = Rc::new(RefCell::new(Type::new_struct()));
            push_tag_scope(tag_scope_stack, tag_name, ty.clone());
            ty
        }
    } else {
        Rc::new(RefCell::new(Type::new_struct()))
    };

    let (members, is_flexible, rest) = struct_members(filename, src, &tok, tag_scope_stack)?;

    {
        let mut ty = ty_rc.borrow_mut();
        ty.kind = TypeKind::Struct;
        ty.members = members;
        ty.size = 0;
        ty.align = 1;
        ty.is_flexible = is_flexible;
    }

    Ok((ty_rc, rest))
}

pub fn struct_decl(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Type, Token), String> {
    let (ty_rc, rest) = struct_union_decl(filename, src, tok, tag_scope_stack)?;
    ty_rc.borrow_mut().kind = TypeKind::Struct;

    if ty_rc.borrow().size < 0 {
        let mut ty = ty_rc.borrow().clone();
        ty.origin = Some(ty_rc.clone());
        return Ok((ty, rest));
    }

    let mut offset = 0;
    let mut max_align = 1;
    {
        let ty = ty_rc.borrow();
        let mut current = ty.members.as_ref();
        while let Some(mem) = current {
            offset = align_to(offset, mem.align);
            if max_align < mem.align {
                max_align = mem.align;
            }
            offset += mem.ty.size;
            current = mem.next.as_ref();
        }
    }

    let size = align_to(offset, max_align);
    {
        let mut ty = ty_rc.borrow_mut();
        ty.align = max_align;
        ty.size = size;
    }

    let mut offset = 0;
    {
        let mut ty = ty_rc.borrow_mut();
        let mut current = ty.members.as_mut();
        while let Some(mem) = current {
            offset = align_to(offset, mem.align);
            mem.offset = offset;
            offset += mem.ty.size;
            current = mem.next.as_mut();
        }
    }

    let mut ty = ty_rc.borrow().clone();
    ty.origin = Some(ty_rc.clone());
    Ok((ty, rest))
}

pub fn union_decl(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Type, Token), String> {
    let (ty_rc, rest) = struct_union_decl(filename, src, tok, tag_scope_stack)?;
    ty_rc.borrow_mut().kind = TypeKind::Union;

    if ty_rc.borrow().size < 0 {
        let mut ty = ty_rc.borrow().clone();
        ty.origin = Some(ty_rc.clone());
        return Ok((ty, rest));
    }

    let mut max_align = 1;
    let mut max_size = 0;
    {
        let ty = ty_rc.borrow();
        let mut current = ty.members.as_ref();
        while let Some(mem) = current {
            if max_align < mem.align {
                max_align = mem.align;
            }
            if max_size < mem.ty.size {
                max_size = mem.ty.size;
            }
            current = mem.next.as_ref();
        }
    }

    {
        let mut ty = ty_rc.borrow_mut();
        ty.align = max_align;
        ty.size = align_to(max_size, max_align);
    }

    let mut ty = ty_rc.borrow().clone();
    ty.origin = Some(ty_rc.clone());
    Ok((ty, rest))
}

pub fn enum_specifier(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut [Vec<TagScope>],
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let ty = Type::new_enum();

    let tag = if tok.kind == TokenKind::Ident {
        let tag_tok = tok.clone();
        let tok = tok.next.as_ref().unwrap().as_ref().clone();
        (Some(tag_tok), tok)
    } else {
        (None, tok.clone())
    };

    let (tag, mut tok) = (tag.0, tag.1);

    if let Some(tag_tok) = &tag
        && !equal(src, &tok, "{")
    {
        let tag_name: String = src.chars().skip(tag_tok.loc).take(tag_tok.len).collect();
        if let Some(ty) = find_tag(tag_scope_stack, &tag_name) {
            if ty.borrow().kind != TypeKind::Enum {
                return Err(error_tok(filename, src, tag_tok, "not an enum tag"));
            }
            return Ok((ty.borrow().clone(), tok));
        }
        return Err(error_tok(filename, src, tag_tok, "unknown enum type"));
    }

    tok = skip(filename, src, &tok, "{")?;

    let mut val: i64 = 0;
    let mut i = 0;

    loop {
        let (is_end, new_tok) = consume_end(src, &tok);
        if is_end {
            tok = new_tok;
            break;
        }
        if i > 0 {
            tok = skip(filename, src, &tok, ",")?;
        }
        i += 1;

        let name = get_ident(src, &tok)?;
        tok = tok.next.as_ref().unwrap().as_ref().clone();

        if equal(src, &tok, "=") {
            let (v, new_tok) = const_expr(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                tag_scope_stack,
                scope_stack,
            )?;
            val = v;
            tok = new_tok;
        }

        let scope = VarScope {
            name: name.clone(),
            var: None,
            type_def: None,
            enum_ty: Some(ty.clone()),
            enum_val: val,
        };

        if let Some(last_scope) = scope_stack.last_mut() {
            last_scope.push(scope);
        }

        val += 1;
    }

    if let Some(tag_tok) = tag {
        let tag_name: String = src.chars().skip(tag_tok.loc).take(tag_tok.len).collect();
        push_tag_scope(tag_scope_stack, tag_name, Rc::new(RefCell::new(ty.clone())));
    }

    Ok((ty, tok))
}

pub fn get_struct_member(
    filename: &str,
    ty: &Type,
    src: &str,
    tok: &Token,
) -> Result<crate::Member, String> {
    let mut current = ty.members.as_ref();
    while let Some(mem) = current {
        if let Some(name) = &mem.name
            && name.len == tok.len
        {
            let mem_name: String = src.chars().skip(name.loc).take(name.len).collect();
            let tok_name: String = src.chars().skip(tok.loc).take(tok.len).collect();
            if mem_name == tok_name {
                return Ok(mem.as_ref().clone());
            }
        }
        current = mem.next.as_ref();
    }
    Err(error_tok(filename, src, tok, "no such member"))
}

pub fn struct_ref(filename: &str, src: &str, lhs: Node, tok: &Token) -> Result<Node, String> {
    let mut lhs = lhs;
    add_type(&mut lhs);

    if lhs.ty.as_ref().unwrap().kind != TypeKind::Struct
        && lhs.ty.as_ref().unwrap().kind != TypeKind::Union
    {
        return Err(error_tok(filename, src, tok, "not a struct nor a union"));
    }

    let member = get_struct_member(filename, lhs.ty.as_ref().unwrap(), src, tok)?;
    let mut node = new_unary(NodeKind::Member, lhs, tok.loc, tok.line_no);
    node.member = Some(Box::new(member));
    Ok(node)
}

pub fn declspec(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
    mut attr: Option<&mut VarAttr>,
) -> Result<(Type, Token), String> {
    const VOID: i32 = 1 << 0;
    const BOOL: i32 = 1 << 2;
    const CHAR: i32 = 1 << 4;
    const SHORT: i32 = 1 << 6;
    const INT: i32 = 1 << 8;
    const LONG: i32 = 1 << 10;
    const OTHER: i32 = 1 << 12;
    const SHORT_INT: i32 = SHORT + INT;
    const LONG_INT: i32 = LONG + INT;
    const LONG_LONG: i32 = LONG + LONG;
    const LONG_LONG_INT: i32 = LONG_LONG + INT;

    let mut ty = Type::new_int();
    let mut counter = 0;
    let mut tok = tok.clone();

    while is_typename(src, &tok, scope_stack) {
        if equal(src, &tok, "typedef") || equal(src, &tok, "static") || equal(src, &tok, "extern") {
            if let Some(a) = attr.as_mut() {
                if equal(src, &tok, "typedef") {
                    a.is_typedef = true;
                } else if equal(src, &tok, "static") {
                    a.is_static = true;
                } else {
                    a.is_extern = true;
                }
                if a.is_typedef && a.is_static as i32 + a.is_extern as i32 > 1 {
                    return Err(error_tok(
                        filename,
                        src,
                        &tok,
                        "typedef may not be used together with static or extern",
                    ));
                }
            } else {
                return Err(error_tok(
                    filename,
                    src,
                    &tok,
                    "storage class specifier is not allowed in this context",
                ));
            }
            tok = *tok.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(src, &tok, "_Alignas") {
            if attr.is_none() {
                return Err(error_tok(
                    filename,
                    src,
                    &tok,
                    "_Alignas is not allowed in this context",
                ));
            }
            tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;

            if is_typename(src, &tok, scope_stack) {
                let (align_ty, new_tok) =
                    typename(filename, src, &tok, tag_scope_stack, scope_stack)?;
                tok = new_tok;
                if let Some(a) = attr.as_mut() {
                    a.align = align_ty.align;
                }
            } else {
                let (val, new_tok) = const_expr(filename, src, &tok, tag_scope_stack, scope_stack)?;
                tok = new_tok;
                if let Some(a) = attr.as_mut() {
                    a.align = val;
                }
            }
            tok = skip(filename, src, &tok, ")")?;
            continue;
        }

        let ty2 = find_typedef(scope_stack, &tok, src);
        if equal(src, &tok, "struct")
            || equal(src, &tok, "union")
            || equal(src, &tok, "enum")
            || ty2.is_some()
        {
            if counter > 0 {
                break;
            }

            if equal(src, &tok, "struct") {
                let (new_ty, new_tok) =
                    struct_decl(filename, src, tok.next.as_ref().unwrap(), tag_scope_stack)?;
                ty = new_ty;
                tok = new_tok;
            } else if equal(src, &tok, "union") {
                let (new_ty, new_tok) =
                    union_decl(filename, src, tok.next.as_ref().unwrap(), tag_scope_stack)?;
                ty = new_ty;
                tok = new_tok;
            } else if equal(src, &tok, "enum") {
                let (new_ty, new_tok) = enum_specifier(
                    filename,
                    src,
                    tok.next.as_ref().unwrap(),
                    tag_scope_stack,
                    scope_stack,
                )?;
                ty = new_ty;
                tok = new_tok;
            } else {
                ty = ty2.unwrap().borrow().clone();
                tok = *tok.next.as_ref().unwrap().clone();
            }
            counter += OTHER;
            continue;
        }

        if equal(src, &tok, "void") {
            counter += VOID;
        } else if equal(src, &tok, "_Bool") {
            counter += BOOL;
        } else if equal(src, &tok, "char") {
            counter += CHAR;
        } else if equal(src, &tok, "short") {
            counter += SHORT;
        } else if equal(src, &tok, "int") {
            counter += INT;
        } else if equal(src, &tok, "long") {
            counter += LONG;
        } else {
            unreachable!();
        }

        match counter {
            VOID => ty = Type::new_void(),
            BOOL => ty = Type::new_bool(),
            CHAR => ty = Type::new_char(),
            SHORT | SHORT_INT => ty = Type::new_short(),
            INT => ty = Type::new_int(),
            LONG | LONG_INT | LONG_LONG | LONG_LONG_INT => ty = Type::new_long(),
            _ => return Err(error_tok(filename, src, &tok, "invalid type")),
        }

        tok = *tok.next.as_ref().unwrap().clone();
    }

    Ok((ty, tok))
}

pub fn is_typename(src: &str, tok: &Token, scope_stack: &[Vec<VarScope>]) -> bool {
    equal(src, tok, "void")
        || equal(src, tok, "_Bool")
        || equal(src, tok, "char")
        || equal(src, tok, "short")
        || equal(src, tok, "int")
        || equal(src, tok, "long")
        || equal(src, tok, "struct")
        || equal(src, tok, "union")
        || equal(src, tok, "typedef")
        || equal(src, tok, "enum")
        || equal(src, tok, "static")
        || equal(src, tok, "extern")
        || equal(src, tok, "_Alignas")
        || find_typedef(scope_stack, tok, src).is_some()
}

pub fn get_number(tok: &Token) -> Result<i64, String> {
    if tok.kind != TokenKind::Num {
        return Err("expected a number".to_string());
    }
    Ok(tok.val)
}

pub fn is_function(src: &str, tok: &Token) -> Result<bool, String> {
    if equal(src, tok, ";") {
        return Ok(false);
    }

    let dummy = Type::new_int();
    let mut tag_scope_stack: Vec<Vec<TagScope>> = vec![Vec::new()];
    let mut empty_scope: Vec<Vec<VarScope>> = vec![];
    let (ty, _) = declarator("", src, tok, dummy, &mut tag_scope_stack, &mut empty_scope)?;
    Ok(ty.kind == TypeKind::Func)
}

fn write_buf(buf: &mut [u8], offset: usize, val: u64, sz: i64) {
    match sz {
        1 => buf[offset] = val as u8,
        2 => {
            let bytes = (val as u16).to_le_bytes();
            buf[offset..offset + 2].copy_from_slice(&bytes);
        }
        4 => {
            let bytes = (val as u32).to_le_bytes();
            buf[offset..offset + 4].copy_from_slice(&bytes);
        }
        8 => {
            let bytes = val.to_le_bytes();
            buf[offset..offset + 8].copy_from_slice(&bytes);
        }
        _ => unreachable!(),
    }
}

fn write_gvar_data(
    filename: &str,
    src: &str,
    init: &Initializer,
    ty: &Type,
    buf: &mut [u8],
    offset: usize,
    rel_head: &mut Option<Box<crate::Relocation>>,
) -> Result<(), String> {
    if ty.kind == TypeKind::Array {
        let base_ty = ty.base.as_ref().unwrap().borrow().clone();
        let sz = base_ty.size as usize;
        for i in 0..ty.array_len as usize {
            write_gvar_data(
                filename,
                src,
                &init.children[i],
                &base_ty,
                buf,
                offset + sz * i,
                rel_head,
            )?;
        }
        return Ok(());
    }

    if ty.kind == TypeKind::Struct {
        let mut current = ty.members.as_ref();
        while let Some(mem) = current {
            write_gvar_data(
                filename,
                src,
                &init.children[mem.idx as usize],
                &mem.ty,
                buf,
                offset + mem.offset as usize,
                rel_head,
            )?;
            current = mem.next.as_ref();
        }
        return Ok(());
    }

    if ty.kind == TypeKind::Union {
        let first_member_ty = &ty.members.as_ref().unwrap().ty;
        return write_gvar_data(
            filename,
            src,
            &init.children[0],
            first_member_ty,
            buf,
            offset,
            rel_head,
        );
    }

    if init.expr.is_none() {
        return Ok(());
    }

    let mut expr = init.expr.as_ref().unwrap().clone();
    let mut label: Option<String> = None;
    let val = eval2(filename, src, &mut expr, Some(&mut label))?;

    if label.is_none() {
        write_buf(buf, offset, val as u64, ty.size);
        return Ok(());
    }

    let rel = crate::Relocation {
        next: None,
        offset: offset as i64,
        label: label.unwrap(),
        addend: val,
    };

    if let Some(head) = rel_head {
        let mut cur = head;
        while cur.next.is_some() {
            cur = cur.next.as_mut().unwrap();
        }
        cur.next = Some(Box::new(rel));
    } else {
        *rel_head = Some(Box::new(rel));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn gvar_initializer(
    filename: &str,
    src: &str,
    tok: &Token,
    var: &mut Obj,
    globals: &mut Vec<Obj>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<Token, String> {
    let mut empty_locals: Vec<Obj> = Vec::new();
    let mut scope_stack_vec: Vec<Vec<VarScope>> = scope_stack.to_vec();
    let (init, new_ty, tok) = initializer(
        filename,
        src,
        tok,
        &var.ty,
        &mut empty_locals,
        globals,
        &mut scope_stack_vec,
        tag_scope_stack,
    )?;

    var.ty = new_ty;
    let mut buf = vec![0u8; var.ty.size as usize];
    let mut rel_head: Option<Box<crate::Relocation>> = None;
    write_gvar_data(filename, src, &init, &var.ty, &mut buf, 0, &mut rel_head)?;
    var.init_data = Some(buf);
    var.rel = rel_head;
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
pub fn global_variable(
    filename: &str,
    src: &str,
    tok: &Token,
    basety: Type,
    globals: &mut Vec<Obj>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
    attr: &VarAttr,
) -> Result<Token, String> {
    let mut tok = tok.clone();
    let mut first = true;

    while !equal(src, &tok, ";") {
        if !first {
            tok = skip(filename, src, &tok, ",")?;
        }
        first = false;

        let (ty, new_tok) = declarator(
            filename,
            src,
            &tok,
            basety.clone(),
            tag_scope_stack,
            scope_stack,
        )?;
        tok = new_tok;
        if ty.kind == TypeKind::Array && ty.array_len < 0 && !equal(src, &tok, "=") {
            return Err(error_tok(
                filename,
                src,
                &tok,
                "variable has incomplete type",
            ));
        }
        let name = get_ident(src, ty.name.as_ref().unwrap())?;
        let mut var = new_gvar(name, ty);
        var.is_definition = !attr.is_extern;
        var.is_static = attr.is_static;
        if attr.align > 0 {
            var.align = attr.align;
        }
        if equal(src, &tok, "=") {
            tok = gvar_initializer(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                &mut var,
                globals,
                tag_scope_stack,
                scope_stack,
            )?;
        }
        globals.push(var);
    }

    Ok(*tok.next.as_ref().unwrap().clone())
}

pub fn func_params(
    filename: &str,
    src: &str,
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let mut tok = tok.clone();

    if equal(src, &tok, "void") && tok.next.as_ref().is_some_and(|next| equal(src, next, ")")) {
        let func_ty = func_type(ty);
        let rest = tok
            .next
            .as_ref()
            .unwrap()
            .next
            .as_ref()
            .unwrap()
            .as_ref()
            .clone();
        return Ok((func_ty, rest));
    }

    let mut head = Type {
        kind: TypeKind::Int,
        size: 0,
        align: 0,
        base: None,
        name: None,
        return_ty: None,
        params: None,
        next: None,
        array_len: 0,
        members: None,
        origin: None,
        is_flexible: false,
        is_variadic: false,
    };
    let mut cur = &mut head;
    let mut first = true;
    let mut is_variadic = false;

    while !equal(src, &tok, ")") {
        if !first {
            tok = skip(filename, src, &tok, ",")?;
        }
        first = false;

        if equal(src, &tok, "...") {
            is_variadic = true;
            tok = tok.next.as_ref().unwrap().as_ref().clone();
            tok = skip(filename, src, &tok, ")")?;
            let mut func_ty = func_type(ty);
            func_ty.params = head.next;
            func_ty.is_variadic = is_variadic;
            return Ok((func_ty, tok));
        }

        let (basety, new_tok) = declspec(filename, src, &tok, tag_scope_stack, scope_stack, None)?;
        tok = new_tok;
        let (param_ty, new_tok) =
            declarator(filename, src, &tok, basety, tag_scope_stack, scope_stack)?;
        tok = new_tok;

        let param_ty = if param_ty.kind == TypeKind::Array {
            let name = param_ty.name.clone();
            let mut ptr_ty = Type::new_ptr(param_ty.base.unwrap().borrow().clone());
            ptr_ty.name = name;
            ptr_ty
        } else {
            param_ty
        };

        let param_copy = copy_type(&param_ty);
        cur.next = Some(Box::new(param_copy));
        cur = cur.next.as_mut().unwrap();
    }

    let mut func_ty = func_type(ty);
    func_ty.params = head.next;
    func_ty.is_variadic = is_variadic;
    let rest = tok.next.as_ref().unwrap().as_ref().clone();
    Ok((func_ty, rest))
}

pub fn array_dimensions(
    filename: &str,
    src: &str,
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    if equal(src, tok, "]") {
        let (ty, rest) = type_suffix(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        )?;
        return Ok((Type::new_array(ty, -1), rest));
    }

    let (sz, tok) = const_expr(filename, src, tok, tag_scope_stack, scope_stack)?;
    let tok = skip(filename, src, &tok, "]")?;
    let (ty, rest) = type_suffix(filename, src, &tok, ty, tag_scope_stack, scope_stack)?;
    Ok((Type::new_array(ty, sz), rest))
}

pub fn type_suffix(
    filename: &str,
    src: &str,
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    if equal(src, tok, "(") {
        return func_params(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        );
    }

    if equal(src, tok, "[") {
        return array_dimensions(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        );
    }

    Ok((ty, tok.clone()))
}

pub fn declarator(
    filename: &str,
    src: &str,
    tok: &Token,
    mut ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
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

    if equal(src, &tok, "(") {
        let start = tok.clone();
        let dummy = Type::new_int();
        let (_, tok) = declarator(
            filename,
            src,
            start.next.as_ref().unwrap(),
            dummy,
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ")")?;
        let (ty, rest) = type_suffix(filename, src, &tok, ty, tag_scope_stack, scope_stack)?;
        let (ty, _) = declarator(
            filename,
            src,
            start.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        )?;
        return Ok((ty, rest));
    }

    if tok.kind != TokenKind::Ident {
        return Err(error_tok(filename, src, &tok, "expected a variable name"));
    }

    let name_tok = tok.clone();
    let (ty, tok) = type_suffix(
        filename,
        src,
        tok.next.as_ref().unwrap(),
        ty,
        tag_scope_stack,
        scope_stack,
    )?;
    let mut ty = ty;
    ty.name = Some(Box::new(name_tok));
    Ok((ty, tok))
}

pub fn abstract_declarator(
    filename: &str,
    src: &str,
    tok: &Token,
    mut ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
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

    if equal(src, &tok, "(") {
        let start = tok.clone();
        let dummy = Type::new_int();
        let (_, tok) = abstract_declarator(
            filename,
            src,
            start.next.as_ref().unwrap(),
            dummy,
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ")")?;
        let (ty, rest) = type_suffix(filename, src, &tok, ty, tag_scope_stack, scope_stack)?;
        let (ty, _) = abstract_declarator(
            filename,
            src,
            start.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        )?;
        return Ok((ty, rest));
    }

    type_suffix(filename, src, &tok, ty, tag_scope_stack, scope_stack)
}

pub fn typename(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let (ty, tok) = declspec(filename, src, tok, tag_scope_stack, scope_stack, None)?;
    abstract_declarator(filename, src, &tok, ty, tag_scope_stack, scope_stack)
}

#[allow(clippy::too_many_arguments)]
pub fn declaration(
    filename: &str,
    src: &str,
    tok: &Token,
    basety: Type,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    attr: Option<&VarAttr>,
) -> Result<(Node, Token), String> {
    let mut tok = tok.clone();

    let mut head = Node {
        kind: NodeKind::Num,
        tok_loc: tok.loc,
        line_no: tok.line_no,
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
        func_ty: None,
        args: None,
        var: None,
        val: 0,
        member: None,
        label: None,
        unique_label: None,
        goto_next: None,
        brk_label: None,
        cont_label: None,
        case_next: None,
        default_case: None,
    };
    let mut cur = &mut head;
    let mut i = 0;

    while !equal(src, &tok, ";") {
        if i > 0 {
            tok = skip(filename, src, &tok, ",")?;
        }
        i += 1;

        let (ty, new_tok) = declarator(
            filename,
            src,
            &tok,
            basety.clone(),
            tag_scope_stack,
            scope_stack,
        )?;
        tok = new_tok;
        if ty.kind == TypeKind::Void {
            return Err(error_tok(
                filename,
                src,
                ty.name.as_ref().unwrap(),
                "variable declared void",
            ));
        }
        let name = get_ident(src, ty.name.as_ref().unwrap())?;

        if let Some(a) = attr
            && a.is_static
        {
            let mut var = new_anon_gvar(ty.clone());
            var.is_static = true;
            var.is_definition = true;
            if equal(src, &tok, "=") {
                tok = gvar_initializer(
                    filename,
                    src,
                    tok.next.as_ref().unwrap(),
                    &mut var,
                    globals,
                    tag_scope_stack,
                    scope_stack,
                )?;
            }
            globals.push(var.clone());
            scope_stack.last_mut().unwrap().push(VarScope {
                name,
                var: Some(var),
                type_def: None,
                enum_ty: None,
                enum_val: 0,
            });
            continue;
        }

        new_lvar(name.clone(), ty, locals, scope_stack);

        if let Some(a) = attr
            && a.align > 0
        {
            let var_idx = locals.iter().position(|v| v.name == name).unwrap();
            locals[var_idx].align = a.align;
        }

        if equal(src, &tok, "=") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let tok_next = tok.next.as_ref().unwrap().clone();
            let (expr_node, new_tok) = lvar_initializer(
                filename,
                src,
                &tok_next,
                &name,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            tok = new_tok;
            cur.next = Some(Box::new(new_unary(
                NodeKind::ExprStmt,
                expr_node,
                tok_loc,
                line_no,
            )));
            cur = cur.next.as_mut().unwrap();
        }

        let var_idx = locals.iter().position(|v| v.name == name).unwrap();
        if locals[var_idx].ty.size < 0 {
            return Err(error_tok(
                filename,
                src,
                locals[var_idx].ty.name.as_ref().unwrap(),
                "variable has incomplete type",
            ));
        }
        if locals[var_idx].ty.kind == TypeKind::Void {
            return Err(error_tok(
                filename,
                src,
                locals[var_idx].ty.name.as_ref().unwrap(),
                "variable declared void",
            ));
        }
    }

    let tok_loc = tok.loc;
    let line_no = tok.line_no;
    let mut node = new_node(NodeKind::Block, tok_loc, line_no);
    node.body = head.next;
    Ok((node, *tok.next.as_ref().unwrap().clone()))
}

pub fn parse_typedef(
    filename: &str,
    src: &str,
    tok: &Token,
    basety: Type,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<Token, String> {
    let mut tok = tok.clone();
    let mut first = true;

    while !equal(src, &tok, ";") {
        if !first {
            tok = skip(filename, src, &tok, ",")?;
        }
        first = false;

        let (ty, new_tok) = declarator(
            filename,
            src,
            &tok,
            basety.clone(),
            &mut Vec::new(),
            scope_stack,
        )?;
        tok = new_tok;
        let name = get_ident(src, ty.name.as_ref().unwrap())?;
        let type_def = if let Some(origin) = &ty.origin {
            origin.clone()
        } else {
            Rc::new(RefCell::new(ty))
        };
        scope_stack.last_mut().unwrap().push(VarScope {
            name,
            var: None,
            type_def: Some(type_def),
            enum_ty: None,
            enum_val: 0,
        });
    }

    Ok(*tok.next.as_ref().unwrap().clone())
}

pub fn create_param_lvars(
    src: &str,
    param: &Type,
    locals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
) {
    let mut current = Some(param);

    while let Some(p) = current {
        if let Some(name_tok) = &p.name {
            let name = get_ident(src, name_tok).unwrap();
            new_lvar(name, p.clone(), locals, scope_stack);
        }
        current = p.next.as_ref().map(|b| b.as_ref());
    }
}

fn resolve_goto_labels(filename: &str, src: &str, body: &mut Node) -> Result<(), String> {
    let mut gotos_vec: Vec<Node> = Vec::new();
    let mut labels_vec: Vec<Node> = Vec::new();

    let mut current = gotos_get();
    while let Some(node) = current {
        gotos_vec.push(node.as_ref().clone());
        current = node.goto_next;
    }

    let mut current = labels_get();
    while let Some(node) = current {
        labels_vec.push(node.as_ref().clone());
        current = node.goto_next;
    }

    gotos_set(None);
    labels_set(None);

    for goto in &mut gotos_vec {
        let mut found = false;
        for label in &labels_vec {
            if goto.label == label.label {
                set_unique_label(body, &goto.label, &label.unique_label);
                found = true;
                break;
            }
        }
        if !found {
            let label_name = goto.label.as_ref().unwrap();
            let tok = Token {
                kind: TokenKind::Ident,
                next: None,
                val: 0,
                loc: goto.tok_loc,
                len: label_name.len(),
                ty: None,
                str: None,
                line_no: goto.line_no,
            };
            return Err(error_tok(filename, src, &tok, "use of undeclared label"));
        }
    }

    Ok(())
}

fn set_unique_label(node: &mut Node, label: &Option<String>, unique_label: &Option<String>) {
    if node.kind == NodeKind::Goto && node.label == *label {
        node.unique_label = unique_label.clone();
    }
    if let Some(lhs) = &mut node.lhs {
        set_unique_label(lhs, label, unique_label);
    }
    if let Some(rhs) = &mut node.rhs {
        set_unique_label(rhs, label, unique_label);
    }
    if let Some(cond) = &mut node.cond {
        set_unique_label(cond, label, unique_label);
    }
    if let Some(then) = &mut node.then {
        set_unique_label(then, label, unique_label);
    }
    if let Some(els) = &mut node.els {
        set_unique_label(els, label, unique_label);
    }
    if let Some(init) = &mut node.init {
        set_unique_label(init, label, unique_label);
    }
    if let Some(inc) = &mut node.inc {
        set_unique_label(inc, label, unique_label);
    }
    if let Some(body) = &mut node.body {
        let mut n = body;
        loop {
            set_unique_label(n, label, unique_label);
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
            set_unique_label(n, label, unique_label);
            if let Some(next) = &mut n.next {
                n = next;
            } else {
                break;
            }
        }
    }
    if let Some(goto_next) = &mut node.goto_next {
        set_unique_label(goto_next, label, unique_label);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn function(
    filename: &str,
    src: &str,
    tok: &Token,
    basety: Type,
    globals: &mut Vec<Obj>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
    attr: &VarAttr,
) -> Result<(Obj, Token), String> {
    let (ty, tok) = declarator(filename, src, tok, basety, tag_scope_stack, scope_stack)?;
    let name = get_ident(src, ty.name.as_ref().unwrap())?;

    let mut fn_obj = new_gvar(name, ty.clone());
    fn_obj.is_function = true;

    let (is_definition, tok) = consume(src, &tok, ";");
    fn_obj.is_definition = !is_definition;
    fn_obj.is_static = attr.is_static;

    if !fn_obj.is_definition {
        return Ok((fn_obj, tok));
    }

    let mut locals: Vec<Obj> = Vec::new();
    let mut local_scope_stack: Vec<Vec<VarScope>> = scope_stack.to_vec();
    local_scope_stack.push(Vec::new());
    tag_scope_stack.push(Vec::new());

    if let Some(params) = &ty.params {
        create_param_lvars(src, params, &mut locals, &mut local_scope_stack);
    }

    fn_obj.params = locals.clone();

    let tok = skip(filename, src, &tok, "{")?;
    let return_ty = ty.return_ty.as_ref().map(|b| b.as_ref());
    let (mut body, tok) = compound_stmt(
        filename,
        src,
        &tok,
        &mut locals,
        globals,
        &mut local_scope_stack,
        tag_scope_stack,
        return_ty,
    )?;

    add_type(&mut body);
    resolve_goto_labels(filename, src, &mut body)?;

    fn_obj.body = Some(Box::new(body));
    fn_obj.locals = locals;

    tag_scope_stack.pop();

    Ok((fn_obj, tok))
}

#[allow(clippy::too_many_arguments)]
pub fn compound_stmt(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    return_ty: Option<&Type>,
) -> Result<(Node, Token), String> {
    let tok_loc = tok.loc;
    let line_no = tok.line_no;

    scope_stack.push(Vec::new());
    tag_scope_stack.push(Vec::new());

    let mut head = Node {
        kind: NodeKind::Num,
        tok_loc,
        line_no,
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
        func_ty: None,
        args: None,
        var: None,
        val: 0,
        member: None,
        label: None,
        unique_label: None,
        goto_next: None,
        brk_label: None,
        cont_label: None,
        case_next: None,
        default_case: None,
    };
    let mut cur = &mut head;

    let mut tok = tok.clone();
    while !equal(src, &tok, "}") {
        if is_typename(src, &tok, scope_stack) && !equal(src, tok.next.as_ref().unwrap(), ":") {
            let mut attr = VarAttr::default();
            let (basety, new_tok) = declspec(
                filename,
                src,
                &tok,
                tag_scope_stack,
                scope_stack,
                Some(&mut attr),
            )?;
            tok = new_tok;

            if attr.is_typedef {
                tok = parse_typedef(filename, src, &tok, basety, scope_stack)?;
                continue;
            }

            if is_function(src, &tok)? {
                let (_, new_tok) = function(
                    filename,
                    src,
                    &tok,
                    basety,
                    globals,
                    tag_scope_stack,
                    scope_stack,
                    &attr,
                )?;
                tok = new_tok;
                continue;
            }

            if attr.is_extern {
                tok = global_variable(
                    filename,
                    src,
                    &tok,
                    basety,
                    globals,
                    tag_scope_stack,
                    scope_stack,
                    &attr,
                )?;
                continue;
            }

            let (node, new_tok) = declaration(
                filename,
                src,
                &tok,
                basety,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
                Some(&attr),
            )?;
            tok = new_tok;
            cur.next = Some(Box::new(node));
            cur = cur.next.as_mut().unwrap();
        } else {
            let (node, new_tok) = stmt(
                filename,
                src,
                &tok,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
                return_ty,
            )?;
            tok = new_tok;
            cur.next = Some(Box::new(node));
            cur = cur.next.as_mut().unwrap();
        }
    }

    scope_stack.pop();
    tag_scope_stack.pop();

    let mut node = new_node(NodeKind::Block, tok_loc, line_no);
    node.body = head.next;
    Ok((node, *tok.next.as_ref().unwrap().clone()))
}

#[allow(clippy::too_many_arguments)]
pub fn stmt(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    return_ty: Option<&Type>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "return") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let tok = tok.next.as_ref().unwrap();
        let (consumed, tok) = consume(src, tok, ";");
        if consumed {
            let node = new_node(NodeKind::Return, tok_loc, line_no);
            return Ok((node, tok));
        }
        let (mut expr_node, tok) = expr(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ";")?;
        if let Some(ret_ty) = return_ty {
            add_type(&mut expr_node);
            expr_node = new_cast(expr_node, ret_ty.clone());
        }
        let node = new_unary(NodeKind::Return, expr_node, tok_loc, line_no);
        return Ok((node, tok));
    }
    if equal(src, tok, "if") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::If, tok_loc, line_no);
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;
        let (then, tok) = stmt(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.then = Some(Box::new(then));
        let mut tok = tok;
        if equal(src, &tok, "else") {
            let (els, new_tok) = stmt(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
                return_ty,
            )?;
            node.els = Some(Box::new(els));
            tok = new_tok;
        }
        return Ok((node, tok));
    }
    if equal(src, tok, "for") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::For, tok_loc, line_no);
        let mut tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;

        scope_stack.push(Vec::new());
        tag_scope_stack.push(Vec::new());

        let brk = brk_label_get();
        let cont = cont_label_get();
        let brk_name = new_unique_name();
        let cont_name = new_unique_name();
        brk_label_set(Some(brk_name.clone()));
        cont_label_set(Some(cont_name.clone()));
        node.brk_label = Some(brk_name);
        node.cont_label = Some(cont_name);

        if is_typename(src, &tok, scope_stack) {
            let (basety, new_tok) =
                declspec(filename, src, &tok, tag_scope_stack, scope_stack, None)?;
            tok = new_tok;
            let (init, new_tok) = declaration(
                filename,
                src,
                &tok,
                basety,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
                None,
            )?;
            node.init = Some(Box::new(init));
            tok = new_tok;
        } else {
            let (init, new_tok) = expr_stmt(
                filename,
                src,
                &tok,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node.init = Some(Box::new(init));
            tok = new_tok;
        }

        if !equal(src, &tok, ";") {
            let (cond, new_tok) = expr(
                filename,
                src,
                &tok,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node.cond = Some(Box::new(cond));
            tok = new_tok;
        }
        tok = skip(filename, src, &tok, ";")?;

        if !equal(src, &tok, ")") {
            let (inc, new_tok) = expr(
                filename,
                src,
                &tok,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node.inc = Some(Box::new(inc));
            tok = new_tok;
        }
        tok = skip(filename, src, &tok, ")")?;

        let (then, tok) = stmt(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.then = Some(Box::new(then));

        scope_stack.pop();
        tag_scope_stack.pop();
        brk_label_set(brk);
        cont_label_set(cont);

        return Ok((node, tok));
    }
    if equal(src, tok, "while") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::While, tok_loc, line_no);
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;

        let brk = brk_label_get();
        let cont = cont_label_get();
        let brk_name = new_unique_name();
        let cont_name = new_unique_name();
        brk_label_set(Some(brk_name.clone()));
        cont_label_set(Some(cont_name.clone()));
        node.brk_label = Some(brk_name);
        node.cont_label = Some(cont_name);

        let (then, tok) = stmt(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.then = Some(Box::new(then));
        brk_label_set(brk);
        cont_label_set(cont);
        return Ok((node, tok));
    }
    if equal(src, tok, "do") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Do, tok_loc, line_no);

        let brk = brk_label_get();
        let cont = cont_label_get();
        let brk_name = new_unique_name();
        let cont_name = new_unique_name();
        brk_label_set(Some(brk_name.clone()));
        cont_label_set(Some(cont_name.clone()));
        node.brk_label = Some(brk_name);
        node.cont_label = Some(cont_name);

        let (then, tok) = stmt(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.then = Some(Box::new(then));

        brk_label_set(brk);
        cont_label_set(cont);

        let tok = skip(filename, src, &tok, "while")?;
        let tok = skip(filename, src, &tok, "(")?;
        let (cond, tok) = expr(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;
        let tok = skip(filename, src, &tok, ";")?;
        return Ok((node, tok));
    }
    if equal(src, tok, "goto") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Goto, tok_loc, line_no);
        let label_tok = tok.next.as_ref().unwrap();
        node.label = Some(get_ident(src, label_tok)?);
        node.goto_next = gotos_get();
        gotos_set(Some(Box::new(node.clone())));
        let tok = skip(filename, src, label_tok.next.as_ref().unwrap(), ";")?;
        return Ok((node, tok));
    }
    if equal(src, tok, "break") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let brk = brk_label_get();
        brk_label_set(brk.clone());
        if brk.is_none() {
            return Err(error_tok(filename, src, tok, "stray break"));
        }
        let mut node = new_node(NodeKind::Goto, tok_loc, line_no);
        node.unique_label = brk;
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), ";")?;
        return Ok((node, tok));
    }
    if equal(src, tok, "continue") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let cont = cont_label_get();
        cont_label_set(cont.clone());
        if cont.is_none() {
            return Err(error_tok(filename, src, tok, "stray continue"));
        }
        let mut node = new_node(NodeKind::Goto, tok_loc, line_no);
        node.unique_label = cont;
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), ";")?;
        return Ok((node, tok));
    }
    if equal(src, tok, "switch") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Switch, tok_loc, line_no);
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node.cond = Some(Box::new(cond));
        let tok = skip(filename, src, &tok, ")")?;

        let sw = current_switch_get();
        let brk = brk_label_get();
        let brk_name = new_unique_name();
        node.brk_label = Some(brk_name.clone());
        brk_label_set(Some(brk_name));
        current_switch_set(Some(Box::new(node.clone())));

        let (then, tok) = stmt(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.then = Some(Box::new(then));

        let updated_sw = current_switch_get();
        if let Some(sw_node) = updated_sw {
            node.case_next = sw_node.case_next;
            node.default_case = sw_node.default_case;
        }

        current_switch_set(sw);
        brk_label_set(brk);
        return Ok((node, tok));
    }
    if equal(src, tok, "case") {
        let sw = current_switch_get();
        current_switch_set(sw.clone());
        if sw.is_none() {
            return Err(error_tok(filename, src, tok, "stray case"));
        }
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (val, new_tok) = const_expr(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &new_tok, ":")?;

        let mut node = new_node(NodeKind::Case, tok_loc, line_no);
        node.label = Some(new_unique_name());
        node.val = val;
        let (lhs, tok) = stmt(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.lhs = Some(Box::new(lhs));

        let mut sw_node = current_switch_get().unwrap();
        let mut link = new_case_link(&node);
        link.case_next = sw_node.case_next;
        sw_node.case_next = Some(Box::new(link));
        current_switch_set(Some(sw_node));
        return Ok((node, tok));
    }
    if equal(src, tok, "default") {
        let sw = current_switch_get();
        current_switch_set(sw.clone());
        if sw.is_none() {
            return Err(error_tok(filename, src, tok, "stray default"));
        }
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let tok = skip(filename, src, tok.next.as_ref().unwrap(), ":")?;

        let mut node = new_node(NodeKind::Case, tok_loc, line_no);
        node.label = Some(new_unique_name());
        let (lhs, tok) = stmt(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.lhs = Some(Box::new(lhs));

        let mut sw_node = current_switch_get().unwrap();
        sw_node.default_case = Some(Box::new(new_case_link(&node)));
        current_switch_set(Some(sw_node));
        return Ok((node, tok));
    }
    if tok.kind == TokenKind::Ident && equal(src, tok.next.as_ref().unwrap(), ":") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Label, tok_loc, line_no);
        node.label = Some(src.chars().skip(tok.loc).take(tok.len).collect());
        node.unique_label = Some(new_unique_name());
        let (lhs, tok) = stmt(
            filename,
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.lhs = Some(Box::new(lhs));
        node.goto_next = labels_get();
        labels_set(Some(Box::new(node.clone())));
        return Ok((node, tok));
    }
    if equal(src, tok, "{") {
        return compound_stmt(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        );
    }
    expr_stmt(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn expr_stmt(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, ";") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let tok = *tok.next.as_ref().unwrap().clone();
        return Ok((new_node(NodeKind::Block, tok_loc, line_no), tok));
    }
    let tok_loc = tok.loc;
    let line_no = tok.line_no;
    let (expr_node, tok) = expr(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;
    let tok = skip(filename, src, &tok, ";")?;
    let node = new_unary(NodeKind::ExprStmt, expr_node, tok_loc, line_no);
    Ok((node, tok))
}

pub fn eval(filename: &str, src: &str, node: &mut Node) -> Result<i64, String> {
    eval2(filename, src, node, None)
}

pub fn eval2(
    filename: &str,
    src: &str,
    node: &mut Node,
    label: Option<&mut Option<String>>,
) -> Result<i64, String> {
    add_type(node);

    match node.kind {
        NodeKind::Add => {
            let lhs = eval2(filename, src, node.lhs.as_mut().unwrap(), label)?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs.wrapping_add(rhs))
        }
        NodeKind::Sub => {
            let lhs = eval2(filename, src, node.lhs.as_mut().unwrap(), label)?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs.wrapping_sub(rhs))
        }
        NodeKind::Mul => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs.wrapping_mul(rhs))
        }
        NodeKind::Div => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs.wrapping_div(rhs))
        }
        NodeKind::Neg => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            Ok(-lhs)
        }
        NodeKind::Mod => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs % rhs)
        }
        NodeKind::BitAnd => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs & rhs)
        }
        NodeKind::BitOr => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs | rhs)
        }
        NodeKind::BitXor => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs ^ rhs)
        }
        NodeKind::Shl => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs << rhs)
        }
        NodeKind::Shr => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok(lhs >> rhs)
        }
        NodeKind::Eq => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok((lhs == rhs) as i64)
        }
        NodeKind::Ne => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok((lhs != rhs) as i64)
        }
        NodeKind::Lt => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok((lhs < rhs) as i64)
        }
        NodeKind::Le => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok((lhs <= rhs) as i64)
        }
        NodeKind::Cond => {
            let cond = eval(filename, src, node.cond.as_mut().unwrap())?;
            if cond != 0 {
                eval2(filename, src, node.then.as_mut().unwrap(), label)
            } else {
                eval2(filename, src, node.els.as_mut().unwrap(), label)
            }
        }
        NodeKind::Comma => eval2(filename, src, node.rhs.as_mut().unwrap(), label),
        NodeKind::Not => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            Ok((lhs == 0) as i64)
        }
        NodeKind::BitNot => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            Ok(!lhs)
        }
        NodeKind::LogAnd => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok((lhs != 0 && rhs != 0) as i64)
        }
        NodeKind::LogOr => {
            let lhs = eval(filename, src, node.lhs.as_mut().unwrap())?;
            let rhs = eval(filename, src, node.rhs.as_mut().unwrap())?;
            Ok((lhs != 0 || rhs != 0) as i64)
        }
        NodeKind::Cast => {
            let val = eval2(filename, src, node.lhs.as_mut().unwrap(), label)?;
            let ty = node.ty.as_ref().unwrap();
            if is_integer(ty) {
                match ty.size {
                    1 => Ok((val as u8) as i64),
                    2 => Ok((val as u16) as i64),
                    4 => Ok((val as u32) as i64),
                    _ => Ok(val),
                }
            } else if ty.kind == TypeKind::Ptr {
                Ok(val)
            } else {
                Err(error_at(
                    filename,
                    src,
                    node.tok_loc,
                    "not a compile-time constant",
                ))
            }
        }
        NodeKind::Addr => eval_rval(filename, src, node.lhs.as_mut().unwrap(), label),
        NodeKind::Member => {
            if label.is_none() {
                return Err(error_at(
                    filename,
                    src,
                    node.tok_loc,
                    "not a compile-time constant",
                ));
            }
            let ty = node.ty.as_ref().unwrap();
            if ty.kind != TypeKind::Array {
                return Err(error_at(filename, src, node.tok_loc, "invalid initializer"));
            }
            let offset = eval_rval(filename, src, node.lhs.as_mut().unwrap(), label)?
                + node.member.as_ref().unwrap().offset;
            Ok(offset)
        }
        NodeKind::Var => {
            if label.is_none() {
                return Err(error_at(
                    filename,
                    src,
                    node.tok_loc,
                    "not a compile-time constant",
                ));
            }
            let var = node.var.as_ref().unwrap();
            let ty = &var.ty;
            if ty.kind != TypeKind::Array && ty.kind != TypeKind::Func {
                return Err(error_at(filename, src, node.tok_loc, "invalid initializer"));
            }
            if let Some(l) = label {
                *l = Some(var.name.clone());
            }
            Ok(0)
        }
        NodeKind::Num => Ok(node.val),
        _ => Err(error_at(
            filename,
            src,
            node.tok_loc,
            "not a compile-time constant",
        )),
    }
}

fn eval_rval(
    filename: &str,
    src: &str,
    node: &mut Node,
    label: Option<&mut Option<String>>,
) -> Result<i64, String> {
    match node.kind {
        NodeKind::Var => {
            let var = node.var.as_ref().unwrap();
            if var.is_local {
                return Err(error_at(
                    filename,
                    src,
                    node.tok_loc,
                    "not a compile-time constant",
                ));
            }
            if let Some(l) = label {
                *l = Some(var.name.clone());
            }
            Ok(0)
        }
        NodeKind::Deref => eval2(filename, src, node.lhs.as_mut().unwrap(), label),
        NodeKind::Member => {
            let offset = eval_rval(filename, src, node.lhs.as_mut().unwrap(), label)?
                + node.member.as_ref().unwrap().offset;
            Ok(offset)
        }
        _ => Err(error_at(filename, src, node.tok_loc, "invalid initializer")),
    }
}

pub fn const_expr(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut [Vec<TagScope>],
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(i64, Token), String> {
    let mut empty_locals: Vec<Obj> = Vec::new();
    let mut empty_globals: Vec<Obj> = Vec::new();
    let mut tag_scope_stack = tag_scope_stack.to_vec();
    let mut scope_stack = scope_stack.to_owned();
    let mut node = conditional(
        filename,
        src,
        tok,
        &mut empty_locals,
        &mut empty_globals,
        &mut scope_stack,
        &mut tag_scope_stack,
    )?;
    let val = eval(filename, src, &mut node.0)?;
    Ok((val, node.1))
}

pub fn expr(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (node, tok) = assign(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    if equal(src, &tok, ",") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = expr(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((
            new_binary(NodeKind::Comma, node, rhs, tok_loc, line_no),
            tok,
        ));
    }

    Ok((node, tok))
}

fn to_assign(
    mut binary: Node,
    locals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
) -> Node {
    add_type(binary.lhs.as_mut().unwrap());
    add_type(binary.rhs.as_mut().unwrap());

    let tok_loc = binary.tok_loc;
    let line_no = binary.line_no;
    let lhs_ty = binary.lhs.as_ref().unwrap().ty.as_ref().unwrap().clone();

    let var = new_lvar(String::new(), pointer_to(lhs_ty), locals, scope_stack);

    let expr1 = new_binary(
        NodeKind::Assign,
        new_var_node(var.clone(), tok_loc, line_no),
        new_unary(
            NodeKind::Addr,
            binary.lhs.as_ref().unwrap().as_ref().clone(),
            tok_loc,
            line_no,
        ),
        tok_loc,
        line_no,
    );

    let deref_var = new_unary(
        NodeKind::Deref,
        new_var_node(var.clone(), tok_loc, line_no),
        tok_loc,
        line_no,
    );

    let op_node = new_binary(
        binary.kind,
        new_unary(
            NodeKind::Deref,
            new_var_node(var, tok_loc, line_no),
            tok_loc,
            line_no,
        ),
        binary.rhs.as_ref().unwrap().as_ref().clone(),
        tok_loc,
        line_no,
    );

    let expr2 = new_binary(NodeKind::Assign, deref_var, op_node, tok_loc, line_no);

    new_binary(NodeKind::Comma, expr1, expr2, tok_loc, line_no)
}

#[allow(clippy::too_many_arguments)]
fn new_inc_dec(
    node: Node,
    tok_loc: usize,
    line_no: usize,
    addend: i64,
    filename: &str,
    src: &str,
    locals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
) -> Result<Node, String> {
    let mut node = node;
    add_type(&mut node);
    let ty = node.ty.as_ref().unwrap().clone();

    let binary = new_add(
        node,
        new_num(addend, tok_loc, line_no),
        tok_loc,
        line_no,
        filename,
        src,
    )?;
    let assigned = to_assign(binary, locals, scope_stack);
    let result = new_add(
        assigned,
        new_num(-addend, tok_loc, line_no),
        tok_loc,
        line_no,
        filename,
        src,
    )?;
    Ok(new_cast(result, ty))
}

pub fn assign(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, tok) = conditional(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;
    if equal(src, &tok, "=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::Assign, node, rhs, tok_loc, line_no);
        return Ok((node, tok));
    }

    if equal(src, &tok, "+=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_add(node, rhs, tok_loc, line_no, filename, src)?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "-=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_sub(node, rhs, tok_loc, line_no, filename, src)?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "*=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Mul, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "/=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Div, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "%=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Mod, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "&=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::BitAnd, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "|=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::BitOr, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "^=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::BitXor, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, "<<=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Shl, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, &tok, ">>=") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Shr, node, rhs, tok_loc, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    Ok((node, tok))
}

#[allow(clippy::too_many_arguments)]
pub fn conditional(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (cond, mut tok) = logor(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    if !equal(src, &tok, "?") {
        return Ok((cond, tok));
    }

    let tok_loc = tok.loc;
    let line_no = tok.line_no;
    let (then, new_tok) = expr(
        filename,
        src,
        tok.next.as_ref().unwrap(),
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;
    tok = skip(filename, src, &new_tok, ":")?;

    let (els, tok) = conditional(
        filename,
        src,
        &tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    let mut node = new_node(NodeKind::Cond, tok_loc, line_no);
    node.cond = Some(Box::new(cond));
    node.then = Some(Box::new(then));
    node.els = Some(Box::new(els));
    Ok((node, tok))
}

pub fn logor(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = logand(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    while equal(src, &tok, "||") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, new_tok) = logand(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::LogOr, node, rhs, tok_loc, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn logand(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = bitor(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    while equal(src, &tok, "&&") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, new_tok) = bitor(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::LogAnd, node, rhs, tok_loc, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn bitor(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = bitxor(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    while equal(src, &tok, "|") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, new_tok) = bitxor(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::BitOr, node, rhs, tok_loc, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn bitxor(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = bitand(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    while equal(src, &tok, "^") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, new_tok) = bitand(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::BitXor, node, rhs, tok_loc, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn bitand(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = equality(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    while equal(src, &tok, "&") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (rhs, new_tok) = equality(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::BitAnd, node, rhs, tok_loc, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn equality(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = relational(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    loop {
        if equal(src, &tok, "==") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = relational(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Eq, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "!=") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = relational(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Ne, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn relational(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = shift(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    loop {
        if equal(src, &tok, "<") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = shift(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Lt, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "<=") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = shift(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Le, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (lhs, new_tok) = shift(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Lt, lhs, node, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">=") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (lhs, new_tok) = shift(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Le, lhs, node, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn shift(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = add(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    loop {
        if equal(src, &tok, "<<") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = add(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Shl, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">>") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = add(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Shr, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn add(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = mul(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    loop {
        if equal(src, &tok, "+") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = mul(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_add(node, rhs, tok_loc, line_no, filename, src)?;
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "-") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = mul(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_sub(node, rhs, tok_loc, line_no, filename, src)?;
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn mul(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = cast(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    loop {
        if equal(src, &tok, "*") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = cast(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Mul, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "/") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = cast(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Div, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "%") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (rhs, new_tok) = cast(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Mod, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn cast(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "(") && is_typename(src, tok.next.as_ref().unwrap(), scope_stack) {
        let start = tok;
        let tok_loc = tok.loc;
        let (ty, new_tok) = typename(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &new_tok, ")")?;

        if equal(src, &tok, "{") {
            return unary(
                filename,
                src,
                start,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            );
        }

        let (node, tok) = cast(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let mut node = new_cast(node, ty);
        node.tok_loc = tok_loc;
        return Ok((node, tok));
    }

    unary(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )
}

pub fn unary(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "+") {
        return cast(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        );
    }

    if equal(src, tok, "-") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((new_unary(NodeKind::Neg, node, tok_loc, line_no), tok));
    }

    if equal(src, tok, "&") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((new_unary(NodeKind::Addr, node, tok_loc, line_no), tok));
    }

    if equal(src, tok, "*") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (mut node, tok) = cast(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        add_type(&mut node);
        let lhs_ty = node.ty.as_ref().unwrap();
        if (lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array)
            && lhs_ty.base.as_ref().unwrap().borrow().kind == TypeKind::Void
        {
            return Err(error_at(
                filename,
                src,
                tok_loc,
                "dereferencing a void pointer",
            ));
        }
        return Ok((new_unary(NodeKind::Deref, node, tok_loc, line_no), tok));
    }

    if equal(src, tok, "!") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((new_unary(NodeKind::Not, node, tok_loc, line_no), tok));
    }

    if equal(src, tok, "~") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((new_unary(NodeKind::BitNot, node, tok_loc, line_no), tok));
    }

    if equal(src, tok, "++") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (node, tok) = unary(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_add(
            node,
            new_num(1, tok_loc, line_no),
            tok_loc,
            line_no,
            filename,
            src,
        )?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(src, tok, "--") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (node, tok) = unary(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_sub(
            node,
            new_num(1, tok_loc, line_no),
            tok_loc,
            line_no,
            filename,
            src,
        )?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    postfix(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn postfix(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "(") && is_typename(src, tok.next.as_ref().unwrap(), scope_stack) {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (ty, tok) = typename(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ")")?;

        if scope_stack.len() <= 1 {
            let mut var = new_anon_gvar(ty);
            let tok = gvar_initializer(
                filename,
                src,
                &tok,
                &mut var,
                globals,
                tag_scope_stack,
                scope_stack,
            )?;
            globals.push(var.clone());
            return Ok((new_var_node(var, tok_loc, line_no), tok));
        }

        let var = new_lvar(String::new(), ty, locals, scope_stack);
        let (lhs, tok) = lvar_initializer(
            filename,
            src,
            &tok,
            &var.name,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let rhs = new_var_node(var, tok_loc, line_no);
        return Ok((new_binary(NodeKind::Comma, lhs, rhs, tok_loc, line_no), tok));
    }

    let (mut node, mut tok) = primary(
        filename,
        src,
        tok,
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;

    loop {
        if equal(src, &tok, "[") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (idx, new_tok) = expr(
                filename,
                src,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            tok = skip(filename, src, &new_tok, "]")?;
            node = new_unary(
                NodeKind::Deref,
                new_add(node, idx, tok_loc, line_no, filename, src)?,
                tok_loc,
                line_no,
            );
            continue;
        }

        if equal(src, &tok, ".") {
            let tok_next = tok.next.as_ref().unwrap();
            node = struct_ref(filename, src, node, tok_next)?;
            tok = *tok_next.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(src, &tok, "->") {
            node = new_unary(NodeKind::Deref, node, tok.loc, tok.line_no);
            let tok_next = tok.next.as_ref().unwrap();
            node = struct_ref(filename, src, node, tok_next)?;
            tok = *tok_next.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(src, &tok, "++") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            node = new_inc_dec(
                node,
                tok_loc,
                line_no,
                1,
                filename,
                src,
                locals,
                scope_stack,
            )?;
            tok = *tok.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(src, &tok, "--") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            node = new_inc_dec(
                node,
                tok_loc,
                line_no,
                -1,
                filename,
                src,
                locals,
                scope_stack,
            )?;
            tok = *tok.next.as_ref().unwrap().clone();
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn funcall(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let tok_loc = tok.loc;
    let line_no = tok.line_no;
    let funcname: String = src.chars().skip(tok.loc).take(tok.len).collect();

    let sc = find_var(scope_stack, globals, &funcname)
        .ok_or_else(|| error_tok(filename, src, tok, "implicit declaration of a function"))?;

    let var = sc
        .var
        .ok_or_else(|| error_tok(filename, src, tok, "implicit declaration of a function"))?;

    if var.ty.kind != TypeKind::Func {
        return Err(error_tok(filename, src, tok, "not a function"));
    }

    let ty = var.ty.clone();
    let return_ty = var.ty.return_ty.as_ref().unwrap().as_ref().clone();
    let mut param_ty = var.ty.params.clone();

    let mut tok = skip(filename, src, tok.next.as_ref().unwrap(), "(")?;

    let mut head = Node {
        kind: NodeKind::Num,
        tok_loc,
        line_no,
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
        func_ty: None,
        args: None,
        var: None,
        val: 0,
        member: None,
        label: None,
        unique_label: None,
        goto_next: None,
        brk_label: None,
        cont_label: None,
        case_next: None,
        default_case: None,
    };
    let mut cur = &mut head;

    while !equal(src, &tok, ")") {
        if cur.tok_loc != tok_loc || cur.kind != NodeKind::Num {
            tok = skip(filename, src, &tok, ",")?;
        }
        let (mut arg, new_tok) = assign(
            filename,
            src,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        tok = new_tok;
        add_type(&mut arg);

        if let Some(pt) = param_ty {
            if pt.kind == TypeKind::Struct || pt.kind == TypeKind::Union {
                return Err(error_tok(
                    filename,
                    src,
                    &tok,
                    "passing struct or union is not supported yet",
                ));
            }
            arg = new_cast(arg, pt.as_ref().clone());
            param_ty = pt.next.clone();
        }

        cur.next = Some(Box::new(arg));
        cur = cur.next.as_mut().unwrap();
    }

    let tok = skip(filename, src, &tok, ")")?;

    let mut node = new_node(NodeKind::FuncCall, tok_loc, line_no);
    node.funcname = Some(funcname);
    node.func_ty = Some(ty);
    node.ty = Some(return_ty);
    node.args = head.next;
    Ok((node, tok))
}

pub fn primary(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(src, tok, "(") && equal(src, tok.next.as_ref().unwrap(), "{") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (body, tok) = compound_stmt(
            filename,
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            None,
        )?;
        let tok = skip(filename, src, &tok, ")")?;
        let mut node = new_node(NodeKind::StmtExpr, tok_loc, line_no);
        node.body = body.body;
        return Ok((node, tok));
    }

    if equal(src, tok, "(") {
        let (node, tok) = expr(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ")")?;
        return Ok((node, tok));
    }

    if equal(src, tok, "sizeof")
        && equal(src, tok.next.as_ref().unwrap(), "(")
        && is_typename(
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            scope_stack,
        )
    {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (ty, tok) = typename(
            filename,
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ")")?;
        return Ok((new_num(ty.size, tok_loc, line_no), tok));
    }

    if equal(src, tok, "sizeof") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (mut node, tok) = unary(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        add_type(&mut node);
        let size = node.ty.as_ref().unwrap().size;
        return Ok((new_num(size, tok_loc, line_no), tok));
    }

    if equal(src, tok, "_Alignof")
        && equal(src, tok.next.as_ref().unwrap(), "(")
        && is_typename(
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            scope_stack,
        )
    {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (ty, tok) = typename(
            filename,
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ")")?;
        return Ok((new_num(ty.align, tok_loc, line_no), tok));
    }

    if equal(src, tok, "_Alignof") {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let (mut node, tok) = unary(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        add_type(&mut node);
        let align = node.ty.as_ref().unwrap().align;
        return Ok((new_num(align, tok_loc, line_no), tok));
    }

    if tok.kind == TokenKind::Ident {
        if equal(src, tok.next.as_ref().unwrap(), "(") {
            return funcall(
                filename,
                src,
                tok,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            );
        }

        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let funcname: String = src.chars().skip(tok.loc).take(tok.len).collect();

        let sc = find_var(scope_stack, globals, &funcname)
            .ok_or_else(|| error_tok(filename, src, tok, "undefined variable"))?;

        if sc.var.is_none() && sc.enum_ty.is_none() {
            return Err(error_tok(filename, src, tok, "undefined variable"));
        }

        let node = if let Some(var) = sc.var {
            new_var_node(var, tok_loc, line_no)
        } else {
            new_num(sc.enum_val, tok_loc, line_no)
        };
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    if tok.kind == TokenKind::Str {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let str_content = tok.str.as_ref().unwrap();
        let ty = tok.ty.as_ref().unwrap().clone();
        let var = new_string_literal(str_content, ty);
        let node = new_var_node(var.clone(), tok_loc, line_no);
        globals.push(var);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    if tok.kind == TokenKind::Num {
        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let node = new_num(tok.val, tok_loc, line_no);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    Err(error_tok(filename, src, tok, "expected an expression"))
}

pub fn pointer_to(base: Type) -> Type {
    if let Some(origin) = &base.origin {
        Type::new_ptr_shared(origin.clone())
    } else {
        Type::new_ptr(base)
    }
}

pub fn func_type(return_ty: Type) -> Type {
    Type {
        kind: TypeKind::Func,
        size: 0,
        align: 0,
        base: None,
        name: None,
        return_ty: Some(Box::new(return_ty)),
        params: None,
        next: None,
        array_len: 0,
        members: None,
        origin: None,
        is_flexible: false,
        is_variadic: false,
    }
}

pub fn is_integer(ty: &Type) -> bool {
    ty.kind == TypeKind::Bool
        || ty.kind == TypeKind::Char
        || ty.kind == TypeKind::Short
        || ty.kind == TypeKind::Int
        || ty.kind == TypeKind::Long
        || ty.kind == TypeKind::Enum
}

pub fn copy_type(ty: &Type) -> Type {
    ty.clone()
}

fn copy_struct_type(ty: &Type) -> Type {
    let mut ty = copy_type(ty);

    let mut new_members: Vec<crate::Member> = Vec::new();
    let mut current = ty.members.as_ref();
    while let Some(mem) = current {
        new_members.push(mem.as_ref().clone());
        current = mem.next.as_ref();
    }

    let mut members: Option<Box<crate::Member>> = None;
    for mem in new_members.into_iter().rev() {
        let mut m = mem;
        m.next = members;
        members = Some(Box::new(m));
    }
    ty.members = members;

    ty
}

pub fn get_common_type(ty1: &Type, ty2: &Type) -> Type {
    if let Some(base) = &ty1.base {
        return Type::new_ptr(base.borrow().clone());
    }
    if ty1.size == 8 || ty2.size == 8 {
        return Type::new_long();
    }
    Type::new_int()
}

pub fn usual_arith_conv(lhs: &mut Node, rhs: &mut Node) {
    let ty = get_common_type(lhs.ty.as_ref().unwrap(), rhs.ty.as_ref().unwrap());
    *lhs = new_cast(lhs.clone(), ty.clone());
    *rhs = new_cast(rhs.clone(), ty);
}

pub fn add_type(node: &mut Node) {
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
        NodeKind::Num => {
            node.ty = if node.val == (node.val as i32) as i64 {
                Some(Type::new_int())
            } else {
                Some(Type::new_long())
            };
        }
        NodeKind::Add
        | NodeKind::Sub
        | NodeKind::Mul
        | NodeKind::Div
        | NodeKind::Mod
        | NodeKind::BitAnd
        | NodeKind::BitOr
        | NodeKind::BitXor
        | NodeKind::Shl
        | NodeKind::Shr => {
            usual_arith_conv(node.lhs.as_mut().unwrap(), node.rhs.as_mut().unwrap());
            node.ty = node.lhs.as_ref().unwrap().ty.clone();
        }
        NodeKind::Neg => {
            let ty = get_common_type(
                &Type::new_int(),
                node.lhs.as_ref().unwrap().ty.as_ref().unwrap(),
            );
            node.lhs = Some(Box::new(new_cast(
                node.lhs.as_ref().unwrap().as_ref().clone(),
                ty.clone(),
            )));
            node.ty = Some(ty);
        }
        NodeKind::Assign => {
            let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
            if lhs_ty.kind == TypeKind::Array {
                node.ty = Some(Type::new_int());
            } else {
                if lhs_ty.kind != TypeKind::Struct && lhs_ty.kind != TypeKind::Union {
                    node.rhs = Some(Box::new(new_cast(
                        node.rhs.as_ref().unwrap().as_ref().clone(),
                        lhs_ty.clone(),
                    )));
                }
                node.ty = Some(lhs_ty.clone());
            }
        }
        NodeKind::Eq | NodeKind::Ne | NodeKind::Lt | NodeKind::Le => {
            usual_arith_conv(node.lhs.as_mut().unwrap(), node.rhs.as_mut().unwrap());
            node.ty = Some(Type::new_int());
        }
        NodeKind::FuncCall => {
            node.ty = Some(Type::new_long());
        }
        NodeKind::Not | NodeKind::LogAnd | NodeKind::LogOr => {
            node.ty = Some(Type::new_int());
        }
        NodeKind::BitNot => {
            node.ty = node.lhs.as_ref().unwrap().ty.clone();
        }
        NodeKind::Return
        | NodeKind::If
        | NodeKind::For
        | NodeKind::While
        | NodeKind::Do
        | NodeKind::Block
        | NodeKind::ExprStmt
        | NodeKind::Cast
        | NodeKind::Goto
        | NodeKind::Label
        | NodeKind::Switch
        | NodeKind::Case
        | NodeKind::NullExpr
        | NodeKind::Memzero => {}
        NodeKind::Var => {
            node.ty = Some(node.var.as_ref().unwrap().ty.clone());
        }
        NodeKind::Cond => {
            let then_ty = node.then.as_ref().unwrap().ty.as_ref().unwrap();
            let els_ty = node.els.as_ref().unwrap().ty.as_ref().unwrap();
            if then_ty.kind == TypeKind::Void || els_ty.kind == TypeKind::Void {
                node.ty = Some(Type::new_void());
            } else {
                usual_arith_conv(node.then.as_mut().unwrap(), node.els.as_mut().unwrap());
                node.ty = node.then.as_ref().unwrap().ty.clone();
            }
        }
        NodeKind::Comma => {
            node.ty = node.rhs.as_ref().unwrap().ty.clone();
        }
        NodeKind::Member => {
            node.ty = Some(node.member.as_ref().unwrap().ty.clone());
        }
        NodeKind::Addr => {
            let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
            if lhs_ty.kind == TypeKind::Array {
                node.ty = Some(Type::new_ptr(
                    lhs_ty.base.as_ref().unwrap().borrow().clone(),
                ));
            } else {
                node.ty = Some(Type::new_ptr(lhs_ty.clone()));
            }
        }
        NodeKind::Deref => {
            let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
            if lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array {
                node.ty = Some(lhs_ty.base.as_ref().unwrap().borrow().clone());
            } else {
                node.ty = Some(Type::new_int());
            }
        }
        NodeKind::StmtExpr => {
            if let Some(body) = &node.body {
                let mut stmt = body.as_ref();
                while let Some(next) = &stmt.next {
                    stmt = next.as_ref();
                }
                if stmt.kind == NodeKind::ExprStmt {
                    node.ty = stmt.lhs.as_ref().unwrap().ty.clone();
                }
            }
        }
    }
}

pub fn new_add(
    lhs: Node,
    rhs: Node,
    tok_loc: usize,
    line_no: usize,
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
        return Ok(new_binary(NodeKind::Add, lhs, rhs, tok_loc, line_no));
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

    let base_size = lhs
        .ty
        .as_ref()
        .unwrap()
        .base
        .as_ref()
        .unwrap()
        .borrow()
        .size;
    let rhs = new_binary(
        NodeKind::Mul,
        rhs,
        new_long(base_size, tok_loc, line_no),
        tok_loc,
        line_no,
    );
    Ok(new_binary(NodeKind::Add, lhs, rhs, tok_loc, line_no))
}

pub fn new_sub(
    lhs: Node,
    rhs: Node,
    tok_loc: usize,
    line_no: usize,
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
        return Ok(new_binary(NodeKind::Sub, lhs, rhs, tok_loc, line_no));
    }

    if (lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array) && is_integer(rhs_ty) {
        let lhs_ty_clone = lhs.ty.clone();
        let base_size = lhs
            .ty
            .as_ref()
            .unwrap()
            .base
            .as_ref()
            .unwrap()
            .borrow()
            .size;
        let mut rhs = new_binary(
            NodeKind::Mul,
            rhs,
            new_long(base_size, tok_loc, line_no),
            tok_loc,
            line_no,
        );
        add_type(&mut rhs);
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc, line_no);
        node.ty = Some(Type::new_ptr(
            lhs_ty_clone
                .unwrap()
                .base
                .as_ref()
                .unwrap()
                .borrow()
                .clone(),
        ));
        return Ok(node);
    }

    if (lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array)
        && (rhs_ty.kind == TypeKind::Ptr || rhs_ty.kind == TypeKind::Array)
    {
        let base_size = lhs
            .ty
            .as_ref()
            .unwrap()
            .base
            .as_ref()
            .unwrap()
            .borrow()
            .size;
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc, line_no);
        node.ty = Some(Type::new_int());
        let mut result = new_binary(
            NodeKind::Div,
            node,
            new_long(base_size, tok_loc, line_no),
            tok_loc,
            line_no,
        );
        result.ty = Some(Type::new_int());
        return Ok(result);
    }

    Err(error_at(filename, src, tok_loc, "invalid operands"))
}

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    File, Node, NodeKind, Obj, TagScope, Token, TokenKind, Type, TypeKind, VarAttr, VarScope,
    align_to, error_at, error_tok, new_unique_name, new_var_unique_id,
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

fn is_end(files: &[File], tok: &Token) -> bool {
    equal(files, tok, "}")
        || (equal(files, tok, ",") && tok.next.as_ref().is_some_and(|n| equal(files, n, "}")))
}

fn consume_end(files: &[File], tok: &Token) -> (bool, Token) {
    if equal(files, tok, "}") {
        return (true, tok.next.as_ref().unwrap().as_ref().clone());
    }
    if equal(files, tok, ",") && tok.next.as_ref().is_some_and(|n| equal(files, n, "}")) {
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

pub fn new_node(kind: NodeKind, tok_loc: usize, file_no: usize, line_no: usize) -> Node {
    Node {
        kind,
        tok_loc,
        file_no,
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
        func_ty: None,
        args: None,
        pass_by_stack: false,
        var: None,
        val: 0,
        fval: 0.0,
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
        fval: tok.fval,
        loc: tok.loc,
        len: tok.len,
        ty: tok.ty.clone(),
        str: tok.str.clone(),
        file_no: tok.file_no,
        line_no: tok.line_no,
        at_bol: tok.at_bol,
    }
}

pub fn new_binary(
    kind: NodeKind,
    lhs: Node,
    rhs: Node,
    tok_loc: usize,
    file_no: usize,
    line_no: usize,
) -> Node {
    let mut node = new_node(kind, tok_loc, file_no, line_no);
    node.lhs = Some(Box::new(lhs));
    node.rhs = Some(Box::new(rhs));
    node
}

pub fn new_unary(
    kind: NodeKind,
    expr: Node,
    tok_loc: usize,
    file_no: usize,
    line_no: usize,
) -> Node {
    let mut node = new_node(kind, tok_loc, file_no, line_no);
    node.lhs = Some(Box::new(expr));
    node
}

pub fn new_num(val: i64, tok_loc: usize, file_no: usize, line_no: usize) -> Node {
    let mut node = new_node(NodeKind::Num, tok_loc, file_no, line_no);
    node.val = val;
    node
}

pub fn new_long(val: i64, tok_loc: usize, file_no: usize, line_no: usize) -> Node {
    let mut node = new_node(NodeKind::Num, tok_loc, file_no, line_no);
    node.val = val;
    node.ty = Some(Type::new_long());
    node
}

pub fn new_ulong(val: i64, tok_loc: usize, file_no: usize, line_no: usize) -> Node {
    let mut node = new_node(NodeKind::Num, tok_loc, file_no, line_no);
    node.val = val;
    node.ty = Some(Type::new_ulong());
    node
}

pub fn new_var_node(var: Obj, tok_loc: usize, file_no: usize, line_no: usize) -> Node {
    let mut node = new_node(NodeKind::Var, tok_loc, file_no, line_no);
    node.var = Some(Box::new(var.clone()));
    node.ty = Some(var.ty);
    node
}

pub fn new_cast(expr: Node, ty: Type) -> Node {
    let mut expr = expr;
    add_type(&mut expr);
    let mut node = new_node(NodeKind::Cast, expr.tok_loc, expr.file_no, expr.line_no);
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
    files: &[File],
) -> Option<Rc<RefCell<Type>>> {
    if tok.kind != TokenKind::Ident {
        return None;
    }
    let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
    let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();
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
        va_area: None,
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
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    if equal(files, tok, "{") {
        let tok = skip(files, tok, "{")?;
        let tok = skip_excess_element(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        return skip(files, &tok, "}");
    }

    let (_, tok) = assign(files, tok, locals, globals, scope_stack, tag_scope_stack)?;
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
fn count_array_init_elements(
    files: &[File],
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
        let (is_end, _) = consume_end(files, &tok);
        if is_end {
            break;
        }
        if i > 0 {
            tok = skip(files, &tok, ",")?;
        }
        let mut dummy = dummy.clone();
        tok = initializer2(
            files,
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
        init.children[i].expr = Some(new_num(c as i64, tok.loc, tok.file_no, tok.line_no));
    }
    tok.next.as_ref().unwrap().as_ref().clone()
}

#[allow(clippy::too_many_arguments)]
fn array_initializer1(
    files: &[File],
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    let mut tok = skip(files, tok, "{")?;

    if init.is_flexible {
        let len = count_array_init_elements(
            files,
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
        let (is_end, new_tok) = consume_end(files, &tok);
        if is_end {
            return Ok(new_tok);
        }
        if i > 0 {
            tok = skip(files, &tok, ",")?;
        }

        if i < init.ty.array_len as usize {
            tok = initializer2(
                files,
                &tok,
                &mut init.children[i],
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
        } else {
            tok = skip_excess_element(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        }
        i += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn array_initializer2(
    files: &[File],
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    if init.is_flexible {
        let len = count_array_init_elements(
            files,
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
        if is_end(files, &tok) {
            break;
        }
        if i > 0 {
            tok = skip(files, &tok, ",")?;
        }
        tok = initializer2(
            files,
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
    files: &[File],
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    let mut tok = skip(files, tok, "{")?;

    let mut mem = init.ty.members.as_ref();
    let mut first = true;

    loop {
        let (is_end, new_tok) = consume_end(files, &tok);
        if is_end {
            return Ok(new_tok);
        }

        if !first {
            tok = skip(files, &tok, ",")?;
        }
        first = false;

        if let Some(m) = mem {
            tok = initializer2(
                files,
                &tok,
                &mut init.children[m.idx as usize],
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            mem = m.next.as_ref();
        } else {
            tok = skip_excess_element(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn struct_initializer2(
    files: &[File],
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
        if is_end(files, &tok) {
            break;
        }
        if !first {
            tok = skip(files, &tok, ",")?;
        }
        first = false;
        tok = initializer2(
            files,
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
    files: &[File],
    tok: &Token,
    init: &mut Initializer,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    if equal(files, tok, "{") {
        let tok = skip(files, tok, "{")?;
        let tok = initializer2(
            files,
            &tok,
            &mut init.children[0],
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let (_, tok) = consume(files, &tok, ",");
        return skip(files, &tok, "}");
    }
    initializer2(
        files,
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
    files: &[File],
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
        if equal(files, tok, "{") {
            return array_initializer1(
                files,
                tok,
                init,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            );
        }
        return array_initializer2(
            files,
            tok,
            init,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        );
    }

    if init.ty.kind == TypeKind::Struct {
        if equal(files, tok, "{") {
            return struct_initializer1(
                files,
                tok,
                init,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            );
        }

        let (expr_node, new_tok) =
            assign(files, tok, locals, globals, scope_stack, tag_scope_stack)?;
        let mut expr_node = expr_node;
        add_type(&mut expr_node);
        if expr_node.ty.as_ref().unwrap().kind == TypeKind::Struct {
            init.expr = Some(expr_node);
            return Ok(new_tok);
        }

        return struct_initializer2(
            files,
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
            files,
            tok,
            init,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        );
    }

    if equal(files, tok, "{") {
        let tok = initializer2(
            files,
            tok.next.as_ref().unwrap(),
            init,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return skip(files, &tok, "}");
    }

    let (expr_node, tok) = assign(files, tok, locals, globals, scope_stack, tag_scope_stack)?;
    init.expr = Some(expr_node);
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
fn initializer(
    files: &[File],
    tok: &Token,
    ty: &Type,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Initializer, Type, Token), String> {
    let mut init = new_initializer(ty, true);
    let tok = initializer2(
        files,
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
    file_no: usize,
    line_no: usize,
    files: &[File],
) -> Result<Node, String> {
    if let Some(var) = &desg.var {
        return Ok(new_var_node(var.clone(), tok_loc, file_no, line_no));
    }

    if let Some(member) = &desg.member {
        let node = init_desg_expr(
            desg.next.as_ref().unwrap().as_ref(),
            tok_loc,
            file_no,
            line_no,
            files,
        )?;
        let mut node = new_unary(NodeKind::Member, node, tok_loc, file_no, line_no);
        node.member = Some(Box::new(member.clone()));
        return Ok(node);
    }

    let lhs = init_desg_expr(
        desg.next.as_ref().unwrap().as_ref(),
        tok_loc,
        file_no,
        line_no,
        files,
    )?;
    let rhs = new_num(desg.idx, tok_loc, file_no, line_no);
    let add_node = new_add(lhs, rhs, tok_loc, line_no, file_no, files)?;
    Ok(new_unary(
        NodeKind::Deref,
        add_node,
        tok_loc,
        file_no,
        line_no,
    ))
}

fn create_lvar_init(
    init: &Initializer,
    ty: &Type,
    desg: &InitDesg,
    tok_loc: usize,
    file_no: usize,
    line_no: usize,
    files: &[File],
) -> Result<Node, String> {
    if ty.kind == TypeKind::Array {
        let mut node = new_node(NodeKind::NullExpr, tok_loc, file_no, line_no);
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
                file_no,
                line_no,
                files,
            )?;
            node = new_binary(NodeKind::Comma, node, rhs, tok_loc, file_no, line_no);
        }
        return Ok(node);
    }

    if ty.kind == TypeKind::Struct {
        if let Some(rhs) = &init.expr {
            let lhs = init_desg_expr(desg, tok_loc, file_no, line_no, files)?;
            return Ok(new_binary(
                NodeKind::Assign,
                lhs,
                rhs.clone(),
                tok_loc,
                file_no,
                line_no,
            ));
        }

        let mut node = new_node(NodeKind::NullExpr, tok_loc, file_no, line_no);

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
                file_no,
                line_no,
                files,
            )?;
            node = new_binary(NodeKind::Comma, node, rhs, tok_loc, file_no, line_no);
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
            file_no,
            line_no,
            files,
        );
    }

    let lhs = init_desg_expr(desg, tok_loc, file_no, line_no, files)?;
    if init.expr.is_none() {
        return Ok(new_node(NodeKind::NullExpr, tok_loc, file_no, line_no));
    }
    let rhs = init.expr.as_ref().unwrap().clone();
    Ok(new_binary(
        NodeKind::Assign,
        lhs,
        rhs,
        tok_loc,
        file_no,
        line_no,
    ))
}

#[allow(clippy::too_many_arguments)]
fn lvar_initializer(
    files: &[File],
    tok: &Token,
    var_name: &str,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let tok_loc = tok.loc;
    let file_no = tok.file_no;
    let line_no = tok.line_no;

    let var_idx = locals
        .iter()
        .position(|v| v.name == var_name)
        .ok_or_else(|| format!("variable not found: {}", var_name))?;
    let old_ty = locals[var_idx].ty.clone();
    let (init, new_ty, tok) = initializer(
        files,
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

    let mut lhs = new_node(NodeKind::Memzero, tok_loc, file_no, line_no);
    lhs.var = Some(Box::new(var.clone()));

    let desg = InitDesg {
        next: None,
        idx: 0,
        member: None,
        var: Some(var.clone()),
    };
    let rhs = create_lvar_init(&init, &new_ty, &desg, tok_loc, file_no, line_no, files)?;
    Ok((
        new_binary(NodeKind::Comma, lhs, rhs, tok_loc, file_no, line_no),
        tok,
    ))
}

pub fn get_ident(files: &[File], tok: &Token) -> Result<String, String> {
    if tok.kind != TokenKind::Ident {
        return Err(error_tok(files, tok, "expected an identifier"));
    }
    let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
    let name: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();
    Ok(name)
}

pub fn struct_members(
    files: &[File],
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Option<Box<crate::Member>>, bool, Token), String> {
    let mut tok = tok.clone();
    let mut members: Vec<crate::Member> = Vec::new();
    let mut idx: i64 = 0;

    while !equal(files, &tok, "}") {
        let mut attr = VarAttr::default();
        let (basety, new_tok) =
            declspec(files, &tok, tag_scope_stack, scope_stack, Some(&mut attr))?;
        tok = new_tok;
        let mut first = true;

        while !equal(files, &tok, ";") {
            if !first {
                tok = skip(files, &tok, ",")?;
            }
            first = false;

            let (mem_ty, new_tok) =
                declarator(files, &tok, basety.clone(), tag_scope_stack, scope_stack)?;
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
        tok = skip(files, &tok, ";")?;
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
    files: &[File],
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
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
        && !equal(files, &tok, "{")
    {
        let file = files.iter().find(|f| f.file_no == tag_tok.file_no).unwrap();
        let tag_name: String = file
            .contents
            .chars()
            .skip(tag_tok.loc)
            .take(tag_tok.len)
            .collect();
        if let Some(ty) = find_tag(tag_scope_stack, &tag_name) {
            return Ok((ty, tok));
        }

        let ty = Rc::new(RefCell::new(Type::new_struct()));
        ty.borrow_mut().size = -1;
        push_tag_scope(tag_scope_stack, tag_name, ty.clone());
        return Ok((ty, tok));
    }

    tok = skip(files, &tok, "{")?;

    let ty_rc = if let Some(tag_tok) = &tag {
        let file = files.iter().find(|f| f.file_no == tag_tok.file_no).unwrap();
        let tag_name: String = file
            .contents
            .chars()
            .skip(tag_tok.loc)
            .take(tag_tok.len)
            .collect();
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

    let (members, is_flexible, rest) = struct_members(files, &tok, tag_scope_stack, scope_stack)?;

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
    files: &[File],
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let (ty_rc, rest) = struct_union_decl(files, tok, tag_scope_stack, scope_stack)?;
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
    files: &[File],
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let (ty_rc, rest) = struct_union_decl(files, tok, tag_scope_stack, scope_stack)?;
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
    files: &[File],
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
        && !equal(files, &tok, "{")
    {
        let file = files.iter().find(|f| f.file_no == tag_tok.file_no).unwrap();
        let tag_name: String = file
            .contents
            .chars()
            .skip(tag_tok.loc)
            .take(tag_tok.len)
            .collect();
        if let Some(ty) = find_tag(tag_scope_stack, &tag_name) {
            if ty.borrow().kind != TypeKind::Enum {
                return Err(error_tok(files, tag_tok, "not an enum tag"));
            }
            return Ok((ty.borrow().clone(), tok));
        }
        return Err(error_tok(files, tag_tok, "unknown enum type"));
    }

    tok = skip(files, &tok, "{")?;

    let mut val: i64 = 0;
    let mut i = 0;

    loop {
        let (is_end, new_tok) = consume_end(files, &tok);
        if is_end {
            tok = new_tok;
            break;
        }
        if i > 0 {
            tok = skip(files, &tok, ",")?;
        }
        i += 1;

        let name = get_ident(files, &tok)?;
        tok = tok.next.as_ref().unwrap().as_ref().clone();

        if equal(files, &tok, "=") {
            let (v, new_tok) = const_expr(
                files,
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
        let file = files.iter().find(|f| f.file_no == tag_tok.file_no).unwrap();
        let tag_name: String = file
            .contents
            .chars()
            .skip(tag_tok.loc)
            .take(tag_tok.len)
            .collect();
        push_tag_scope(tag_scope_stack, tag_name, Rc::new(RefCell::new(ty.clone())));
    }

    Ok((ty, tok))
}

pub fn get_struct_member(files: &[File], ty: &Type, tok: &Token) -> Result<crate::Member, String> {
    let mut current = ty.members.as_ref();
    while let Some(mem) = current {
        if let Some(name) = &mem.name
            && name.len == tok.len
        {
            let file = files.iter().find(|f| f.file_no == name.file_no).unwrap();
            let mem_name: String = file
                .contents
                .chars()
                .skip(name.loc)
                .take(name.len)
                .collect();
            let tok_file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
            let tok_name: String = tok_file
                .contents
                .chars()
                .skip(tok.loc)
                .take(tok.len)
                .collect();
            if mem_name == tok_name {
                return Ok(mem.as_ref().clone());
            }
        }
        current = mem.next.as_ref();
    }
    Err(error_tok(files, tok, "no such member"))
}

pub fn struct_ref(files: &[File], lhs: Node, tok: &Token) -> Result<Node, String> {
    let mut lhs = lhs;
    add_type(&mut lhs);

    if lhs.ty.as_ref().unwrap().kind != TypeKind::Struct
        && lhs.ty.as_ref().unwrap().kind != TypeKind::Union
    {
        return Err(error_tok(files, tok, "not a struct nor a union"));
    }

    let member = get_struct_member(files, lhs.ty.as_ref().unwrap(), tok)?;
    let mut node = new_unary(NodeKind::Member, lhs, tok.loc, tok.file_no, tok.line_no);
    node.member = Some(Box::new(member));
    Ok(node)
}

pub fn declspec(
    files: &[File],
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
    const FLOAT: i32 = 1 << 12;
    const DOUBLE: i32 = 1 << 14;
    const LONG_DOUBLE: i32 = LONG + DOUBLE;
    const OTHER: i32 = 1 << 16;
    const SIGNED: i32 = 1 << 17;
    const UNSIGNED: i32 = 1 << 18;
    const SHORT_INT: i32 = SHORT + INT;
    const LONG_INT: i32 = LONG + INT;
    const LONG_LONG: i32 = LONG + LONG;
    const LONG_LONG_INT: i32 = LONG_LONG + INT;
    const SIGNED_CHAR: i32 = SIGNED + CHAR;
    const SIGNED_SHORT: i32 = SIGNED + SHORT;
    const SIGNED_SHORT_INT: i32 = SIGNED + SHORT_INT;
    const SIGNED_INT: i32 = SIGNED + INT;
    const SIGNED_LONG: i32 = SIGNED + LONG;
    const SIGNED_LONG_INT: i32 = SIGNED + LONG_INT;
    const SIGNED_LONG_LONG: i32 = SIGNED + LONG_LONG;
    const SIGNED_LONG_LONG_INT: i32 = SIGNED + LONG_LONG_INT;
    const UNSIGNED_CHAR: i32 = UNSIGNED + CHAR;
    const UNSIGNED_SHORT: i32 = UNSIGNED + SHORT;
    const UNSIGNED_SHORT_INT: i32 = UNSIGNED + SHORT_INT;
    const UNSIGNED_INT: i32 = UNSIGNED + INT;
    const UNSIGNED_LONG: i32 = UNSIGNED + LONG;
    const UNSIGNED_LONG_INT: i32 = UNSIGNED + LONG_INT;
    const UNSIGNED_LONG_LONG: i32 = UNSIGNED + LONG_LONG;
    const UNSIGNED_LONG_LONG_INT: i32 = UNSIGNED + LONG_LONG_INT;

    let mut ty = Type::new_int();
    let mut counter = 0;
    let mut tok = tok.clone();

    while is_typename(files, &tok, scope_stack) {
        if equal(files, &tok, "typedef")
            || equal(files, &tok, "static")
            || equal(files, &tok, "extern")
        {
            if let Some(a) = attr.as_mut() {
                if equal(files, &tok, "typedef") {
                    a.is_typedef = true;
                } else if equal(files, &tok, "static") {
                    a.is_static = true;
                } else {
                    a.is_extern = true;
                }
                if a.is_typedef && a.is_static as i32 + a.is_extern as i32 > 1 {
                    return Err(error_tok(
                        files,
                        &tok,
                        "typedef may not be used together with static or extern",
                    ));
                }
            } else {
                return Err(error_tok(
                    files,
                    &tok,
                    "storage class specifier is not allowed in this context",
                ));
            }
            tok = *tok.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(files, &tok, "const")
            || equal(files, &tok, "volatile")
            || equal(files, &tok, "auto")
            || equal(files, &tok, "register")
            || equal(files, &tok, "restrict")
            || equal(files, &tok, "__restrict")
            || equal(files, &tok, "__restrict__")
            || equal(files, &tok, "_Noreturn")
        {
            tok = *tok.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(files, &tok, "_Alignas") {
            if attr.is_none() {
                return Err(error_tok(
                    files,
                    &tok,
                    "_Alignas is not allowed in this context",
                ));
            }
            tok = skip(files, tok.next.as_ref().unwrap(), "(")?;

            if is_typename(files, &tok, scope_stack) {
                let (align_ty, new_tok) = typename(files, &tok, tag_scope_stack, scope_stack)?;
                tok = new_tok;
                if let Some(a) = attr.as_mut() {
                    a.align = align_ty.align;
                }
            } else {
                let (val, new_tok) = const_expr(files, &tok, tag_scope_stack, scope_stack)?;
                tok = new_tok;
                if let Some(a) = attr.as_mut() {
                    a.align = val;
                }
            }
            tok = skip(files, &tok, ")")?;
            continue;
        }

        let ty2 = find_typedef(scope_stack, &tok, files);
        if equal(files, &tok, "struct")
            || equal(files, &tok, "union")
            || equal(files, &tok, "enum")
            || ty2.is_some()
        {
            if counter > 0 {
                break;
            }

            if equal(files, &tok, "struct") {
                let (new_ty, new_tok) = struct_decl(
                    files,
                    tok.next.as_ref().unwrap(),
                    tag_scope_stack,
                    scope_stack,
                )?;
                ty = new_ty;
                tok = new_tok;
            } else if equal(files, &tok, "union") {
                let (new_ty, new_tok) = union_decl(
                    files,
                    tok.next.as_ref().unwrap(),
                    tag_scope_stack,
                    scope_stack,
                )?;
                ty = new_ty;
                tok = new_tok;
            } else if equal(files, &tok, "enum") {
                let (new_ty, new_tok) = enum_specifier(
                    files,
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

        if equal(files, &tok, "void") {
            counter += VOID;
        } else if equal(files, &tok, "_Bool") {
            counter += BOOL;
        } else if equal(files, &tok, "char") {
            counter += CHAR;
        } else if equal(files, &tok, "short") {
            counter += SHORT;
        } else if equal(files, &tok, "int") {
            counter += INT;
        } else if equal(files, &tok, "long") {
            counter += LONG;
        } else if equal(files, &tok, "float") {
            counter += FLOAT;
        } else if equal(files, &tok, "double") {
            counter += DOUBLE;
        } else if equal(files, &tok, "signed") {
            counter |= SIGNED;
        } else if equal(files, &tok, "unsigned") {
            counter |= UNSIGNED;
        } else {
            unreachable!();
        }

        match counter {
            VOID => ty = Type::new_void(),
            BOOL => ty = Type::new_bool(),
            CHAR | SIGNED_CHAR => ty = Type::new_char(),
            UNSIGNED_CHAR => ty = Type::new_uchar(),
            SHORT | SHORT_INT | SIGNED_SHORT | SIGNED_SHORT_INT => ty = Type::new_short(),
            UNSIGNED_SHORT | UNSIGNED_SHORT_INT => ty = Type::new_ushort(),
            INT | SIGNED | SIGNED_INT => ty = Type::new_int(),
            UNSIGNED | UNSIGNED_INT => ty = Type::new_uint(),
            LONG | LONG_INT | LONG_LONG | LONG_LONG_INT | SIGNED_LONG | SIGNED_LONG_INT
            | SIGNED_LONG_LONG | SIGNED_LONG_LONG_INT => ty = Type::new_long(),
            UNSIGNED_LONG | UNSIGNED_LONG_INT | UNSIGNED_LONG_LONG | UNSIGNED_LONG_LONG_INT => {
                ty = Type::new_ulong()
            }
            FLOAT => ty = Type::new_float(),
            DOUBLE | LONG_DOUBLE => ty = Type::new_double(),
            _ => return Err(error_tok(files, &tok, "invalid type")),
        }

        tok = *tok.next.as_ref().unwrap().clone();
    }

    Ok((ty, tok))
}

pub fn is_typename(files: &[File], tok: &Token, scope_stack: &[Vec<VarScope>]) -> bool {
    equal(files, tok, "void")
        || equal(files, tok, "_Bool")
        || equal(files, tok, "char")
        || equal(files, tok, "short")
        || equal(files, tok, "int")
        || equal(files, tok, "long")
        || equal(files, tok, "struct")
        || equal(files, tok, "union")
        || equal(files, tok, "typedef")
        || equal(files, tok, "enum")
        || equal(files, tok, "static")
        || equal(files, tok, "extern")
        || equal(files, tok, "_Alignas")
        || equal(files, tok, "signed")
        || equal(files, tok, "unsigned")
        || equal(files, tok, "const")
        || equal(files, tok, "volatile")
        || equal(files, tok, "auto")
        || equal(files, tok, "register")
        || equal(files, tok, "restrict")
        || equal(files, tok, "__restrict")
        || equal(files, tok, "__restrict__")
        || equal(files, tok, "_Noreturn")
        || equal(files, tok, "float")
        || equal(files, tok, "double")
        || find_typedef(scope_stack, tok, files).is_some()
}

pub fn get_number(tok: &Token) -> Result<i64, String> {
    if tok.kind != TokenKind::Num {
        return Err("expected a number".to_string());
    }
    Ok(tok.val)
}

pub fn is_function(
    files: &[File],
    tok: &Token,
    scope_stack: &[Vec<VarScope>],
) -> Result<bool, String> {
    if equal(files, tok, ";") {
        return Ok(false);
    }

    let dummy = Type::new_int();
    let mut tag_scope_stack: Vec<Vec<TagScope>> = vec![Vec::new()];
    let (ty, _) = declarator(
        files,
        tok,
        dummy,
        &mut tag_scope_stack,
        &mut scope_stack.to_vec(),
    )?;
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
    files: &[File],
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
                files,
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
                files,
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
            files,
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

    if ty.kind == TypeKind::Float {
        let fval = eval_double(files, &mut init.expr.as_ref().unwrap().clone())?;
        let bytes = (fval as f32).to_le_bytes();
        buf[offset..offset + 4].copy_from_slice(&bytes);
        return Ok(());
    }

    if ty.kind == TypeKind::Double {
        let fval = eval_double(files, &mut init.expr.as_ref().unwrap().clone())?;
        let bytes = fval.to_le_bytes();
        buf[offset..offset + 8].copy_from_slice(&bytes);
        return Ok(());
    }

    let mut expr = init.expr.as_ref().unwrap().clone();
    let mut label: Option<String> = None;
    let val = eval2(files, &mut expr, Some(&mut label))?;

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
    files: &[File],
    tok: &Token,
    var: &mut Obj,
    globals: &mut Vec<Obj>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<Token, String> {
    let mut empty_locals: Vec<Obj> = Vec::new();
    let mut scope_stack_vec: Vec<Vec<VarScope>> = scope_stack.to_vec();
    let (init, new_ty, tok) = initializer(
        files,
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
    write_gvar_data(files, &init, &var.ty, &mut buf, 0, &mut rel_head)?;
    var.init_data = Some(buf);
    var.rel = rel_head;
    Ok(tok)
}

#[allow(clippy::too_many_arguments)]
pub fn global_variable(
    files: &[File],
    tok: &Token,
    basety: Type,
    globals: &mut Vec<Obj>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
    attr: &VarAttr,
) -> Result<Token, String> {
    let mut tok = tok.clone();
    let mut first = true;

    loop {
        if !first {
            tok = skip(files, &tok, ",")?;
        }
        first = false;

        let (ty, new_tok) = declarator(files, &tok, basety.clone(), tag_scope_stack, scope_stack)?;
        tok = new_tok;
        if ty.kind == TypeKind::Array && ty.array_len < 0 && !equal(files, &tok, "=") {
            return Err(error_tok(files, &tok, "variable has incomplete type"));
        }
        if ty.name.is_none() {
            return Err(error_tok(
                files,
                ty.name_pos.as_ref().unwrap(),
                "variable name omitted",
            ));
        }
        let name = get_ident(files, ty.name.as_ref().unwrap())?;
        let mut var = new_gvar(name, ty);
        var.is_definition = !attr.is_extern;
        var.is_static = attr.is_static;
        if attr.align > 0 {
            var.align = attr.align;
        }
        if equal(files, &tok, "=") {
            tok = gvar_initializer(
                files,
                tok.next.as_ref().unwrap(),
                &mut var,
                globals,
                tag_scope_stack,
                scope_stack,
            )?;
        }
        globals.push(var);

        if equal(files, &tok, ";") {
            return Ok(*tok.next.as_ref().unwrap().clone());
        }
    }
}

pub fn func_params(
    files: &[File],
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let mut tok = tok.clone();

    if equal(files, &tok, "void")
        && tok
            .next
            .as_ref()
            .is_some_and(|next| equal(files, next, ")"))
    {
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
    };
    let mut cur = &mut head;
    let mut first = true;
    let mut is_variadic = false;

    while !equal(files, &tok, ")") {
        if !first {
            tok = skip(files, &tok, ",")?;
        }
        first = false;

        if equal(files, &tok, "...") {
            is_variadic = true;
            tok = tok.next.as_ref().unwrap().as_ref().clone();
            tok = skip(files, &tok, ")")?;
            let mut func_ty = func_type(ty);
            func_ty.params = head.next;
            func_ty.is_variadic = is_variadic;
            return Ok((func_ty, tok));
        }

        let (basety, new_tok) = declspec(files, &tok, tag_scope_stack, scope_stack, None)?;
        tok = new_tok;
        let (param_ty, new_tok) = declarator(files, &tok, basety, tag_scope_stack, scope_stack)?;
        tok = new_tok;

        let param_ty = if param_ty.kind == TypeKind::Array {
            let name = param_ty.name.clone();
            let mut ptr_ty = Type::new_ptr(param_ty.base.unwrap().borrow().clone());
            ptr_ty.name = name;
            ptr_ty
        } else if param_ty.kind == TypeKind::Func {
            let name = param_ty.name.clone();
            let mut ptr_ty = Type::new_ptr(param_ty);
            ptr_ty.name = name;
            ptr_ty
        } else {
            param_ty
        };

        let param_copy = copy_type(&param_ty);
        cur.next = Some(Box::new(param_copy));
        cur = cur.next.as_mut().unwrap();
    }

    if head.next.is_none() {
        is_variadic = true;
    }

    let mut func_ty = func_type(ty);
    func_ty.params = head.next;
    func_ty.is_variadic = is_variadic;
    let rest = tok.next.as_ref().unwrap().as_ref().clone();
    Ok((func_ty, rest))
}

pub fn array_dimensions(
    files: &[File],
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let mut tok = tok.clone();
    while equal(files, &tok, "static")
        || equal(files, &tok, "restrict")
        || equal(files, &tok, "__restrict")
        || equal(files, &tok, "__restrict__")
    {
        tok = *tok.next.as_ref().unwrap().clone();
    }

    if equal(files, &tok, "]") {
        let (ty, rest) = type_suffix(
            files,
            tok.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        )?;
        return Ok((Type::new_array(ty, -1), rest));
    }

    let (sz, tok) = const_expr(files, &tok, tag_scope_stack, scope_stack)?;
    let tok = skip(files, &tok, "]")?;
    let (ty, rest) = type_suffix(files, &tok, ty, tag_scope_stack, scope_stack)?;
    Ok((Type::new_array(ty, sz), rest))
}

pub fn type_suffix(
    files: &[File],
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    if equal(files, tok, "(") {
        return func_params(
            files,
            tok.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        );
    }

    if equal(files, tok, "[") {
        return array_dimensions(
            files,
            tok.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        );
    }

    Ok((ty, tok.clone()))
}

fn pointers(files: &[File], mut tok: Token, mut ty: Type) -> (Token, Type) {
    loop {
        let (consumed, new_tok) = consume(files, &tok, "*");
        if !consumed {
            break;
        }
        tok = new_tok;
        ty = pointer_to(ty);
        while equal(files, &tok, "const")
            || equal(files, &tok, "volatile")
            || equal(files, &tok, "restrict")
            || equal(files, &tok, "__restrict")
            || equal(files, &tok, "__restrict__")
        {
            tok = *tok.next.as_ref().unwrap().clone();
        }
    }
    (tok, ty)
}

pub fn declarator(
    files: &[File],
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let (tok, ty) = pointers(files, tok.clone(), ty);

    if equal(files, &tok, "(") {
        let start = tok.clone();
        let dummy = Type::new_int();
        let (_, tok) = declarator(
            files,
            start.next.as_ref().unwrap(),
            dummy,
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(files, &tok, ")")?;
        let (ty, rest) = type_suffix(files, &tok, ty, tag_scope_stack, scope_stack)?;
        let (ty, _) = declarator(
            files,
            start.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        )?;
        return Ok((ty, rest));
    }

    let name_pos = token_snapshot(&tok);
    let (name, tok) = if tok.kind == TokenKind::Ident {
        let name_tok = token_snapshot(&tok);
        let tok = tok.next.as_ref().unwrap().as_ref().clone();
        (Some(name_tok), tok)
    } else {
        (None, tok)
    };

    let (ty, tok) = type_suffix(files, &tok, ty, tag_scope_stack, scope_stack)?;

    let mut ty = ty;
    ty.name = name.map(Box::new);
    ty.name_pos = Some(Box::new(name_pos));
    Ok((ty, tok))
}

pub fn abstract_declarator(
    files: &[File],
    tok: &Token,
    ty: Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let (tok, ty) = pointers(files, tok.clone(), ty);

    if equal(files, &tok, "(") {
        let start = tok.clone();
        let dummy = Type::new_int();
        let (_, tok) = abstract_declarator(
            files,
            start.next.as_ref().unwrap(),
            dummy,
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(files, &tok, ")")?;
        let (ty, rest) = type_suffix(files, &tok, ty, tag_scope_stack, scope_stack)?;
        let (ty, _) = abstract_declarator(
            files,
            start.next.as_ref().unwrap(),
            ty,
            tag_scope_stack,
            scope_stack,
        )?;
        return Ok((ty, rest));
    }

    type_suffix(files, &tok, ty, tag_scope_stack, scope_stack)
}

pub fn typename(
    files: &[File],
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(Type, Token), String> {
    let (ty, tok) = declspec(files, tok, tag_scope_stack, scope_stack, None)?;
    abstract_declarator(files, &tok, ty, tag_scope_stack, scope_stack)
}

#[allow(clippy::too_many_arguments)]
pub fn declaration(
    files: &[File],
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
        file_no: tok.file_no,
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
        func_ty: None,
        args: None,
        pass_by_stack: false,
        var: None,
        val: 0,
        fval: 0.0,
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
    let mut first = true;

    loop {
        if !first {
            tok = skip(files, &tok, ",")?;
        }
        first = false;

        let (ty, new_tok) = declarator(files, &tok, basety.clone(), tag_scope_stack, scope_stack)?;
        tok = new_tok;
        if ty.kind == TypeKind::Void {
            return Err(error_tok(
                files,
                ty.name_pos.as_ref().unwrap(),
                "variable declared void",
            ));
        }
        if ty.name.is_none() {
            return Err(error_tok(
                files,
                ty.name_pos.as_ref().unwrap(),
                "variable name omitted",
            ));
        }
        let name = get_ident(files, ty.name.as_ref().unwrap())?;

        if let Some(a) = attr
            && a.is_static
        {
            let mut var = new_anon_gvar(ty.clone());
            var.is_static = true;
            var.is_definition = true;
            if equal(files, &tok, "=") {
                tok = gvar_initializer(
                    files,
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
            if equal(files, &tok, ";") {
                let tok_loc = tok.loc;
                let file_no = tok.file_no;
                let line_no = tok.line_no;
                let mut node = new_node(NodeKind::Block, tok_loc, file_no, line_no);
                node.body = head.next;
                return Ok((node, *tok.next.as_ref().unwrap().clone()));
            }
            continue;
        }

        new_lvar(name.clone(), ty, locals, scope_stack);

        if let Some(a) = attr
            && a.align > 0
        {
            let var_idx = locals.iter().position(|v| v.name == name).unwrap();
            locals[var_idx].align = a.align;
        }

        if equal(files, &tok, "=") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let tok_next = tok.next.as_ref().unwrap().clone();
            let (expr_node, new_tok) = lvar_initializer(
                files,
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
                file_no,
                line_no,
            )));
            cur = cur.next.as_mut().unwrap();
        }

        let var_idx = locals.iter().position(|v| v.name == name).unwrap();
        if locals[var_idx].ty.size < 0 {
            return Err(error_tok(
                files,
                locals[var_idx].ty.name.as_ref().unwrap(),
                "variable has incomplete type",
            ));
        }
        if locals[var_idx].ty.kind == TypeKind::Void {
            return Err(error_tok(
                files,
                locals[var_idx].ty.name.as_ref().unwrap(),
                "variable declared void",
            ));
        }

        if equal(files, &tok, ";") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let mut node = new_node(NodeKind::Block, tok_loc, file_no, line_no);
            node.body = head.next;
            return Ok((node, *tok.next.as_ref().unwrap().clone()));
        }
    }
}

pub fn parse_typedef(
    files: &[File],
    tok: &Token,
    basety: Type,
    scope_stack: &mut [Vec<VarScope>],
) -> Result<Token, String> {
    let mut tok = tok.clone();
    let mut first = true;

    loop {
        if !first {
            tok = skip(files, &tok, ",")?;
        }
        first = false;

        let (ty, new_tok) = declarator(files, &tok, basety.clone(), &mut Vec::new(), scope_stack)?;
        tok = new_tok;
        if ty.name.is_none() {
            if equal(files, &tok, ";") {
                return Ok(*tok.next.as_ref().unwrap().clone());
            }
            return Err(error_tok(
                files,
                ty.name_pos.as_ref().unwrap(),
                "variable name omitted",
            ));
        }
        let name = get_ident(files, ty.name.as_ref().unwrap())?;
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

        if equal(files, &tok, ";") {
            return Ok(*tok.next.as_ref().unwrap().clone());
        }
    }
}

pub fn create_param_lvars(
    files: &[File],
    param: &Type,
    locals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
) -> Result<(), String> {
    let mut current = Some(param);

    while let Some(p) = current {
        if p.name.is_none() {
            return Err(error_tok(
                files,
                p.name_pos.as_ref().unwrap(),
                "parameter name omitted",
            ));
        }
        let name = get_ident(files, p.name.as_ref().unwrap())?;
        new_lvar(name, p.clone(), locals, scope_stack);
        current = p.next.as_ref().map(|b| b.as_ref());
    }
    Ok(())
}

fn resolve_goto_labels(files: &[File], body: &mut Node) -> Result<(), String> {
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
                fval: 0.0,
                loc: goto.tok_loc,
                file_no: goto.file_no,
                len: label_name.len(),
                ty: None,
                str: None,
                line_no: goto.line_no,
                at_bol: false,
                has_space: false,
                hideset: std::collections::HashSet::new(),
                origin: None,
            };
            return Err(error_tok(files, &tok, "use of undeclared label"));
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
    files: &[File],
    tok: &Token,
    basety: Type,
    globals: &mut Vec<Obj>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    scope_stack: &mut [Vec<VarScope>],
    attr: &VarAttr,
) -> Result<(Obj, Token), String> {
    let (ty, tok) = declarator(files, tok, basety, tag_scope_stack, scope_stack)?;
    if ty.name.is_none() {
        return Err(error_tok(
            files,
            ty.name_pos.as_ref().unwrap(),
            "function name omitted",
        ));
    }
    let name = get_ident(files, ty.name.as_ref().unwrap())?;

    let mut fn_obj = new_gvar(name, ty.clone());
    fn_obj.is_function = true;

    let (is_definition, tok) = consume(files, &tok, ";");
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
        create_param_lvars(files, params, &mut locals, &mut local_scope_stack)?;
    }

    fn_obj.params = locals.clone();

    if ty.is_variadic {
        let va_area = new_lvar(
            "__va_area__".to_string(),
            Type::new_array(Type::new_char(), 136),
            &mut locals,
            &mut local_scope_stack,
        );
        fn_obj.va_area = Some(Box::new(va_area));
    }

    let tok = skip(files, &tok, "{")?;

    let func_name_bytes = fn_obj.name.as_bytes();
    let func_name_ty = Type::new_array(Type::new_char(), func_name_bytes.len() as i64 + 1);
    let func_name_var = new_string_literal(func_name_bytes, func_name_ty);
    globals.push(func_name_var.clone());
    local_scope_stack.last_mut().unwrap().push(VarScope {
        name: "__func__".to_string(),
        var: Some(func_name_var.clone()),
        type_def: None,
        enum_ty: None,
        enum_val: 0,
    });
    local_scope_stack.last_mut().unwrap().push(VarScope {
        name: "__FUNCTION__".to_string(),
        var: Some(func_name_var),
        type_def: None,
        enum_ty: None,
        enum_val: 0,
    });

    let return_ty = ty.return_ty.as_ref().map(|b| b.as_ref());
    let (mut body, tok) = compound_stmt(
        files,
        &tok,
        &mut locals,
        globals,
        &mut local_scope_stack,
        tag_scope_stack,
        return_ty,
    )?;

    add_type(&mut body);
    resolve_goto_labels(files, &mut body)?;

    fn_obj.body = Some(Box::new(body));
    fn_obj.locals = locals;

    tag_scope_stack.pop();

    Ok((fn_obj, tok))
}

#[allow(clippy::too_many_arguments)]
pub fn compound_stmt(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    return_ty: Option<&Type>,
) -> Result<(Node, Token), String> {
    let tok_loc = tok.loc;
    let file_no = tok.file_no;
    let line_no = tok.line_no;

    scope_stack.push(Vec::new());
    tag_scope_stack.push(Vec::new());

    let mut head = Node {
        kind: NodeKind::Num,
        tok_loc,
        file_no,
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
        func_ty: None,
        args: None,
        pass_by_stack: false,
        var: None,
        val: 0,
        fval: 0.0,
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
    while !equal(files, &tok, "}") {
        if is_typename(files, &tok, scope_stack) && !equal(files, tok.next.as_ref().unwrap(), ":") {
            let mut attr = VarAttr::default();
            let (basety, new_tok) =
                declspec(files, &tok, tag_scope_stack, scope_stack, Some(&mut attr))?;
            tok = new_tok;

            if attr.is_typedef {
                tok = parse_typedef(files, &tok, basety, scope_stack)?;
                continue;
            }

            if is_function(files, &tok, scope_stack)? {
                let (_, new_tok) = function(
                    files,
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
                    files,
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
                files,
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
                files,
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

    let mut node = new_node(NodeKind::Block, tok_loc, file_no, line_no);
    node.body = head.next;
    Ok((node, *tok.next.as_ref().unwrap().clone()))
}

#[allow(clippy::too_many_arguments)]
pub fn stmt(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
    return_ty: Option<&Type>,
) -> Result<(Node, Token), String> {
    if equal(files, tok, "return") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let tok = tok.next.as_ref().unwrap();
        let (consumed, tok) = consume(files, tok, ";");
        if consumed {
            let node = new_node(NodeKind::Return, tok_loc, file_no, line_no);
            return Ok((node, tok));
        }
        let (mut expr_node, tok) =
            expr(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        let tok = skip(files, &tok, ";")?;
        if let Some(ret_ty) = return_ty {
            add_type(&mut expr_node);
            expr_node = new_cast(expr_node, ret_ty.clone());
        }
        let node = new_unary(NodeKind::Return, expr_node, tok_loc, file_no, line_no);
        return Ok((node, tok));
    }
    if equal(files, tok, "if") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::If, tok_loc, file_no, line_no);
        let tok = skip(files, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(files, &tok, ")")?;
        let (then, tok) = stmt(
            files,
            &tok,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        )?;
        node.then = Some(Box::new(then));
        let mut tok = tok;
        if equal(files, &tok, "else") {
            let (els, new_tok) = stmt(
                files,
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
    if equal(files, tok, "for") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::For, tok_loc, file_no, line_no);
        let mut tok = skip(files, tok.next.as_ref().unwrap(), "(")?;

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

        if is_typename(files, &tok, scope_stack) {
            let (basety, new_tok) = declspec(files, &tok, tag_scope_stack, scope_stack, None)?;
            tok = new_tok;
            let (init, new_tok) = declaration(
                files,
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
            let (init, new_tok) =
                expr_stmt(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
            node.init = Some(Box::new(init));
            tok = new_tok;
        }

        if !equal(files, &tok, ";") {
            let (cond, new_tok) = expr(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
            node.cond = Some(Box::new(cond));
            tok = new_tok;
        }
        tok = skip(files, &tok, ";")?;

        if !equal(files, &tok, ")") {
            let (inc, new_tok) = expr(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
            node.inc = Some(Box::new(inc));
            tok = new_tok;
        }
        tok = skip(files, &tok, ")")?;

        let (then, tok) = stmt(
            files,
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
    if equal(files, tok, "while") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::While, tok_loc, file_no, line_no);
        let tok = skip(files, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(files, &tok, ")")?;

        let brk = brk_label_get();
        let cont = cont_label_get();
        let brk_name = new_unique_name();
        let cont_name = new_unique_name();
        brk_label_set(Some(brk_name.clone()));
        cont_label_set(Some(cont_name.clone()));
        node.brk_label = Some(brk_name);
        node.cont_label = Some(cont_name);

        let (then, tok) = stmt(
            files,
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
    if equal(files, tok, "do") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Do, tok_loc, file_no, line_no);

        let brk = brk_label_get();
        let cont = cont_label_get();
        let brk_name = new_unique_name();
        let cont_name = new_unique_name();
        brk_label_set(Some(brk_name.clone()));
        cont_label_set(Some(cont_name.clone()));
        node.brk_label = Some(brk_name);
        node.cont_label = Some(cont_name);

        let (then, tok) = stmt(
            files,
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

        let tok = skip(files, &tok, "while")?;
        let tok = skip(files, &tok, "(")?;
        let (cond, tok) = expr(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(files, &tok, ")")?;
        let tok = skip(files, &tok, ";")?;
        return Ok((node, tok));
    }
    if equal(files, tok, "goto") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Goto, tok_loc, file_no, line_no);
        let label_tok = tok.next.as_ref().unwrap();
        node.label = Some(get_ident(files, label_tok)?);
        node.goto_next = gotos_get();
        gotos_set(Some(Box::new(node.clone())));
        let tok = skip(files, label_tok.next.as_ref().unwrap(), ";")?;
        return Ok((node, tok));
    }
    if equal(files, tok, "break") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let brk = brk_label_get();
        brk_label_set(brk.clone());
        if brk.is_none() {
            return Err(error_tok(files, tok, "stray break"));
        }
        let mut node = new_node(NodeKind::Goto, tok_loc, file_no, line_no);
        node.unique_label = brk;
        let tok = skip(files, tok.next.as_ref().unwrap(), ";")?;
        return Ok((node, tok));
    }
    if equal(files, tok, "continue") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let cont = cont_label_get();
        cont_label_set(cont.clone());
        if cont.is_none() {
            return Err(error_tok(files, tok, "stray continue"));
        }
        let mut node = new_node(NodeKind::Goto, tok_loc, file_no, line_no);
        node.unique_label = cont;
        let tok = skip(files, tok.next.as_ref().unwrap(), ";")?;
        return Ok((node, tok));
    }
    if equal(files, tok, "switch") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Switch, tok_loc, file_no, line_no);
        let tok = skip(files, tok.next.as_ref().unwrap(), "(")?;
        let (cond, tok) = expr(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        node.cond = Some(Box::new(cond));
        let tok = skip(files, &tok, ")")?;

        let sw = current_switch_get();
        let brk = brk_label_get();
        let brk_name = new_unique_name();
        node.brk_label = Some(brk_name.clone());
        brk_label_set(Some(brk_name));
        current_switch_set(Some(Box::new(node.clone())));

        let (then, tok) = stmt(
            files,
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
    if equal(files, tok, "case") {
        let sw = current_switch_get();
        current_switch_set(sw.clone());
        if sw.is_none() {
            return Err(error_tok(files, tok, "stray case"));
        }
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (val, new_tok) = const_expr(
            files,
            tok.next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(files, &new_tok, ":")?;

        let mut node = new_node(NodeKind::Case, tok_loc, file_no, line_no);
        node.label = Some(new_unique_name());
        node.val = val;
        let (lhs, tok) = stmt(
            files,
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
    if equal(files, tok, "default") {
        let sw = current_switch_get();
        current_switch_set(sw.clone());
        if sw.is_none() {
            return Err(error_tok(files, tok, "stray default"));
        }
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let tok = skip(files, tok.next.as_ref().unwrap(), ":")?;

        let mut node = new_node(NodeKind::Case, tok_loc, file_no, line_no);
        node.label = Some(new_unique_name());
        let (lhs, tok) = stmt(
            files,
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
    if tok.kind == TokenKind::Ident && equal(files, tok.next.as_ref().unwrap(), ":") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let mut node = new_node(NodeKind::Label, tok_loc, file_no, line_no);
        node.label = Some(
            files
                .iter()
                .find(|f| f.file_no == tok.file_no)
                .unwrap()
                .contents
                .chars()
                .skip(tok.loc)
                .take(tok.len)
                .collect(),
        );
        node.unique_label = Some(new_unique_name());
        let (lhs, tok) = stmt(
            files,
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
    if equal(files, tok, "{") {
        return compound_stmt(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            return_ty,
        );
    }
    expr_stmt(files, tok, locals, globals, scope_stack, tag_scope_stack)
}

#[allow(clippy::too_many_arguments)]
pub fn expr_stmt(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(files, tok, ";") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let tok = *tok.next.as_ref().unwrap().clone();
        return Ok((new_node(NodeKind::Block, tok_loc, file_no, line_no), tok));
    }
    let tok_loc = tok.loc;
    let file_no = tok.file_no;
    let line_no = tok.line_no;
    let (expr_node, tok) = expr(files, tok, locals, globals, scope_stack, tag_scope_stack)?;
    let tok = skip(files, &tok, ";")?;
    let node = new_unary(NodeKind::ExprStmt, expr_node, tok_loc, file_no, line_no);
    Ok((node, tok))
}

fn eval_double(files: &[File], node: &mut Node) -> Result<f64, String> {
    add_type(node);

    if crate::is_integer(node.ty.as_ref().unwrap()) {
        if node.ty.as_ref().unwrap().is_unsigned {
            return Ok(eval(files, node)? as u64 as f64);
        }
        return Ok(eval(files, node)? as f64);
    }

    match node.kind {
        NodeKind::Add => {
            let lhs = eval_double(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval_double(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs + rhs)
        }
        NodeKind::Sub => {
            let lhs = eval_double(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval_double(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs - rhs)
        }
        NodeKind::Mul => {
            let lhs = eval_double(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval_double(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs * rhs)
        }
        NodeKind::Div => {
            let lhs = eval_double(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval_double(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs / rhs)
        }
        NodeKind::Neg => {
            let lhs = eval_double(files, node.lhs.as_mut().unwrap())?;
            Ok(-lhs)
        }
        NodeKind::Cond => {
            let cond = eval_double(files, node.cond.as_mut().unwrap())?;
            if cond != 0.0 {
                eval_double(files, node.then.as_mut().unwrap())
            } else {
                eval_double(files, node.els.as_mut().unwrap())
            }
        }
        NodeKind::Comma => eval_double(files, node.rhs.as_mut().unwrap()),
        NodeKind::Cast => {
            if crate::is_flonum(node.lhs.as_ref().unwrap().ty.as_ref().unwrap()) {
                eval_double(files, node.lhs.as_mut().unwrap())
            } else {
                Ok(eval(files, node.lhs.as_mut().unwrap())? as f64)
            }
        }
        NodeKind::Num => Ok(node.fval),
        _ => Err(error_at(
            files,
            node.file_no,
            node.tok_loc,
            "not a compile-time constant",
        )),
    }
}

pub fn eval(files: &[File], node: &mut Node) -> Result<i64, String> {
    eval2(files, node, None)
}

pub fn eval2(
    files: &[File],
    node: &mut Node,
    label: Option<&mut Option<String>>,
) -> Result<i64, String> {
    add_type(node);

    if crate::is_flonum(node.ty.as_ref().unwrap()) {
        return Ok(eval_double(files, node)?.to_bits() as i64);
    }

    match node.kind {
        NodeKind::Add => {
            let lhs = eval2(files, node.lhs.as_mut().unwrap(), label)?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs.wrapping_add(rhs))
        }
        NodeKind::Sub => {
            let lhs = eval2(files, node.lhs.as_mut().unwrap(), label)?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs.wrapping_sub(rhs))
        }
        NodeKind::Mul => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs.wrapping_mul(rhs))
        }
        NodeKind::Div => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            if node.ty.as_ref().unwrap().is_unsigned {
                Ok((lhs as u64 / rhs as u64) as i64)
            } else {
                Ok(lhs.wrapping_div(rhs))
            }
        }
        NodeKind::Neg => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            Ok(-lhs)
        }
        NodeKind::Mod => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            if node.ty.as_ref().unwrap().is_unsigned {
                Ok((lhs as u64 % rhs as u64) as i64)
            } else {
                Ok(lhs % rhs)
            }
        }
        NodeKind::BitAnd => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs & rhs)
        }
        NodeKind::BitOr => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs | rhs)
        }
        NodeKind::BitXor => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs ^ rhs)
        }
        NodeKind::Shl => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok(lhs << rhs)
        }
        NodeKind::Shr => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            let ty = node.ty.as_ref().unwrap();
            if ty.is_unsigned && ty.size == 8 {
                Ok((lhs as u64 >> rhs as u64) as i64)
            } else {
                Ok(lhs >> rhs)
            }
        }
        NodeKind::Eq => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok((lhs == rhs) as i64)
        }
        NodeKind::Ne => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok((lhs != rhs) as i64)
        }
        NodeKind::Lt => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            if node.lhs.as_ref().unwrap().ty.as_ref().unwrap().is_unsigned {
                Ok(((lhs as u64) < (rhs as u64)) as i64)
            } else {
                Ok((lhs < rhs) as i64)
            }
        }
        NodeKind::Le => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            if node.lhs.as_ref().unwrap().ty.as_ref().unwrap().is_unsigned {
                Ok(((lhs as u64) <= (rhs as u64)) as i64)
            } else {
                Ok((lhs <= rhs) as i64)
            }
        }
        NodeKind::Cond => {
            let cond = eval(files, node.cond.as_mut().unwrap())?;
            if cond != 0 {
                eval2(files, node.then.as_mut().unwrap(), label)
            } else {
                eval2(files, node.els.as_mut().unwrap(), label)
            }
        }
        NodeKind::Comma => eval2(files, node.rhs.as_mut().unwrap(), label),
        NodeKind::Not => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            Ok((lhs == 0) as i64)
        }
        NodeKind::BitNot => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            Ok(!lhs)
        }
        NodeKind::LogAnd => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok((lhs != 0 && rhs != 0) as i64)
        }
        NodeKind::LogOr => {
            let lhs = eval(files, node.lhs.as_mut().unwrap())?;
            let rhs = eval(files, node.rhs.as_mut().unwrap())?;
            Ok((lhs != 0 || rhs != 0) as i64)
        }
        NodeKind::Cast => {
            let ty = node.ty.as_ref().unwrap();
            if crate::is_integer(ty) {
                let val = if crate::is_flonum(node.lhs.as_ref().unwrap().ty.as_ref().unwrap()) {
                    eval_double(files, node.lhs.as_mut().unwrap())? as i64
                } else {
                    eval2(files, node.lhs.as_mut().unwrap(), label)?
                };
                match ty.size {
                    1 => {
                        if ty.is_unsigned {
                            Ok((val as u8) as i64)
                        } else {
                            Ok((val as i8) as i64)
                        }
                    }
                    2 => {
                        if ty.is_unsigned {
                            Ok((val as u16) as i64)
                        } else {
                            Ok((val as i16) as i64)
                        }
                    }
                    4 => {
                        if ty.is_unsigned {
                            Ok((val as u32) as i64)
                        } else {
                            Ok((val as i32) as i64)
                        }
                    }
                    _ => Ok(val),
                }
            } else if ty.kind == TypeKind::Ptr {
                let val = eval2(files, node.lhs.as_mut().unwrap(), label)?;
                Ok(val)
            } else {
                Err(error_at(
                    files,
                    node.file_no,
                    node.tok_loc,
                    "not a compile-time constant",
                ))
            }
        }
        NodeKind::Addr => eval_rval(files, node.lhs.as_mut().unwrap(), label),
        NodeKind::Member => {
            if label.is_none() {
                return Err(error_at(
                    files,
                    node.file_no,
                    node.tok_loc,
                    "not a compile-time constant",
                ));
            }
            let ty = node.ty.as_ref().unwrap();
            if ty.kind != TypeKind::Array {
                return Err(error_at(
                    files,
                    node.file_no,
                    node.tok_loc,
                    "invalid initializer",
                ));
            }
            let offset = eval_rval(files, node.lhs.as_mut().unwrap(), label)?
                + node.member.as_ref().unwrap().offset;
            Ok(offset)
        }
        NodeKind::Var => {
            if label.is_none() {
                return Err(error_at(
                    files,
                    node.file_no,
                    node.tok_loc,
                    "not a compile-time constant",
                ));
            }
            let var = node.var.as_ref().unwrap();
            let ty = &var.ty;
            if ty.kind != TypeKind::Array && ty.kind != TypeKind::Func {
                return Err(error_at(
                    files,
                    node.file_no,
                    node.tok_loc,
                    "invalid initializer",
                ));
            }
            if let Some(l) = label {
                *l = Some(var.name.clone());
            }
            Ok(0)
        }
        NodeKind::Num => Ok(node.val),
        _ => Err(error_at(
            files,
            node.file_no,
            node.tok_loc,
            "not a compile-time constant",
        )),
    }
}

fn eval_rval(
    files: &[File],
    node: &mut Node,
    label: Option<&mut Option<String>>,
) -> Result<i64, String> {
    match node.kind {
        NodeKind::Var => {
            let var = node.var.as_ref().unwrap();
            if var.is_local {
                return Err(error_at(
                    files,
                    node.file_no,
                    node.tok_loc,
                    "not a compile-time constant",
                ));
            }
            if let Some(l) = label {
                *l = Some(var.name.clone());
            }
            Ok(0)
        }
        NodeKind::Deref => eval2(files, node.lhs.as_mut().unwrap(), label),
        NodeKind::Member => {
            let offset = eval_rval(files, node.lhs.as_mut().unwrap(), label)?
                + node.member.as_ref().unwrap().offset;
            Ok(offset)
        }
        _ => Err(error_at(
            files,
            node.file_no,
            node.tok_loc,
            "invalid initializer",
        )),
    }
}

pub fn const_expr(
    files: &[File],
    tok: &Token,
    tag_scope_stack: &mut [Vec<TagScope>],
    scope_stack: &mut [Vec<VarScope>],
) -> Result<(i64, Token), String> {
    let mut empty_locals: Vec<Obj> = Vec::new();
    let mut empty_globals: Vec<Obj> = Vec::new();
    let mut tag_scope_stack = tag_scope_stack.to_vec();
    let mut scope_stack = scope_stack.to_owned();
    let mut node = conditional(
        files,
        tok,
        &mut empty_locals,
        &mut empty_globals,
        &mut scope_stack,
        &mut tag_scope_stack,
    )?;
    let val = eval(files, &mut node.0)?;
    Ok((val, node.1))
}

pub fn expr(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (node, tok) = assign(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    if equal(files, &tok, ",") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = expr(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((
            new_binary(NodeKind::Comma, node, rhs, tok_loc, file_no, line_no),
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
    let file_no = binary.file_no;
    let line_no = binary.line_no;
    let lhs_ty = binary.lhs.as_ref().unwrap().ty.as_ref().unwrap().clone();

    let var = new_lvar(String::new(), pointer_to(lhs_ty), locals, scope_stack);

    let expr1 = new_binary(
        NodeKind::Assign,
        new_var_node(var.clone(), tok_loc, file_no, line_no),
        new_unary(
            NodeKind::Addr,
            binary.lhs.as_ref().unwrap().as_ref().clone(),
            tok_loc,
            file_no,
            line_no,
        ),
        tok_loc,
        file_no,
        line_no,
    );

    let deref_var = new_unary(
        NodeKind::Deref,
        new_var_node(var.clone(), tok_loc, file_no, line_no),
        tok_loc,
        file_no,
        line_no,
    );

    let op_node = new_binary(
        binary.kind,
        new_unary(
            NodeKind::Deref,
            new_var_node(var, tok_loc, file_no, line_no),
            tok_loc,
            file_no,
            line_no,
        ),
        binary.rhs.as_ref().unwrap().as_ref().clone(),
        tok_loc,
        file_no,
        line_no,
    );

    let expr2 = new_binary(
        NodeKind::Assign,
        deref_var,
        op_node,
        tok_loc,
        file_no,
        line_no,
    );

    new_binary(NodeKind::Comma, expr1, expr2, tok_loc, file_no, line_no)
}

#[allow(clippy::too_many_arguments)]
fn new_inc_dec(
    node: Node,
    tok_loc: usize,
    line_no: usize,
    file_no: usize,
    addend: i64,
    files: &[File],
    locals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
) -> Result<Node, String> {
    let mut node = node;
    add_type(&mut node);
    let ty = node.ty.as_ref().unwrap().clone();

    let binary = new_add(
        node,
        new_num(addend, tok_loc, file_no, line_no),
        tok_loc,
        line_no,
        file_no,
        files,
    )?;
    let assigned = to_assign(binary, locals, scope_stack);
    let result = new_add(
        assigned,
        new_num(-addend, tok_loc, file_no, line_no),
        tok_loc,
        line_no,
        file_no,
        files,
    )?;
    Ok(new_cast(result, ty))
}

pub fn assign(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, tok) = conditional(files, tok, locals, globals, scope_stack, tag_scope_stack)?;
    if equal(files, &tok, "=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::Assign, node, rhs, tok_loc, file_no, line_no);
        return Ok((node, tok));
    }

    if equal(files, &tok, "+=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_add(node, rhs, tok_loc, line_no, file_no, files)?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "-=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_sub(node, rhs, tok_loc, line_no, file_no, files)?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "*=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Mul, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "/=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Div, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "%=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Mod, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "&=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::BitAnd, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "|=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::BitOr, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "^=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::BitXor, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, "<<=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Shl, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, &tok, ">>=") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, tok) = assign(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_binary(NodeKind::Shr, node, rhs, tok_loc, file_no, line_no);
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    Ok((node, tok))
}

#[allow(clippy::too_many_arguments)]
pub fn conditional(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (cond, mut tok) = logor(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    if !equal(files, &tok, "?") {
        return Ok((cond, tok));
    }

    let tok_loc = tok.loc;
    let file_no = tok.file_no;
    let line_no = tok.line_no;
    let (then, new_tok) = expr(
        files,
        tok.next.as_ref().unwrap(),
        locals,
        globals,
        scope_stack,
        tag_scope_stack,
    )?;
    tok = skip(files, &new_tok, ":")?;

    let (els, tok) = conditional(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;

    let mut node = new_node(NodeKind::Cond, tok_loc, file_no, line_no);
    node.cond = Some(Box::new(cond));
    node.then = Some(Box::new(then));
    node.els = Some(Box::new(els));
    Ok((node, tok))
}

pub fn logor(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = logand(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    while equal(files, &tok, "||") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, new_tok) = logand(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::LogOr, node, rhs, tok_loc, file_no, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn logand(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = bitor(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    while equal(files, &tok, "&&") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, new_tok) = bitor(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::LogAnd, node, rhs, tok_loc, file_no, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn bitor(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = bitxor(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    while equal(files, &tok, "|") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, new_tok) = bitxor(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::BitOr, node, rhs, tok_loc, file_no, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn bitxor(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = bitand(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    while equal(files, &tok, "^") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, new_tok) = bitand(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::BitXor, node, rhs, tok_loc, file_no, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn bitand(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = equality(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    while equal(files, &tok, "&") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (rhs, new_tok) = equality(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        node = new_binary(NodeKind::BitAnd, node, rhs, tok_loc, file_no, line_no);
        tok = new_tok;
    }

    Ok((node, tok))
}

pub fn equality(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) =
        relational(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    loop {
        if equal(files, &tok, "==") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = relational(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Eq, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, "!=") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = relational(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Ne, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn relational(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = shift(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    loop {
        if equal(files, &tok, "<") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = shift(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Lt, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, "<=") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = shift(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Le, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, ">") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (lhs, new_tok) = shift(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Lt, lhs, node, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, ">=") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (lhs, new_tok) = shift(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Le, lhs, node, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn shift(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = add(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    loop {
        if equal(files, &tok, "<<") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = add(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Shl, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, ">>") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = add(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Shr, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn add(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = mul(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    loop {
        if equal(files, &tok, "+") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = mul(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_add(node, rhs, tok_loc, line_no, file_no, files)?;
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, "-") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = mul(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_sub(node, rhs, tok_loc, line_no, file_no, files)?;
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

pub fn mul(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, mut tok) = cast(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    loop {
        if equal(files, &tok, "*") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = cast(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Mul, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, "/") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = cast(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Div, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, "%") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (rhs, new_tok) = cast(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_binary(NodeKind::Mod, node, rhs, tok_loc, file_no, line_no);
            tok = new_tok;
            continue;
        }

        return Ok((node, tok));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn cast(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(files, tok, "(") && is_typename(files, tok.next.as_ref().unwrap(), scope_stack) {
        let start = tok;
        let tok_loc = tok.loc;
        let _file_no = tok.file_no;
        let (ty, new_tok) = typename(
            files,
            tok.next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(files, &new_tok, ")")?;

        if equal(files, &tok, "{") {
            return unary(files, start, locals, globals, scope_stack, tag_scope_stack);
        }

        let (node, tok) = cast(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        let mut node = new_cast(node, ty);
        node.tok_loc = tok_loc;
        return Ok((node, tok));
    }

    unary(files, tok, locals, globals, scope_stack, tag_scope_stack)
}

pub fn unary(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(files, tok, "+") {
        return cast(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        );
    }

    if equal(files, tok, "-") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((
            new_unary(NodeKind::Neg, node, tok_loc, file_no, line_no),
            tok,
        ));
    }

    if equal(files, tok, "&") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((
            new_unary(NodeKind::Addr, node, tok_loc, file_no, line_no),
            tok,
        ));
    }

    if equal(files, tok, "*") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (mut node, tok) = cast(
            files,
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
                files,
                file_no,
                tok_loc,
                "dereferencing a void pointer",
            ));
        }
        return Ok((
            new_unary(NodeKind::Deref, node, tok_loc, file_no, line_no),
            tok,
        ));
    }

    if equal(files, tok, "!") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((
            new_unary(NodeKind::Not, node, tok_loc, file_no, line_no),
            tok,
        ));
    }

    if equal(files, tok, "~") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (node, tok) = cast(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        return Ok((
            new_unary(NodeKind::BitNot, node, tok_loc, file_no, line_no),
            tok,
        ));
    }

    if equal(files, tok, "++") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (node, tok) = unary(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_add(
            node,
            new_num(1, tok_loc, file_no, line_no),
            tok_loc,
            line_no,
            file_no,
            files,
        )?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    if equal(files, tok, "--") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (node, tok) = unary(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let binary = new_sub(
            node,
            new_num(1, tok_loc, file_no, line_no),
            tok_loc,
            line_no,
            file_no,
            files,
        )?;
        return Ok((to_assign(binary, locals, scope_stack), tok));
    }

    postfix(files, tok, locals, globals, scope_stack, tag_scope_stack)
}

#[allow(clippy::too_many_arguments)]
pub fn postfix(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(files, tok, "(") && is_typename(files, tok.next.as_ref().unwrap(), scope_stack) {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (ty, tok) = typename(
            files,
            tok.next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(files, &tok, ")")?;

        if scope_stack.len() <= 1 {
            let mut var = new_anon_gvar(ty);
            let tok =
                gvar_initializer(files, &tok, &mut var, globals, tag_scope_stack, scope_stack)?;
            globals.push(var.clone());
            return Ok((new_var_node(var, tok_loc, file_no, line_no), tok));
        }

        let var = new_lvar(String::new(), ty, locals, scope_stack);
        let (lhs, tok) = lvar_initializer(
            files,
            &tok,
            &var.name,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let rhs = new_var_node(var, tok_loc, file_no, line_no);
        return Ok((
            new_binary(NodeKind::Comma, lhs, rhs, tok_loc, file_no, line_no),
            tok,
        ));
    }

    let (mut node, mut tok) = primary(files, tok, locals, globals, scope_stack, tag_scope_stack)?;

    loop {
        if equal(files, &tok, "(") {
            let (new_node, new_tok) = funcall(
                files,
                &tok,
                node,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            node = new_node;
            tok = new_tok;
            continue;
        }

        if equal(files, &tok, "[") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            let (idx, new_tok) = expr(
                files,
                tok.next.as_ref().unwrap(),
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
            )?;
            tok = skip(files, &new_tok, "]")?;
            node = new_unary(
                NodeKind::Deref,
                new_add(node, idx, tok_loc, line_no, file_no, files)?,
                tok_loc,
                file_no,
                line_no,
            );
            continue;
        }

        if equal(files, &tok, ".") {
            let tok_next = tok.next.as_ref().unwrap();
            node = struct_ref(files, node, tok_next)?;
            tok = *tok_next.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(files, &tok, "->") {
            node = new_unary(NodeKind::Deref, node, tok.loc, tok.file_no, tok.line_no);
            let tok_next = tok.next.as_ref().unwrap();
            node = struct_ref(files, node, tok_next)?;
            tok = *tok_next.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(files, &tok, "++") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            node = new_inc_dec(
                node,
                tok_loc,
                line_no,
                file_no,
                1,
                files,
                locals,
                scope_stack,
            )?;
            tok = *tok.next.as_ref().unwrap().clone();
            continue;
        }

        if equal(files, &tok, "--") {
            let tok_loc = tok.loc;
            let file_no = tok.file_no;
            let line_no = tok.line_no;
            node = new_inc_dec(
                node,
                tok_loc,
                line_no,
                file_no,
                -1,
                files,
                locals,
                scope_stack,
            )?;
            tok = *tok.next.as_ref().unwrap().clone();
            continue;
        }

        return Ok((node, tok));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn funcall(
    files: &[File],
    tok: &Token,
    mut fn_node: Node,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    add_type(&mut fn_node);

    let fn_ty = fn_node.ty.as_ref().unwrap();
    if fn_ty.kind != TypeKind::Func
        && (fn_ty.kind != TypeKind::Ptr
            || fn_ty.base.as_ref().unwrap().borrow().kind != TypeKind::Func)
    {
        return Err(error_tok(files, tok, "not a function"));
    }

    let tok_loc = fn_node.tok_loc;
    let file_no = fn_node.file_no;
    let line_no = fn_node.line_no;

    let ty = if fn_ty.kind == TypeKind::Func {
        fn_ty.clone()
    } else {
        fn_ty.base.as_ref().unwrap().borrow().clone()
    };

    let return_ty = ty.return_ty.as_ref().unwrap().as_ref().clone();
    let mut param_ty = ty.params.clone();

    let mut tok = skip(files, tok, "(")?;

    let mut head = Node {
        kind: NodeKind::Num,
        tok_loc,
        file_no,
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
        func_ty: None,
        args: None,
        pass_by_stack: false,
        var: None,
        val: 0,
        fval: 0.0,
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

    while !equal(files, &tok, ")") {
        if cur.tok_loc != tok_loc || cur.kind != NodeKind::Num {
            tok = skip(files, &tok, ",")?;
        }
        let (mut arg, new_tok) =
            assign(files, &tok, locals, globals, scope_stack, tag_scope_stack)?;
        tok = new_tok;
        add_type(&mut arg);

        if param_ty.is_none() && !ty.is_variadic {
            return Err(error_tok(files, &tok, "too many arguments"));
        }

        if let Some(pt) = param_ty {
            if pt.kind != TypeKind::Struct && pt.kind != TypeKind::Union {
                arg = new_cast(arg, pt.as_ref().clone());
            }
            param_ty = pt.next.clone();
        } else if arg.ty.as_ref().unwrap().kind == TypeKind::Float {
            arg = new_cast(arg, Type::new_double());
        }

        cur.next = Some(Box::new(arg));
        cur = cur.next.as_mut().unwrap();
    }

    if param_ty.is_some() {
        return Err(error_tok(files, &tok, "too few arguments"));
    }

    let tok = skip(files, &tok, ")")?;

    let mut node = new_unary(NodeKind::FuncCall, fn_node, tok_loc, file_no, line_no);
    node.func_ty = Some(ty);
    node.ty = Some(return_ty);
    node.args = head.next;
    Ok((node, tok))
}

pub fn primary(
    files: &[File],
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    if equal(files, tok, "(") && equal(files, tok.next.as_ref().unwrap(), "{") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (body, tok) = compound_stmt(
            files,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
            None,
        )?;
        let tok = skip(files, &tok, ")")?;
        let mut node = new_node(NodeKind::StmtExpr, tok_loc, file_no, line_no);
        node.body = body.body;
        return Ok((node, tok));
    }

    if equal(files, tok, "(") {
        let (node, tok) = expr(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        let tok = skip(files, &tok, ")")?;
        return Ok((node, tok));
    }

    if equal(files, tok, "sizeof")
        && equal(files, tok.next.as_ref().unwrap(), "(")
        && is_typename(
            files,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            scope_stack,
        )
    {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (ty, tok) = typename(
            files,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(files, &tok, ")")?;
        return Ok((new_ulong(ty.size, tok_loc, file_no, line_no), tok));
    }

    if equal(files, tok, "sizeof") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (mut node, tok) = unary(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        add_type(&mut node);
        let size = node.ty.as_ref().unwrap().size;
        return Ok((new_ulong(size, tok_loc, file_no, line_no), tok));
    }

    if equal(files, tok, "_Alignof")
        && equal(files, tok.next.as_ref().unwrap(), "(")
        && is_typename(
            files,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            scope_stack,
        )
    {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (ty, tok) = typename(
            files,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(files, &tok, ")")?;
        return Ok((new_ulong(ty.align, tok_loc, file_no, line_no), tok));
    }

    if equal(files, tok, "_Alignof") {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let (mut node, tok) = unary(
            files,
            tok.next.as_ref().unwrap(),
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        add_type(&mut node);
        let align = node.ty.as_ref().unwrap().align;
        return Ok((new_ulong(align, tok_loc, file_no, line_no), tok));
    }

    if tok.kind == TokenKind::Ident {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let name: String = files
            .iter()
            .find(|f| f.file_no == tok.file_no)
            .unwrap()
            .contents
            .chars()
            .skip(tok.loc)
            .take(tok.len)
            .collect();

        if name == "__builtin_reg_class" {
            let (ty, tok) = typename(
                files,
                tok.next.as_ref().unwrap().next.as_ref().unwrap(),
                tag_scope_stack,
                scope_stack,
            )?;
            let tok = skip(files, &tok, ")")?;

            if crate::is_integer(&ty) || ty.kind == TypeKind::Ptr {
                return Ok((new_num(0, tok_loc, file_no, line_no), tok));
            }
            if crate::is_flonum(&ty) {
                return Ok((new_num(1, tok_loc, file_no, line_no), tok));
            }
            return Ok((new_num(2, tok_loc, file_no, line_no), tok));
        }

        let sc = find_var(scope_stack, globals, &name);

        if let Some(sc) = sc {
            if let Some(var) = sc.var {
                return Ok((
                    new_var_node(var, tok_loc, file_no, line_no),
                    *tok.next.as_ref().unwrap().clone(),
                ));
            }
            if sc.enum_ty.is_some() {
                return Ok((
                    new_num(sc.enum_val, tok_loc, file_no, line_no),
                    *tok.next.as_ref().unwrap().clone(),
                ));
            }
        }

        if equal(files, tok.next.as_ref().unwrap(), "(") {
            return Err(error_tok(files, tok, "implicit declaration of a function"));
        }
        return Err(error_tok(files, tok, "undefined variable"));
    }

    if tok.kind == TokenKind::Str {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let str_content = tok.str.as_ref().unwrap();
        let ty = tok.ty.as_ref().unwrap().clone();
        let var = new_string_literal(str_content, ty);
        let node = new_var_node(var.clone(), tok_loc, file_no, line_no);
        globals.push(var);
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    if tok.kind == TokenKind::Num {
        let tok_loc = tok.loc;
        let file_no = tok.file_no;
        let line_no = tok.line_no;
        let ty = tok.ty.as_ref().unwrap();
        let mut node = if crate::is_flonum(ty) {
            let mut n = new_node(NodeKind::Num, tok_loc, file_no, line_no);
            n.fval = tok.fval;
            n
        } else {
            new_num(tok.val, tok_loc, file_no, line_no)
        };
        node.ty = tok.ty.clone();
        return Ok((node, *tok.next.as_ref().unwrap().clone()));
    }

    Err(error_tok(files, tok, "expected an expression"))
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
        is_unsigned: false,
        base: None,
        name: None,
        name_pos: None,
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

    if ty1.kind == TypeKind::Func {
        return pointer_to(ty1.clone());
    }
    if ty2.kind == TypeKind::Func {
        return pointer_to(ty2.clone());
    }

    if ty1.kind == TypeKind::Double || ty2.kind == TypeKind::Double {
        return Type::new_double();
    }
    if ty1.kind == TypeKind::Float || ty2.kind == TypeKind::Float {
        return Type::new_float();
    }

    let mut ty1 = ty1.clone();
    let mut ty2 = ty2.clone();

    if ty1.size < 4 {
        ty1 = Type::new_int();
    }
    if ty2.size < 4 {
        ty2 = Type::new_int();
    }

    if ty1.size != ty2.size {
        return if ty1.size < ty2.size { ty2 } else { ty1 };
    }

    if ty2.is_unsigned { ty2 } else { ty1 }
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
    file_no: usize,
    files: &[File],
) -> Result<Node, String> {
    let mut lhs = lhs;
    let mut rhs = rhs;
    add_type(&mut lhs);
    add_type(&mut rhs);

    let lhs_ty = lhs.ty.as_ref().unwrap();
    let rhs_ty = rhs.ty.as_ref().unwrap();

    if crate::is_numeric(lhs_ty) && crate::is_numeric(rhs_ty) {
        return Ok(new_binary(
            NodeKind::Add,
            lhs,
            rhs,
            tok_loc,
            file_no,
            line_no,
        ));
    }

    if lhs_ty.kind == TypeKind::Ptr && rhs_ty.kind == TypeKind::Ptr {
        return Err(error_at(files, file_no, tok_loc, "invalid operands"));
    }

    if lhs_ty.kind == TypeKind::Array && rhs_ty.kind == TypeKind::Array {
        return Err(error_at(files, file_no, tok_loc, "invalid operands"));
    }

    if !crate::is_integer(lhs_ty) && !crate::is_integer(rhs_ty) {
        return Err(error_at(files, file_no, tok_loc, "invalid operands"));
    }

    if crate::is_integer(lhs_ty) && (rhs_ty.kind == TypeKind::Ptr || rhs_ty.kind == TypeKind::Array)
    {
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
        new_long(base_size, tok_loc, file_no, line_no),
        tok_loc,
        file_no,
        line_no,
    );
    Ok(new_binary(
        NodeKind::Add,
        lhs,
        rhs,
        tok_loc,
        file_no,
        line_no,
    ))
}

pub fn new_sub(
    lhs: Node,
    rhs: Node,
    tok_loc: usize,
    line_no: usize,
    file_no: usize,
    files: &[File],
) -> Result<Node, String> {
    let mut lhs = lhs;
    let mut rhs = rhs;
    add_type(&mut lhs);
    add_type(&mut rhs);

    let lhs_ty = lhs.ty.as_ref().unwrap();
    let rhs_ty = rhs.ty.as_ref().unwrap();

    if crate::is_numeric(lhs_ty) && crate::is_numeric(rhs_ty) {
        return Ok(new_binary(
            NodeKind::Sub,
            lhs,
            rhs,
            tok_loc,
            file_no,
            line_no,
        ));
    }

    if (lhs_ty.kind == TypeKind::Ptr || lhs_ty.kind == TypeKind::Array) && crate::is_integer(rhs_ty)
    {
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
            new_long(base_size, tok_loc, file_no, line_no),
            tok_loc,
            file_no,
            line_no,
        );
        add_type(&mut rhs);
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc, file_no, line_no);
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
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc, file_no, line_no);
        node.ty = Some(Type::new_long());
        let mut result = new_binary(
            NodeKind::Div,
            node,
            new_long(base_size, tok_loc, file_no, line_no),
            tok_loc,
            file_no,
            line_no,
        );
        result.ty = Some(Type::new_long());
        return Ok(result);
    }

    Err(error_at(files, file_no, tok_loc, "invalid operands"))
}

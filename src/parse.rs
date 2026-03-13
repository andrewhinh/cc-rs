use crate::{
    Node, NodeKind, Obj, TagScope, Token, TokenKind, Type, TypeKind, VarAttr, VarScope, align_to,
    error_at, error_tok, new_unique_name, new_var_unique_id,
};
use crate::{consume, equal, skip};

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

pub fn find_typedef(scope_stack: &[Vec<VarScope>], tok: &Token, src: &str) -> Option<Type> {
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

pub fn find_tag(tag_scope_stack: &[Vec<TagScope>], name: &str) -> Option<Type> {
    for scope in tag_scope_stack.iter().rev() {
        for ts in scope.iter().rev() {
            if ts.name == name {
                return Some(ts.ty.clone());
            }
        }
    }
    None
}

pub fn push_tag_scope(tag_scope_stack: &mut [Vec<TagScope>], name: String, ty: Type) {
    tag_scope_stack
        .last_mut()
        .unwrap()
        .push(TagScope { name, ty });
}

pub fn new_var(name: String, ty: Type) -> Obj {
    Obj {
        name,
        ty,
        is_local: false,
        offset: 0,
        is_function: false,
        is_definition: false,
        is_static: false,
        init_data: None,
        params: Vec::new(),
        body: None,
        locals: Vec::new(),
        stack_size: 0,
        unique_id: new_var_unique_id(),
    }
}

pub fn new_anon_gvar(ty: Type) -> Obj {
    new_var(new_unique_name(), ty)
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
    let mut offset = 0;
    for v in locals.iter() {
        offset += v.ty.size;
    }
    offset += var.ty.size;
    var.offset = offset;
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
    var
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
    ty: &mut Type,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<Token, String> {
    let mut tok = tok.clone();
    let mut members: Vec<crate::Member> = Vec::new();

    while !equal(src, &tok, "}") {
        let mut empty_scope: Vec<Vec<VarScope>> = vec![];
        let (basety, new_tok) =
            declspec(filename, src, &tok, tag_scope_stack, &mut empty_scope, None)?;
        tok = new_tok;
        let mut i = 0;

        while !equal(src, &tok, ";") {
            if i > 0 {
                tok = skip(filename, src, &tok, ",")?;
            }
            i += 1;

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
            let mem = crate::Member {
                next: None,
                ty: mem_ty.clone(),
                name: mem_ty.name.clone(),
                offset: 0,
            };
            members.push(mem);
        }
        tok = skip(filename, src, &tok, ";")?;
    }

    let rest = tok.next.as_ref().unwrap().as_ref().clone();

    if members.is_empty() {
        ty.members = None;
    } else {
        let mut current: Option<Box<crate::Member>> = None;
        for mem in members.into_iter().rev() {
            let mut m = mem;
            m.next = current;
            current = Some(Box::new(m));
        }
        ty.members = current;
    }

    Ok(rest)
}

pub fn struct_union_decl(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Type, Token), String> {
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
        return Err(error_tok(filename, src, tag_tok, "unknown struct type"));
    }

    tok = skip(filename, src, &tok, "{")?;

    let mut ty = Type {
        kind: TypeKind::Struct,
        size: 0,
        align: 1,
        base: None,
        name: None,
        return_ty: None,
        params: None,
        next: None,
        array_len: 0,
        members: None,
    };

    let rest = struct_members(filename, src, &tok, &mut ty, tag_scope_stack)?;

    if let Some(tag_tok) = tag {
        let tag_name: String = src.chars().skip(tag_tok.loc).take(tag_tok.len).collect();
        push_tag_scope(tag_scope_stack, tag_name, ty.clone());
    }

    Ok((ty, rest))
}

pub fn struct_decl(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Type, Token), String> {
    let (mut ty, rest) = struct_union_decl(filename, src, tok, tag_scope_stack)?;
    ty.kind = TypeKind::Struct;

    let mut offset = 0;
    let mut current = ty.members.as_mut();
    while let Some(mem) = current {
        offset = align_to(offset, mem.ty.align);
        mem.offset = offset;
        offset += mem.ty.size;

        if ty.align < mem.ty.align {
            ty.align = mem.ty.align;
        }

        current = mem.next.as_mut();
    }
    ty.size = align_to(offset, ty.align);

    Ok((ty, rest))
}

pub fn union_decl(
    filename: &str,
    src: &str,
    tok: &Token,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Type, Token), String> {
    let (mut ty, rest) = struct_union_decl(filename, src, tok, tag_scope_stack)?;
    ty.kind = TypeKind::Union;

    for mem in ty.members.iter() {
        if ty.align < mem.ty.align {
            ty.align = mem.ty.align;
        }
        if ty.size < mem.ty.size {
            ty.size = mem.ty.size;
        }
    }
    ty.size = align_to(ty.size, ty.align);

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
            if ty.kind != TypeKind::Enum {
                return Err(error_tok(filename, src, tag_tok, "not an enum tag"));
            }
            return Ok((ty, tok));
        }
        return Err(error_tok(filename, src, tag_tok, "unknown enum type"));
    }

    tok = skip(filename, src, &tok, "{")?;

    let mut val: i64 = 0;
    let mut i = 0;

    while !equal(src, &tok, "}") {
        if i > 0 {
            tok = skip(filename, src, &tok, ",")?;
        }
        i += 1;

        let name = get_ident(src, &tok)?;
        tok = tok.next.as_ref().unwrap().as_ref().clone();

        if equal(src, &tok, "=") {
            val = get_number(tok.next.as_ref().unwrap())?;
            tok = tok
                .next
                .as_ref()
                .unwrap()
                .next
                .as_ref()
                .unwrap()
                .as_ref()
                .clone();
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

    let rest = tok.next.as_ref().unwrap().as_ref().clone();

    if let Some(tag_tok) = tag {
        let tag_name: String = src.chars().skip(tag_tok.loc).take(tag_tok.len).collect();
        push_tag_scope(tag_scope_stack, tag_name, ty.clone());
    }

    Ok((ty, rest))
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
        if equal(src, &tok, "typedef") || equal(src, &tok, "static") {
            if let Some(a) = attr.as_mut() {
                if equal(src, &tok, "typedef") {
                    a.is_typedef = true;
                } else {
                    a.is_static = true;
                }
                if a.is_typedef as i32 + a.is_static as i32 > 1 {
                    return Err(error_tok(
                        filename,
                        src,
                        &tok,
                        "typedef and static may not be used together",
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
                ty = ty2.unwrap();
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

pub fn global_variable(
    filename: &str,
    src: &str,
    tok: &Token,
    basety: Type,
    globals: &mut Vec<Obj>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
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
            tag_scope_stack,
            scope_stack,
        )?;
        tok = new_tok;
        let name = get_ident(src, ty.name.as_ref().unwrap())?;
        let var = new_gvar(name, ty);
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
    };
    let mut cur = &mut head;
    let mut first = true;

    while !equal(src, &tok, ")") {
        if !first {
            tok = skip(filename, src, &tok, ",")?;
        }
        first = false;

        let (basety, new_tok) = declspec(filename, src, &tok, tag_scope_stack, scope_stack, None)?;
        tok = new_tok;
        let (param_ty, new_tok) =
            declarator(filename, src, &tok, basety, tag_scope_stack, scope_stack)?;
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
        let sz = get_number(tok.next.as_ref().unwrap())?;
        let tok = skip(
            filename,
            src,
            tok.next.as_ref().unwrap().next.as_ref().unwrap(),
            "]",
        )?;
        let (ty, rest) = type_suffix(filename, src, &tok, ty, tag_scope_stack, scope_stack)?;
        return Ok((Type::new_array(ty, sz), rest));
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
        let var = new_lvar(name, ty.clone(), locals, scope_stack);

        if !equal(src, &tok, "=") {
            continue;
        }

        let tok_loc = tok.loc;
        let line_no = tok.line_no;
        let tok_next = tok.next.as_ref().unwrap().clone();
        let (rhs, new_tok) = assign(
            filename,
            src,
            &tok_next,
            locals,
            globals,
            scope_stack,
            tag_scope_stack,
        )?;
        tok = new_tok;
        let lhs = new_var_node(
            var,
            ty.name.as_ref().unwrap().loc,
            ty.name.as_ref().unwrap().line_no,
        );
        let node = new_binary(NodeKind::Assign, lhs, rhs, tok_loc, line_no);
        cur.next = Some(Box::new(new_unary(
            NodeKind::ExprStmt,
            node,
            tok_loc,
            line_no,
        )));
        cur = cur.next.as_mut().unwrap();
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
        scope_stack.last_mut().unwrap().push(VarScope {
            name,
            var: None,
            type_def: Some(ty),
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
    };
    let mut cur = &mut head;

    let mut tok = tok.clone();
    while !equal(src, &tok, "}") {
        if is_typename(src, &tok, scope_stack) {
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

            let (node, new_tok) = declaration(
                filename,
                src,
                &tok,
                basety,
                locals,
                globals,
                scope_stack,
                tag_scope_stack,
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
        let (mut expr_node, tok) = expr(
            filename,
            src,
            tok.next.as_ref().unwrap(),
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

pub fn assign(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
    let (mut node, tok) = equality(
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
        if equal(src, &tok, "<") {
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
            node = new_binary(NodeKind::Lt, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, "<=") {
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
            node = new_binary(NodeKind::Le, node, rhs, tok_loc, line_no);
            tok = new_tok;
            continue;
        }

        if equal(src, &tok, ">") {
            let tok_loc = tok.loc;
            let line_no = tok.line_no;
            let (lhs, new_tok) = add(
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
            let (lhs, new_tok) = add(
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
        let tok_loc = tok.loc;
        let (ty, tok) = typename(
            filename,
            src,
            tok.next.as_ref().unwrap(),
            tag_scope_stack,
            scope_stack,
        )?;
        let tok = skip(filename, src, &tok, ")")?;
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
            && lhs_ty.base.as_ref().unwrap().kind == TypeKind::Void
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

pub fn postfix(
    filename: &str,
    src: &str,
    tok: &Token,
    locals: &mut Vec<Obj>,
    globals: &mut Vec<Obj>,
    scope_stack: &mut Vec<Vec<VarScope>>,
    tag_scope_stack: &mut Vec<Vec<TagScope>>,
) -> Result<(Node, Token), String> {
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
    Type::new_ptr(base)
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

pub fn get_common_type(ty1: &Type, ty2: &Type) -> Type {
    if let Some(base) = &ty1.base {
        return Type::new_ptr(base.as_ref().clone());
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
        NodeKind::Add | NodeKind::Sub | NodeKind::Mul | NodeKind::Div => {
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
        NodeKind::Return
        | NodeKind::If
        | NodeKind::For
        | NodeKind::While
        | NodeKind::Block
        | NodeKind::ExprStmt
        | NodeKind::Cast => {}
        NodeKind::Var => {
            node.ty = Some(node.var.as_ref().unwrap().ty.clone());
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

    let base_size = lhs.ty.as_ref().unwrap().base.as_ref().unwrap().size;
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
        let base_size = lhs.ty.as_ref().unwrap().base.as_ref().unwrap().size;
        let rhs = new_binary(
            NodeKind::Mul,
            rhs,
            new_long(base_size, tok_loc, line_no),
            tok_loc,
            line_no,
        );
        let mut node = new_binary(NodeKind::Sub, lhs, rhs, tok_loc, line_no);
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

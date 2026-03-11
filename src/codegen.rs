use crate::{
    Node, NodeKind, Obj, TagScope, TokenKind, Type, TypeKind, VarAttr, VarScope, error_at,
};
use crate::{declspec, is_function, parse_typedef, tokenize};
use crate::{function, global_variable};

fn gen_addr(
    node: &Node,
    result: &mut String,
    filename: &str,
    src: &str,
    current_fn: &str,
) -> Result<(), String> {
    match node.kind {
        NodeKind::Var => {
            let var = node.var.as_ref().unwrap();
            if var.is_local {
                result.push_str(&format!("  lea -{}(%rbp), %rax\n", var.offset));
            } else {
                result.push_str(&format!("  lea {}(%rip), %rax\n", var.name));
            }
        }
        NodeKind::Deref => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
        }
        NodeKind::Member => {
            gen_addr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            let offset = node.member.as_ref().unwrap().offset;
            result.push_str(&format!("  add ${}, %rax\n", offset));
        }
        NodeKind::Comma => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            gen_addr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
        }
        _ => return Err(error_at(filename, src, node.tok_loc, "not an lvalue")),
    }
    Ok(())
}

fn load(ty: &Type, result: &mut String) {
    if ty.kind == TypeKind::Array || ty.kind == TypeKind::Struct || ty.kind == TypeKind::Union {
        return;
    }
    if ty.size == 1 {
        result.push_str("  movsbq (%rax), %rax\n");
    } else if ty.size == 2 {
        result.push_str("  movswq (%rax), %rax\n");
    } else if ty.size == 4 {
        result.push_str("  movsxd (%rax), %rax\n");
    } else {
        result.push_str("  mov (%rax), %rax\n");
    }
}

fn store(ty: &Type, result: &mut String) {
    result.push_str("  pop %rdi\n");

    if ty.kind == TypeKind::Struct || ty.kind == TypeKind::Union {
        for i in 0..ty.size {
            result.push_str(&format!("  mov {}(%rax), %r8b\n", i));
            result.push_str(&format!("  mov %r8b, {}(%rdi)\n", i));
        }
        return;
    }

    if ty.size == 1 {
        result.push_str("  mov %al, (%rdi)\n");
    } else if ty.size == 2 {
        result.push_str("  mov %ax, (%rdi)\n");
    } else if ty.size == 4 {
        result.push_str("  mov %eax, (%rdi)\n");
    } else {
        result.push_str("  mov %rax, (%rdi)\n");
    }
}

const I8: usize = 0;
const I16: usize = 1;
const I32: usize = 2;
const I64: usize = 3;

fn get_type_id(ty: &Type) -> usize {
    match ty.kind {
        TypeKind::Char => I8,
        TypeKind::Short => I16,
        TypeKind::Int => I32,
        _ => I64,
    }
}

fn cast_type(from: &Type, to: &Type, result: &mut String) {
    if to.kind == TypeKind::Void {
        return;
    }

    let t1 = get_type_id(from);
    let t2 = get_type_id(to);

    let cast_table: [[Option<&str>; 4]; 4] = [
        [None, None, None, Some("movsbl %al, %eax")],
        [
            Some("movsbl %al, %eax"),
            None,
            None,
            Some("movswl %ax, %eax"),
        ],
        [
            Some("movsbl %al, %eax"),
            Some("movswl %ax, %eax"),
            None,
            Some("movsxd %eax, %rax"),
        ],
        [
            Some("movsbl %al, %eax"),
            Some("movswl %ax, %eax"),
            None,
            None,
        ],
    ];

    if let Some(inst) = cast_table[t1][t2] {
        result.push_str(&format!("  {}\n", inst));
    }
}

fn gen_expr(
    node: &Node,
    result: &mut String,
    filename: &str,
    src: &str,
    current_fn: &str,
) -> Result<(), String> {
    result.push_str(&format!("  .loc 1 {}\n", node.line_no));

    match node.kind {
        NodeKind::Num => {
            result.push_str(&format!("  mov ${}, %rax\n", node.val));
            return Ok(());
        }
        NodeKind::Neg => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            result.push_str("  neg %rax\n");
            return Ok(());
        }
        NodeKind::Var => {
            gen_addr(node, result, filename, src, current_fn)?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Member => {
            gen_addr(node, result, filename, src, current_fn)?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Addr => {
            gen_addr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            return Ok(());
        }
        NodeKind::Deref => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Assign => {
            gen_addr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            result.push_str("  push %rax\n");
            gen_expr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            store(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::FuncCall => {
            let argreg = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
            let mut nargs = 0;
            let mut arg = node.args.as_ref();
            while let Some(arg_node) = arg {
                gen_expr(arg_node, result, filename, src, current_fn)?;
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
        NodeKind::StmtExpr => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                gen_stmt(stmt_node, result, filename, src, current_fn)?;
                n = stmt_node.next.as_ref();
            }
            return Ok(());
        }
        NodeKind::Comma => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            gen_expr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            return Ok(());
        }
        NodeKind::Cast => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            cast_type(
                node.lhs.as_ref().unwrap().ty.as_ref().unwrap(),
                node.ty.as_ref().unwrap(),
                result,
            );
            return Ok(());
        }
        _ => {}
    }

    gen_expr(
        node.rhs.as_ref().unwrap(),
        result,
        filename,
        src,
        current_fn,
    )?;
    result.push_str("  push %rax\n");
    gen_expr(
        node.lhs.as_ref().unwrap(),
        result,
        filename,
        src,
        current_fn,
    )?;
    result.push_str("  pop %rdi\n");

    let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
    let (ax, di) = if lhs_ty.kind == TypeKind::Long || lhs_ty.base.is_some() {
        ("%rax", "%rdi")
    } else {
        ("%eax", "%edi")
    };

    match node.kind {
        NodeKind::Add => result.push_str(&format!("  add {}, {}\n", di, ax)),
        NodeKind::Sub => result.push_str(&format!("  sub {}, {}\n", di, ax)),
        NodeKind::Mul => result.push_str(&format!("  imul {}, {}\n", di, ax)),
        NodeKind::Div => {
            if lhs_ty.size == 8 {
                result.push_str("  cqo\n");
            } else {
                result.push_str("  cdq\n");
            }
            result.push_str(&format!("  idiv {}\n", di));
        }
        NodeKind::Eq | NodeKind::Ne | NodeKind::Lt | NodeKind::Le => {
            result.push_str(&format!("  cmp {}, {}\n", di, ax));
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
        | NodeKind::StmtExpr
        | NodeKind::Var
        | NodeKind::Member
        | NodeKind::Assign
        | NodeKind::Addr
        | NodeKind::Deref
        | NodeKind::Return
        | NodeKind::Block
        | NodeKind::If
        | NodeKind::For
        | NodeKind::While
        | NodeKind::Comma
        | NodeKind::Cast => unreachable!(),
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
    result.push_str(&format!("  .loc 1 {}\n", node.line_no));

    match node.kind {
        NodeKind::If => {
            let c = count();
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
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
                gen_expr(cond, result, filename, src, current_fn)?;
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
                gen_expr(inc, result, filename, src, current_fn)?;
            }
            result.push_str(&format!("  jmp .L.begin.{}\n", c));
            result.push_str(&format!(".L.end.{}:\n", c));
        }
        NodeKind::While => {
            let c = count();
            result.push_str(&format!(".L.begin.{}:\n", c));
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
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
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
            result.push_str(&format!("  jmp .L.return.{}\n", current_fn));
        }
        NodeKind::ExprStmt => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
            )?;
        }
        _ => return Err(error_at(filename, src, node.tok_loc, "invalid statement")),
    }
    Ok(())
}

fn store_gp(r: usize, offset: i64, sz: i64, result: &mut String) {
    let argreg8 = ["%dil", "%sil", "%dl", "%cl", "%r8b", "%r9b"];
    let argreg16 = ["%di", "%si", "%dx", "%cx", "%r8w", "%r9w"];
    let argreg32 = ["%edi", "%esi", "%edx", "%ecx", "%r8d", "%r9d"];
    let argreg64 = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
    match sz {
        1 => result.push_str(&format!("  mov {}, -{}(%rbp)\n", argreg8[r], offset)),
        2 => result.push_str(&format!("  mov {}, -{}(%rbp)\n", argreg16[r], offset)),
        4 => result.push_str(&format!("  mov {}, -{}(%rbp)\n", argreg32[r], offset)),
        8 => result.push_str(&format!("  mov {}, -{}(%rbp)\n", argreg64[r], offset)),
        _ => unreachable!(),
    }
}

fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
}

fn fix_var_offsets(node: &mut Node, locals: &[Obj]) {
    if let Some(var) = &mut node.var
        && let Some(lv) = locals.iter().find(|l| l.name == var.name)
    {
        var.offset = lv.offset;
    }
    if let Some(lhs) = &mut node.lhs {
        fix_var_offsets(lhs, locals);
    }
    if let Some(rhs) = &mut node.rhs {
        fix_var_offsets(rhs, locals);
    }
    if let Some(cond) = &mut node.cond {
        fix_var_offsets(cond, locals);
    }
    if let Some(then) = &mut node.then {
        fix_var_offsets(then, locals);
    }
    if let Some(els) = &mut node.els {
        fix_var_offsets(els, locals);
    }
    if let Some(init) = &mut node.init {
        fix_var_offsets(init, locals);
    }
    if let Some(inc) = &mut node.inc {
        fix_var_offsets(inc, locals);
    }
    if let Some(body) = &mut node.body {
        let mut n = body;
        loop {
            fix_var_offsets(n, locals);
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
            fix_var_offsets(n, locals);
            if let Some(next) = &mut n.next {
                n = next;
            } else {
                break;
            }
        }
    }
}

pub fn emit_assembly(filename: &str, src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let tok = tokenize(filename, src)?;

    let mut globals: Vec<Obj> = Vec::new();
    let mut tag_scope_stack: Vec<Vec<TagScope>> = vec![Vec::new()];
    let mut scope_stack: Vec<Vec<VarScope>> = vec![Vec::new()];

    let mut tok = tok;
    while tok.kind != TokenKind::Eof {
        let mut attr = VarAttr::default();
        let (basety, new_tok) = declspec(
            filename,
            src,
            &tok,
            &mut tag_scope_stack,
            &scope_stack,
            Some(&mut attr),
        )?;
        tok = new_tok;

        if attr.is_typedef {
            tok = parse_typedef(filename, src, &tok, basety, &mut scope_stack)?;
            continue;
        }

        if is_function(src, &tok)? {
            let (func, new_tok) = function(
                filename,
                src,
                &tok,
                basety,
                &mut globals,
                &mut tag_scope_stack,
                &scope_stack,
            )?;
            tok = new_tok;
            globals.push(func);
        } else {
            tok = global_variable(
                filename,
                src,
                &tok,
                basety,
                &mut globals,
                &mut tag_scope_stack,
                &scope_stack,
            )?;
        }
    }

    let mut result = String::new();
    result.push_str(&format!(".file 1 \"{}\"\n", filename));

    let mut has_data = false;
    for var in globals.iter() {
        if var.is_function {
            continue;
        }
        if !has_data {
            result.push_str("  .data\n");
            has_data = true;
        }
        result.push_str(&format!("  .globl {}\n", var.name));
        result.push_str(&format!("{}:\n", var.name));

        if let Some(init_data) = &var.init_data {
            for byte in init_data {
                result.push_str(&format!("  .byte {}\n", byte));
            }
        } else {
            result.push_str(&format!("  .zero {}\n", var.ty.size));
        }
    }

    for func in globals.iter_mut() {
        if !func.is_function || !func.is_definition {
            continue;
        }

        let mut offset = 0;
        for var in func.locals.iter_mut().rev() {
            offset += var.ty.size;
            offset = align_to(offset, var.ty.align);
            var.offset = offset;
        }
        let stack_size = align_to(offset, 16);

        let locals = func.locals.clone();
        if let Some(body) = &mut func.body {
            fix_var_offsets(body, &locals);
        }

        result.push_str("  .text\n");
        result.push_str(&format!("  .globl {}\n", func.name));
        result.push_str(&format!("{}:\n", func.name));

        result.push_str("  push %rbp\n");
        result.push_str("  mov %rsp, %rbp\n");
        result.push_str(&format!("  sub ${}, %rsp\n", stack_size));

        for (i, var) in func.params.iter_mut().enumerate() {
            let local_var = func.locals.iter().find(|l| l.name == var.name);
            if let Some(lv) = local_var {
                var.offset = lv.offset;
            }
            store_gp(i, var.offset, var.ty.size, &mut result);
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
    }

    Ok(result)
}

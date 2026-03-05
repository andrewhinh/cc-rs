use crate::{Node, NodeKind, Obj, TokenKind, Type, TypeKind, error_at};
use crate::{declspec, is_function, tokenize};
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
    if ty.kind == TypeKind::Array {
        return;
    }
    if ty.size == 1 {
        result.push_str("  movsbq (%rax), %rax\n");
    } else {
        result.push_str("  mov (%rax), %rax\n");
    }
}

fn store(ty: &Type, result: &mut String) {
    result.push_str("  pop %rdi\n");
    if ty.size == 1 {
        result.push_str("  mov %al, (%rdi)\n");
    } else {
        result.push_str("  mov %rax, (%rdi)\n");
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
        | NodeKind::StmtExpr
        | NodeKind::Var
        | NodeKind::Assign
        | NodeKind::Addr
        | NodeKind::Deref
        | NodeKind::Return
        | NodeKind::Block
        | NodeKind::If
        | NodeKind::For
        | NodeKind::While
        | NodeKind::Comma => unreachable!(),
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

fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
}

pub fn emit_assembly(filename: &str, src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let tok = tokenize(filename, src)?;

    let mut globals: Vec<Obj> = Vec::new();

    let mut tok = tok;
    while tok.kind != TokenKind::Eof {
        let (basety, new_tok) = declspec(filename, src, &tok)?;
        tok = new_tok;

        if is_function(src, &tok)? {
            let (func, new_tok) = function(filename, src, &tok, basety, &mut globals)?;
            tok = new_tok;
            globals.push(func);
        } else {
            tok = global_variable(filename, src, &tok, basety, &mut globals)?;
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

    for func in globals.iter() {
        if !func.is_function {
            continue;
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

        let argreg8 = ["%dil", "%sil", "%dl", "%cl", "%r8b", "%r9b"];
        let argreg64 = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
        for (i, var) in func.params.iter().enumerate() {
            if var.ty.size == 1 {
                result.push_str(&format!("  mov {}, -{}(%rbp)\n", argreg8[i], var.offset));
            } else {
                result.push_str(&format!("  mov {}, -{}(%rbp)\n", argreg64[i], var.offset));
            }
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

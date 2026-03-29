use crate::preprocess::preprocess;
use crate::tokenize::tokenize;
use crate::{
    Node, NodeKind, Obj, TagScope, TokenKind, Type, TypeKind, VarAttr, VarScope, error_at,
};
use crate::{declspec, is_function, parse_typedef};
use crate::{function, global_variable};

fn gen_addr(
    node: &Node,
    result: &mut String,
    filename: &str,
    src: &str,
    current_fn: &str,
    depth: &mut i32,
) -> Result<(), String> {
    match node.kind {
        NodeKind::Var => {
            let var = node.var.as_ref().unwrap();
            if var.is_local {
                result.push_str(&format!("  lea -{}(%rbp), %rax\n", var.offset));
                return Ok(());
            }

            if node.ty.as_ref().unwrap().kind == TypeKind::Func {
                if var.is_definition {
                    result.push_str(&format!("  lea {}(%rip), %rax\n", var.name));
                } else {
                    result.push_str(&format!("  mov {}@GOTPCREL(%rip), %rax\n", var.name));
                }
                return Ok(());
            }

            result.push_str(&format!("  lea {}(%rip), %rax\n", var.name));
        }
        NodeKind::Deref => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
        }
        NodeKind::Member => {
            gen_addr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
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
                depth,
            )?;
            gen_addr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
        }
        _ => return Err(error_at(filename, src, node.tok_loc, "not an lvalue")),
    }
    Ok(())
}

fn load(ty: &Type, result: &mut String) {
    match ty.kind {
        TypeKind::Array | TypeKind::Struct | TypeKind::Union | TypeKind::Func => return,
        TypeKind::Float => {
            result.push_str("  movss (%rax), %xmm0\n");
            return;
        }
        TypeKind::Double => {
            result.push_str("  movsd (%rax), %xmm0\n");
            return;
        }
        _ => {}
    }
    let insn = if ty.is_unsigned { "movz" } else { "movs" };
    if ty.size == 1 {
        result.push_str(&format!("  {}bl (%rax), %eax\n", insn));
    } else if ty.size == 2 {
        result.push_str(&format!("  {}wl (%rax), %eax\n", insn));
    } else if ty.size == 4 {
        result.push_str("  movsxd (%rax), %rax\n");
    } else {
        result.push_str("  mov (%rax), %rax\n");
    }
}

fn store(ty: &Type, result: &mut String, depth: &mut i32) {
    result.push_str("  pop %rdi\n");
    *depth -= 1;

    match ty.kind {
        TypeKind::Struct | TypeKind::Union => {
            for i in 0..ty.size {
                result.push_str(&format!("  mov {}(%rax), %r8b\n", i));
                result.push_str(&format!("  mov %r8b, {}(%rdi)\n", i));
            }
            return;
        }
        TypeKind::Float => {
            result.push_str("  movss %xmm0, (%rdi)\n");
            return;
        }
        TypeKind::Double => {
            result.push_str("  movsd %xmm0, (%rdi)\n");
            return;
        }
        _ => {}
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

fn cmp_zero(ty: &Type, result: &mut String) {
    match ty.kind {
        TypeKind::Float => {
            result.push_str("  xorps %xmm1, %xmm1\n");
            result.push_str("  ucomiss %xmm1, %xmm0\n");
            return;
        }
        TypeKind::Double => {
            result.push_str("  xorpd %xmm1, %xmm1\n");
            result.push_str("  ucomisd %xmm1, %xmm0\n");
            return;
        }
        _ => {}
    }
    if crate::is_integer(ty) && ty.size <= 4 {
        result.push_str("  cmp $0, %eax\n");
    } else {
        result.push_str("  cmp $0, %rax\n");
    }
}

fn pushf(result: &mut String, depth: &mut i32) {
    result.push_str("  sub $8, %rsp\n");
    result.push_str("  movsd %xmm0, (%rsp)\n");
    *depth += 1;
}

fn popf(result: &mut String, depth: &mut i32, reg: i32) {
    result.push_str(&format!("  movsd (%rsp), %xmm{}\n", reg));
    result.push_str("  add $8, %rsp\n");
    *depth -= 1;
}

fn push_args(
    node: &Node,
    result: &mut String,
    filename: &str,
    src: &str,
    current_fn: &str,
    depth: &mut i32,
) -> Result<(), String> {
    if let Some(next) = node.next.as_ref() {
        push_args(next, result, filename, src, current_fn, depth)?;
    }
    gen_expr(node, result, filename, src, current_fn, depth)?;
    if crate::is_flonum(node.ty.as_ref().unwrap()) {
        pushf(result, depth);
    } else {
        result.push_str("  push %rax\n");
        *depth += 1;
    }
    Ok(())
}

const I8: usize = 0;
const I16: usize = 1;
const I32: usize = 2;
const I64: usize = 3;
const U8: usize = 4;
const U16: usize = 5;
const U32: usize = 6;
const U64: usize = 7;
const F32: usize = 8;
const F64: usize = 9;

fn get_type_id(ty: &Type) -> usize {
    match ty.kind {
        TypeKind::Bool => U8,
        TypeKind::Char => {
            if ty.is_unsigned {
                U8
            } else {
                I8
            }
        }
        TypeKind::Short => {
            if ty.is_unsigned {
                U16
            } else {
                I16
            }
        }
        TypeKind::Int => {
            if ty.is_unsigned {
                U32
            } else {
                I32
            }
        }
        TypeKind::Long => {
            if ty.is_unsigned {
                U64
            } else {
                I64
            }
        }
        TypeKind::Float => F32,
        TypeKind::Double => F64,
        _ => {
            if ty.is_unsigned {
                U64
            } else {
                I64
            }
        }
    }
}

fn cast_type(from: &Type, to: &Type, result: &mut String) {
    if to.kind == TypeKind::Void {
        return;
    }

    if to.kind == TypeKind::Bool {
        match from.kind {
            TypeKind::Float => {
                result.push_str("  xorps %xmm1, %xmm1\n");
                result.push_str("  ucomiss %xmm0, %xmm1\n");
                result.push_str("  setp %al\n");
                result.push_str("  setne %dl\n");
                result.push_str("  or %dl, %al\n");
                result.push_str("  movzb %al, %rax\n");
                return;
            }
            TypeKind::Double => {
                result.push_str("  xorpd %xmm1, %xmm1\n");
                result.push_str("  ucomisd %xmm0, %xmm1\n");
                result.push_str("  setp %al\n");
                result.push_str("  setne %dl\n");
                result.push_str("  or %dl, %al\n");
                result.push_str("  movzb %al, %rax\n");
                return;
            }
            _ => {
                cmp_zero(from, result);
                result.push_str("  setne %al\n");
                result.push_str("  movzb %al, %rax\n");
                return;
            }
        }
    }

    let t1 = get_type_id(from);
    let t2 = get_type_id(to);

    let i32i8: &str = "movsbl %al, %eax";
    let i32u8: &str = "movzbl %al, %eax";
    let i32i16: &str = "movswl %ax, %eax";
    let i32u16: &str = "movzwl %ax, %eax";
    let i32i64: &str = "movsxd %eax, %rax";
    let i32f32: &str = "cvtsi2ssl %eax, %xmm0";
    let i32f64: &str = "cvtsi2sdl %eax, %xmm0";

    let u32i64: &str = "mov %eax, %eax";
    let u32f32: &str = "mov %eax, %eax; cvtsi2ssq %rax, %xmm0";
    let u32f64: &str = "mov %eax, %eax; cvtsi2sdq %rax, %xmm0";

    let i64f32: &str = "cvtsi2ssq %rax, %xmm0";
    let i64f64: &str = "cvtsi2sdq %rax, %xmm0";

    let u64f32: &str = "cvtsi2ssq %rax, %xmm0";
    let u64f64: &str = "test %rax,%rax; js 1f; pxor %xmm0,%xmm0; cvtsi2sd %rax,%xmm0; jmp 2f; 1: \
                        mov %rax,%rdi; and $1,%eax; pxor %xmm0,%xmm0; shr %rdi; or %rax,%rdi; \
                        cvtsi2sd %rdi,%xmm0; addsd %xmm0,%xmm0; 2:";

    let f32i8: &str = "cvttss2sil %xmm0, %eax; movsbl %al, %eax";
    let f32u8: &str = "cvttss2sil %xmm0, %eax; movzbl %al, %eax";
    let f32i16: &str = "cvttss2sil %xmm0, %eax; movswl %ax, %eax";
    let f32u16: &str = "cvttss2sil %xmm0, %eax; movzwl %ax, %eax";
    let f32i32: &str = "cvttss2sil %xmm0, %eax";
    let f32u32: &str = "cvttss2siq %xmm0, %rax";
    let f32i64: &str = "cvttss2siq %xmm0, %rax";
    let f32u64: &str = "cvttss2siq %xmm0, %rax";
    let f32f64: &str = "cvtss2sd %xmm0, %xmm0";

    let f64i8: &str = "cvttsd2sil %xmm0, %eax; movsbl %al, %eax";
    let f64u8: &str = "cvttsd2sil %xmm0, %eax; movzbl %al, %eax";
    let f64i16: &str = "cvttsd2sil %xmm0, %eax; movswl %ax, %eax";
    let f64u16: &str = "cvttsd2sil %xmm0, %eax; movzwl %ax, %eax";
    let f64i32: &str = "cvttsd2sil %xmm0, %eax";
    let f64u32: &str = "cvttsd2siq %xmm0, %rax";
    let f64f32: &str = "cvtsd2ss %xmm0, %xmm0";
    let f64i64: &str = "cvttsd2siq %xmm0, %rax";
    let f64u64: &str = "cvttsd2siq %xmm0, %rax";

    let cast_table: [[Option<&str>; 10]; 10] = [
        [
            None,
            None,
            None,
            Some(i32i64),
            Some(i32u8),
            Some(i32u16),
            None,
            Some(i32i64),
            Some(i32f32),
            Some(i32f64),
        ],
        [
            Some(i32i8),
            None,
            None,
            Some(i32i64),
            Some(i32u8),
            Some(i32u16),
            None,
            Some(i32i64),
            Some(i32f32),
            Some(i32f64),
        ],
        [
            Some(i32i8),
            Some(i32i16),
            None,
            Some(i32i64),
            Some(i32u8),
            Some(i32u16),
            None,
            Some(i32i64),
            Some(i32f32),
            Some(i32f64),
        ],
        [
            Some(i32i8),
            Some(i32i16),
            None,
            None,
            Some(i32u8),
            Some(i32u16),
            None,
            None,
            Some(i64f32),
            Some(i64f64),
        ],
        [
            Some(i32i8),
            None,
            None,
            Some(i32i64),
            None,
            None,
            None,
            Some(i32i64),
            Some(i32f32),
            Some(i32f64),
        ],
        [
            Some(i32i8),
            Some(i32i16),
            None,
            Some(i32i64),
            Some(i32u8),
            None,
            None,
            Some(i32i64),
            Some(i32f32),
            Some(i32f64),
        ],
        [
            Some(i32i8),
            Some(i32i16),
            None,
            Some(u32i64),
            Some(i32u8),
            Some(i32u16),
            None,
            Some(u32i64),
            Some(u32f32),
            Some(u32f64),
        ],
        [
            Some(i32i8),
            Some(i32i16),
            None,
            None,
            Some(i32u8),
            Some(i32u16),
            None,
            None,
            Some(u64f32),
            Some(u64f64),
        ],
        [
            Some(f32i8),
            Some(f32i16),
            Some(f32i32),
            Some(f32i64),
            Some(f32u8),
            Some(f32u16),
            Some(f32u32),
            Some(f32u64),
            None,
            Some(f32f64),
        ],
        [
            Some(f64i8),
            Some(f64i16),
            Some(f64i32),
            Some(f64i64),
            Some(f64u8),
            Some(f64u16),
            Some(f64u32),
            Some(f64u64),
            Some(f64f32),
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
    depth: &mut i32,
) -> Result<(), String> {
    result.push_str(&format!("  .loc 1 {}\n", node.line_no));

    match node.kind {
        NodeKind::NullExpr => {
            return Ok(());
        }
        NodeKind::Num => {
            let ty = node.ty.as_ref().unwrap();
            match ty.kind {
                TypeKind::Float => {
                    let bits = node.fval as f32;
                    let bits_u32 = bits.to_bits();
                    result.push_str(&format!(
                        "  mov ${}, %eax # float {}\n",
                        bits_u32, node.fval
                    ));
                    result.push_str("  movq %rax, %xmm0\n");
                }
                TypeKind::Double => {
                    let bits_u64 = node.fval.to_bits();
                    result.push_str(&format!(
                        "  mov ${}, %rax # double {}\n",
                        bits_u64, node.fval
                    ));
                    result.push_str("  movq %rax, %xmm0\n");
                }
                _ => {
                    result.push_str(&format!("  mov ${}, %rax\n", node.val));
                }
            }
            return Ok(());
        }
        NodeKind::Neg => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;

            let ty = node.ty.as_ref().unwrap();
            match ty.kind {
                TypeKind::Float => {
                    result.push_str("  mov $1, %rax\n");
                    result.push_str("  shl $31, %rax\n");
                    result.push_str("  movq %rax, %xmm1\n");
                    result.push_str("  xorps %xmm1, %xmm0\n");
                    return Ok(());
                }
                TypeKind::Double => {
                    result.push_str("  mov $1, %rax\n");
                    result.push_str("  shl $63, %rax\n");
                    result.push_str("  movq %rax, %xmm1\n");
                    result.push_str("  xorpd %xmm1, %xmm0\n");
                    return Ok(());
                }
                _ => {}
            }

            result.push_str("  neg %rax\n");
            return Ok(());
        }
        NodeKind::Var => {
            gen_addr(node, result, filename, src, current_fn, depth)?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Member => {
            gen_addr(node, result, filename, src, current_fn, depth)?;
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
                depth,
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
                depth,
            )?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Not => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.lhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str("  sete %al\n");
            result.push_str("  movzb %al, %rax\n");
            return Ok(());
        }
        NodeKind::BitNot => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str("  not %rax\n");
            return Ok(());
        }
        NodeKind::LogAnd => {
            let c = count();
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.lhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je .L.false.{}\n", c));
            gen_expr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.rhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je .L.false.{}\n", c));
            result.push_str("  mov $1, %rax\n");
            result.push_str(&format!("  jmp .L.end.{}\n", c));
            result.push_str(&format!(".L.false.{}:\n", c));
            result.push_str("  mov $0, %rax\n");
            result.push_str(&format!(".L.end.{}:\n", c));
            return Ok(());
        }
        NodeKind::LogOr => {
            let c = count();
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.lhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  jne .L.true.{}\n", c));
            gen_expr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.rhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  jne .L.true.{}\n", c));
            result.push_str("  mov $0, %rax\n");
            result.push_str(&format!("  jmp .L.end.{}\n", c));
            result.push_str(&format!(".L.true.{}:\n", c));
            result.push_str("  mov $1, %rax\n");
            result.push_str(&format!(".L.end.{}:\n", c));
            return Ok(());
        }
        NodeKind::Assign => {
            gen_addr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str("  push %rax\n");
            *depth += 1;
            gen_expr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            store(node.ty.as_ref().unwrap(), result, depth);
            return Ok(());
        }
        NodeKind::FuncCall => {
            let argreg = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];

            if let Some(args) = node.args.as_ref() {
                push_args(args, result, filename, src, current_fn, depth)?;
            }

            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;

            let mut gp = 0;
            let mut fp = 0;
            let mut arg = node.args.as_ref();
            while let Some(arg_node) = arg {
                if crate::is_flonum(arg_node.ty.as_ref().unwrap()) {
                    popf(result, depth, fp);
                    fp += 1;
                } else {
                    result.push_str(&format!("  pop {}\n", argreg[gp]));
                    *depth -= 1;
                    gp += 1;
                }
                arg = arg_node.next.as_ref();
            }

            if *depth % 2 == 0 {
                result.push_str("  call *%rax\n");
            } else {
                result.push_str("  sub $8, %rsp\n");
                result.push_str("  call *%rax\n");
                result.push_str("  add $8, %rsp\n");
            }

            let ty = node.ty.as_ref().unwrap();
            match ty.kind {
                TypeKind::Bool => {
                    result.push_str("  movzx %al, %eax\n");
                }
                TypeKind::Char => {
                    if ty.is_unsigned {
                        result.push_str("  movzbl %al, %eax\n");
                    } else {
                        result.push_str("  movsbl %al, %eax\n");
                    }
                }
                TypeKind::Short => {
                    if ty.is_unsigned {
                        result.push_str("  movzwl %ax, %eax\n");
                    } else {
                        result.push_str("  movswl %ax, %eax\n");
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        NodeKind::StmtExpr => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                gen_stmt(stmt_node, result, filename, src, current_fn, depth)?;
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
                depth,
            )?;
            gen_expr(
                node.rhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
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
                depth,
            )?;
            cast_type(
                node.lhs.as_ref().unwrap().ty.as_ref().unwrap(),
                node.ty.as_ref().unwrap(),
                result,
            );
            return Ok(());
        }
        NodeKind::Memzero => {
            let var = node.var.as_ref().unwrap();
            result.push_str(&format!("  mov ${}, %rcx\n", var.ty.size));
            result.push_str(&format!("  lea -{}(%rbp), %rdi\n", var.offset));
            result.push_str("  mov $0, %al\n");
            result.push_str("  rep stosb\n");
            return Ok(());
        }
        NodeKind::Cond => {
            let c = count();
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je .L.else.{}\n", c));
            gen_expr(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("  jmp .L.end.{}\n", c));
            result.push_str(&format!(".L.else.{}:\n", c));
            gen_expr(
                node.els.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str(&format!(".L.end.{}:\n", c));
            return Ok(());
        }
        _ => {}
    }

    let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
    if crate::is_flonum(lhs_ty) {
        gen_expr(
            node.rhs.as_ref().unwrap(),
            result,
            filename,
            src,
            current_fn,
            depth,
        )?;
        pushf(result, depth);
        gen_expr(
            node.lhs.as_ref().unwrap(),
            result,
            filename,
            src,
            current_fn,
            depth,
        )?;
        popf(result, depth, 1);

        let sz = if lhs_ty.kind == TypeKind::Float {
            "ss"
        } else {
            "sd"
        };

        match node.kind {
            NodeKind::Add => {
                result.push_str(&format!("  add{} %xmm1, %xmm0\n", sz));
                return Ok(());
            }
            NodeKind::Sub => {
                result.push_str(&format!("  sub{} %xmm1, %xmm0\n", sz));
                return Ok(());
            }
            NodeKind::Mul => {
                result.push_str(&format!("  mul{} %xmm1, %xmm0\n", sz));
                return Ok(());
            }
            NodeKind::Div => {
                result.push_str(&format!("  div{} %xmm1, %xmm0\n", sz));
                return Ok(());
            }
            NodeKind::Eq | NodeKind::Ne | NodeKind::Lt | NodeKind::Le => {
                result.push_str(&format!("  ucomi{} %xmm0, %xmm1\n", sz));

                match node.kind {
                    NodeKind::Eq => {
                        result.push_str("  sete %al\n");
                        result.push_str("  setnp %dl\n");
                        result.push_str("  and %dl, %al\n");
                    }
                    NodeKind::Ne => {
                        result.push_str("  setne %al\n");
                        result.push_str("  setp %dl\n");
                        result.push_str("  or %dl, %al\n");
                    }
                    NodeKind::Lt => {
                        result.push_str("  seta %al\n");
                    }
                    NodeKind::Le => {
                        result.push_str("  setae %al\n");
                    }
                    _ => unreachable!(),
                }

                result.push_str("  and $1, %al\n");
                result.push_str("  movzb %al, %rax\n");
                return Ok(());
            }
            _ => {
                return Err(error_at(filename, src, node.tok_loc, "invalid expression"));
            }
        }
    }

    gen_expr(
        node.rhs.as_ref().unwrap(),
        result,
        filename,
        src,
        current_fn,
        depth,
    )?;
    result.push_str("  push %rax\n");
    *depth += 1;
    gen_expr(
        node.lhs.as_ref().unwrap(),
        result,
        filename,
        src,
        current_fn,
        depth,
    )?;
    result.push_str("  pop %rdi\n");
    *depth -= 1;

    let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
    let (ax, di, dx) = if lhs_ty.kind == TypeKind::Long || lhs_ty.base.is_some() {
        ("%rax", "%rdi", "%rdx")
    } else {
        ("%eax", "%edi", "%edx")
    };

    match node.kind {
        NodeKind::Add => result.push_str(&format!("  add {}, {}\n", di, ax)),
        NodeKind::Sub => result.push_str(&format!("  sub {}, {}\n", di, ax)),
        NodeKind::Mul => result.push_str(&format!("  imul {}, {}\n", di, ax)),
        NodeKind::Div | NodeKind::Mod => {
            if lhs_ty.is_unsigned {
                result.push_str(&format!("  mov $0, {}\n", dx));
                result.push_str(&format!("  div {}\n", di));
            } else {
                if lhs_ty.size == 8 {
                    result.push_str("  cqo\n");
                } else {
                    result.push_str("  cdq\n");
                }
                result.push_str(&format!("  idiv {}\n", di));
            }
            if node.kind == NodeKind::Mod {
                result.push_str("  mov %rdx, %rax\n");
            }
        }
        NodeKind::BitAnd => result.push_str(&format!("  and {}, {}\n", di, ax)),
        NodeKind::BitOr => result.push_str(&format!("  or {}, {}\n", di, ax)),
        NodeKind::BitXor => result.push_str(&format!("  xor {}, {}\n", di, ax)),
        NodeKind::Shl => {
            result.push_str("  mov %rdi, %rcx\n");
            result.push_str(&format!("  shl %cl, {}\n", ax));
        }
        NodeKind::Shr => {
            result.push_str("  mov %rdi, %rcx\n");
            if lhs_ty.is_unsigned {
                result.push_str(&format!("  shr %cl, {}\n", ax));
            } else {
                result.push_str(&format!("  sar %cl, {}\n", ax));
            }
        }
        NodeKind::Eq | NodeKind::Ne | NodeKind::Lt | NodeKind::Le => {
            result.push_str(&format!("  cmp {}, {}\n", di, ax));
            match node.kind {
                NodeKind::Eq => result.push_str("  sete %al\n"),
                NodeKind::Ne => result.push_str("  setne %al\n"),
                NodeKind::Lt => {
                    if lhs_ty.is_unsigned {
                        result.push_str("  setb %al\n");
                    } else {
                        result.push_str("  setl %al\n");
                    }
                }
                NodeKind::Le => {
                    if lhs_ty.is_unsigned {
                        result.push_str("  setbe %al\n");
                    } else {
                        result.push_str("  setle %al\n");
                    }
                }
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
        | NodeKind::Not
        | NodeKind::BitNot
        | NodeKind::LogAnd
        | NodeKind::LogOr
        | NodeKind::Return
        | NodeKind::Block
        | NodeKind::If
        | NodeKind::For
        | NodeKind::While
        | NodeKind::Do
        | NodeKind::Comma
        | NodeKind::Cast
        | NodeKind::Cond
        | NodeKind::Goto
        | NodeKind::Label
        | NodeKind::Switch
        | NodeKind::Case
        | NodeKind::NullExpr
        | NodeKind::Memzero => unreachable!(),
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
    depth: &mut i32,
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
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je .L.else.{}\n", c));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("  jmp .L.end.{}\n", c));
            result.push_str(&format!(".L.else.{}:\n", c));
            if let Some(els) = node.els.as_ref() {
                gen_stmt(els, result, filename, src, current_fn, depth)?;
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
                depth,
            )?;
            result.push_str(&format!(".L.begin.{}:\n", c));
            if let Some(cond) = node.cond.as_ref() {
                gen_expr(cond, result, filename, src, current_fn, depth)?;
                cmp_zero(cond.ty.as_ref().unwrap(), result);
                result.push_str(&format!("  je {}\n", node.brk_label.as_ref().unwrap()));
            }
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("{}:\n", node.cont_label.as_ref().unwrap()));
            if let Some(inc) = node.inc.as_ref() {
                gen_expr(inc, result, filename, src, current_fn, depth)?;
            }
            result.push_str(&format!("  jmp .L.begin.{}\n", c));
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::While => {
            let c = count();
            result.push_str(&format!("{}:\n", node.cont_label.as_ref().unwrap()));
            result.push_str(&format!(".L.begin.{}:\n", c));
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je {}\n", node.brk_label.as_ref().unwrap()));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("  jmp .L.begin.{}\n", c));
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::Do => {
            let c = count();
            result.push_str(&format!(".L.begin.{}:\n", c));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("{}:\n", node.cont_label.as_ref().unwrap()));
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  jne .L.begin.{}\n", c));
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::Block => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                gen_stmt(stmt_node, result, filename, src, current_fn, depth)?;
                n = stmt_node.next.as_ref();
            }
        }
        NodeKind::Return => {
            if let Some(lhs) = node.lhs.as_ref() {
                gen_expr(lhs, result, filename, src, current_fn, depth)?;
            }
            result.push_str(&format!("  jmp .L.return.{}\n", current_fn));
        }
        NodeKind::ExprStmt => {
            gen_expr(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
        }
        NodeKind::Goto => {
            result.push_str(&format!("  jmp {}\n", node.unique_label.as_ref().unwrap()));
        }
        NodeKind::Label => {
            result.push_str(&format!("{}:\n", node.unique_label.as_ref().unwrap()));
            gen_stmt(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
        }
        NodeKind::Switch => {
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;

            let mut case_node = node.case_next.as_ref();
            while let Some(cn) = case_node {
                let reg = if node.cond.as_ref().unwrap().ty.as_ref().unwrap().size == 8 {
                    "%rax"
                } else {
                    "%eax"
                };
                result.push_str(&format!("  cmp ${}, {}\n", cn.val, reg));
                result.push_str(&format!("  je {}\n", cn.label.as_ref().unwrap()));
                case_node = cn.case_next.as_ref();
            }

            if let Some(default) = node.default_case.as_ref() {
                result.push_str(&format!("  jmp {}\n", default.label.as_ref().unwrap()));
            }

            result.push_str(&format!("  jmp {}\n", node.brk_label.as_ref().unwrap()));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::Case => {
            result.push_str(&format!("{}:\n", node.label.as_ref().unwrap()));
            gen_stmt(
                node.lhs.as_ref().unwrap(),
                result,
                filename,
                src,
                current_fn,
                depth,
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

fn store_fp(r: usize, offset: i64, sz: i64, result: &mut String) {
    match sz {
        4 => result.push_str(&format!("  movss %xmm{}, -{}(%rbp)\n", r, offset)),
        8 => result.push_str(&format!("  movsd %xmm{}, -{}(%rbp)\n", r, offset)),
        _ => unreachable!(),
    }
}

fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
}

fn fix_var_offsets(node: &mut Node, locals: &[Obj]) {
    if let Some(var) = &mut node.var
        && let Some(lv) = locals.iter().find(|l| l.unique_id == var.unique_id)
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
    if let Some(goto_next) = &mut node.goto_next {
        fix_var_offsets(goto_next, locals);
    }
    if let Some(case_next) = &mut node.case_next {
        fix_var_offsets(case_next, locals);
    }
    if let Some(default_case) = &mut node.default_case {
        fix_var_offsets(default_case, locals);
    }
}

pub fn emit_assembly(filename: &str, src: &str) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let tok = tokenize(filename, src)?;
    let mut tok = preprocess(filename, src, tok)?;

    let mut globals: Vec<Obj> = Vec::new();
    let mut tag_scope_stack: Vec<Vec<TagScope>> = vec![Vec::new()];
    let mut scope_stack: Vec<Vec<VarScope>> = vec![Vec::new()];

    while tok.kind != TokenKind::Eof {
        let mut attr = VarAttr::default();
        let (basety, new_tok) = declspec(
            filename,
            src,
            &tok,
            &mut tag_scope_stack,
            &mut scope_stack,
            Some(&mut attr),
        )?;
        tok = new_tok;

        if attr.is_typedef {
            tok = parse_typedef(filename, src, &tok, basety, &mut scope_stack)?;
            continue;
        }

        if is_function(src, &tok, &scope_stack)? {
            let (func, new_tok) = function(
                filename,
                src,
                &tok,
                basety,
                &mut globals,
                &mut tag_scope_stack,
                &mut scope_stack,
                &attr,
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
                &mut scope_stack,
                &attr,
            )?;
        }
    }

    let mut result = String::new();
    result.push_str(&format!(".file 1 \"{}\"\n", filename));

    for var in globals.iter() {
        if var.is_function || !var.is_definition {
            continue;
        }
        if var.is_static {
            result.push_str(&format!("  .local {}\n", var.name));
        } else {
            result.push_str(&format!("  .globl {}\n", var.name));
        }

        if let Some(init_data) = &var.init_data {
            result.push_str("  .data\n");
            result.push_str(&format!("  .align {}\n", var.align));
            result.push_str(&format!("{}:\n", var.name));

            let mut rel = var.rel.clone();
            let mut pos = 0;
            while pos < var.ty.size as usize {
                if let Some(ref r) = rel
                    && r.offset as usize == pos
                {
                    result.push_str(&format!("  .quad {}+{}\n", r.label, r.addend));
                    rel = r.next.clone();
                    pos += 8;
                    continue;
                }
                result.push_str(&format!("  .byte {}\n", init_data[pos]));
                pos += 1;
            }
            continue;
        }

        result.push_str("  .bss\n");
        result.push_str(&format!("  .align {}\n", var.align));
        result.push_str(&format!("{}:\n", var.name));
        result.push_str(&format!("  .zero {}\n", var.ty.size));
    }

    for func in globals.iter_mut() {
        if !func.is_function || !func.is_definition {
            continue;
        }

        let mut offset = 0;
        for var in func.locals.iter_mut() {
            offset += var.ty.size;
            offset = align_to(offset, var.align);
            var.offset = offset;
        }
        let stack_size = align_to(offset, 16);

        let locals = func.locals.clone();
        if let Some(body) = &mut func.body {
            fix_var_offsets(body, &locals);
        }

        result.push_str("  .text\n");
        if func.is_static {
            result.push_str(&format!("  .local {}\n", func.name));
        } else {
            result.push_str(&format!("  .globl {}\n", func.name));
        }
        result.push_str(&format!("{}:\n", func.name));

        result.push_str("  push %rbp\n");
        result.push_str("  mov %rsp, %rbp\n");
        result.push_str(&format!("  sub ${}, %rsp\n", stack_size));

        if let Some(va_area) = &func.va_area {
            let off = va_area.offset;
            let mut gp = 0;
            let mut fp = 0;
            for var in func.params.iter() {
                if crate::is_flonum(&var.ty) {
                    fp += 1;
                } else {
                    gp += 1;
                }
            }

            result.push_str(&format!("  movl ${}, -{}(%rbp)\n", gp * 8, off));
            result.push_str(&format!("  movl ${}, -{}(%rbp)\n", fp * 8 + 48, off - 4));
            // overflow_arg_area at -(off-8)(%rbp) = rbp + 16
            result.push_str("  lea 16(%rbp), %rax\n");
            result.push_str(&format!("  movq %rax, -{}(%rbp)\n", off - 8));
            // reg_save_area at -(off-16)(%rbp) = rbp - (off-24)
            result.push_str(&format!("  lea -{}(%rbp), %rax\n", off - 24));
            result.push_str(&format!("  movq %rax, -{}(%rbp)\n", off - 16));

            // Save GP registers at va_area + 24 onwards
            result.push_str(&format!("  movq %rdi, -{}(%rbp)\n", off - 24));
            result.push_str(&format!("  movq %rsi, -{}(%rbp)\n", off - 32));
            result.push_str(&format!("  movq %rdx, -{}(%rbp)\n", off - 40));
            result.push_str(&format!("  movq %rcx, -{}(%rbp)\n", off - 48));
            result.push_str(&format!("  movq %r8, -{}(%rbp)\n", off - 56));
            result.push_str(&format!("  movq %r9, -{}(%rbp)\n", off - 64));
            // Save FP registers
            result.push_str(&format!("  movsd %xmm0, -{}(%rbp)\n", off - 72));
            result.push_str(&format!("  movsd %xmm1, -{}(%rbp)\n", off - 80));
            result.push_str(&format!("  movsd %xmm2, -{}(%rbp)\n", off - 88));
            result.push_str(&format!("  movsd %xmm3, -{}(%rbp)\n", off - 96));
            result.push_str(&format!("  movsd %xmm4, -{}(%rbp)\n", off - 104));
            result.push_str(&format!("  movsd %xmm5, -{}(%rbp)\n", off - 112));
            result.push_str(&format!("  movsd %xmm6, -{}(%rbp)\n", off - 120));
            result.push_str(&format!("  movsd %xmm7, -{}(%rbp)\n", off - 128));
        }

        let mut gp = 0;
        let mut fp = 0;
        for var in func.params.iter_mut() {
            let local_var = func.locals.iter().find(|l| l.name == var.name);
            if let Some(lv) = local_var {
                var.offset = lv.offset;
            }
            if crate::is_flonum(&var.ty) {
                store_fp(fp, var.offset, var.ty.size, &mut result);
                fp += 1;
            } else {
                store_gp(gp, var.offset, var.ty.size, &mut result);
                gp += 1;
            }
        }

        let mut depth: i32 = 0;
        gen_stmt(
            func.body.as_ref().unwrap(),
            &mut result,
            filename,
            src,
            &func.name,
            &mut depth,
        )?;
        assert!(depth == 0, "depth should be 0 after function body");

        result.push_str(&format!(".L.return.{}:\n", func.name));
        result.push_str("  mov %rbp, %rsp\n");
        result.push_str("  pop %rbp\n");
        result.push_str("  ret\n");
    }

    Ok(result)
}

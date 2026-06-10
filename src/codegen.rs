use crate::File;
use crate::parse::{declare_builtin_functions, mark_live_globals, scan_globals};
use crate::preprocess::{preprocess, reset_counter};
use crate::tokenize_input;
use crate::{
    Node, NodeKind, Obj, TagScope, TokenKind, Type, TypeKind, VarAttr, VarScope, error_at,
};
use crate::{declspec, function, global_variable, is_function, new_unique_name, parse_typedef};
use crate::{get_input_files, get_opt_fcommon};

fn epilogue_lbl(fn_name: &str) -> String {
    format!(".L.{}.ret_{}", std::process::id(), fn_name)
}

fn builtin_alloca(current_fn: &Obj, result: &mut String) {
    let off = current_fn.alloca_bottom.as_ref().unwrap().offset;
    result.push_str("  add $15, %rdi\n");
    result.push_str("  and $0xfffffff0, %edi\n");
    result.push_str(&format!("  mov {off}(%rbp), %rcx\n"));
    result.push_str("  sub %rsp, %rcx\n");
    result.push_str("  mov %rsp, %rax\n");
    result.push_str("  sub %rdi, %rsp\n");
    result.push_str("  mov %rsp, %rdx\n");
    result.push_str("1:\n");
    result.push_str("  cmp $0, %rcx\n");
    result.push_str("  je 2f\n");
    result.push_str("  mov (%rax), %r8b\n");
    result.push_str("  mov %r8b, (%rdx)\n");
    result.push_str("  inc %rdx\n");
    result.push_str("  inc %rax\n");
    result.push_str("  dec %rcx\n");
    result.push_str("  jmp 1b\n");
    result.push_str("2:\n");
    result.push_str(&format!("  mov {off}(%rbp), %rax\n"));
    result.push_str("  sub %rdi, %rax\n");
    result.push_str(&format!("  mov %rax, {off}(%rbp)\n"));
}

fn gen_addr(
    node: &Node,
    result: &mut String,
    files: &[File],
    current_fn: &Obj,
    depth: &mut i32,
) -> Result<(), String> {
    match node.kind {
        NodeKind::Var => {
            let var = node.var.as_ref().unwrap();
            // VLA names hold the alloca'd pointer in their stack slot.
            if var.ty.kind == TypeKind::Vla {
                result.push_str(&format!("  mov {}(%rbp), %rax\n", var.offset));
                return Ok(());
            }
            if var.is_local {
                result.push_str(&format!("  lea {}(%rbp), %rax\n", var.offset));
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

            if var.is_tls {
                result.push_str("  mov %fs:0, %rax\n");
                result.push_str(&format!("  add ${}@tpoff, %rax\n", var.name));
                return Ok(());
            }

            result.push_str(&format!("  lea {}(%rip), %rax\n", var.name));
        }
        NodeKind::Deref => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
        }
        NodeKind::Member => {
            gen_addr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            let offset = node.member.as_ref().unwrap().offset;
            result.push_str(&format!("  add ${}, %rax\n", offset));
        }
        NodeKind::Comma => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            gen_addr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;
        }
        NodeKind::FuncCall => {
            if node.ret_buffer.is_some() {
                gen_expr(node, result, files, current_fn, depth)?;
                return Ok(());
            }
            return Err(error_at(files, node.file_no, node.tok_loc, "not an lvalue"));
        }
        NodeKind::VlaPtr => {
            // Assignment LHS: take the address of the pointer slot, not its value.
            let var = node.var.as_ref().unwrap();
            result.push_str(&format!("  lea {}(%rbp), %rax\n", var.offset));
            return Ok(());
        }
        _ => return Err(error_at(files, node.file_no, node.tok_loc, "not an lvalue")),
    }
    Ok(())
}

fn load(ty: &Type, result: &mut String) {
    match ty.kind {
        TypeKind::Array | TypeKind::Struct | TypeKind::Union | TypeKind::Func | TypeKind::Vla => {
            return;
        }
        TypeKind::Float => {
            result.push_str("  movss (%rax), %xmm0\n");
            return;
        }
        TypeKind::Double => {
            result.push_str("  movsd (%rax), %xmm0\n");
            return;
        }
        TypeKind::LDouble => {
            result.push_str("  fldt (%rax)\n");
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
        TypeKind::LDouble => {
            result.push_str("  fstpt (%rdi)\n");
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
        TypeKind::LDouble => {
            result.push_str("  fldz\n");
            result.push_str("  fucomip\n");
            result.push_str("  fstp %st(0)\n");
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

const GP_MAX: i32 = 6;
const FP_MAX: i32 = 8;

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

fn has_flonum(ty: &Type, lo: i64, hi: i64, offset: i64) -> bool {
    match ty.kind {
        TypeKind::Struct | TypeKind::Union => {
            let mut mem = ty.members.as_ref();
            while let Some(m) = mem {
                if !has_flonum(&m.ty, lo, hi, offset + m.offset) {
                    return false;
                }
                mem = m.next.as_ref();
            }
            true
        }
        TypeKind::Array => {
            let base_ty = ty.base.as_ref().unwrap().borrow().clone();
            for i in 0..ty.array_len {
                if !has_flonum(&base_ty, lo, hi, offset + base_ty.size * i) {
                    return false;
                }
            }
            true
        }
        _ => offset < lo || hi <= offset || crate::is_sse_flonum(ty),
    }
}

fn has_flonum1(ty: &Type) -> bool {
    has_flonum(ty, 0, 8, 0)
}

fn has_flonum2(ty: &Type) -> bool {
    has_flonum(ty, 8, 16, 0)
}

fn push_struct(ty: &Type, result: &mut String, depth: &mut i32) {
    let sz = {
        let mut s = ty.size;
        s = (s + 7) / 8 * 8;
        s
    };
    result.push_str(&format!("  sub ${}, %rsp\n", sz));
    *depth += (sz / 8) as i32;

    for i in 0..ty.size {
        result.push_str(&format!("  mov {}(%rax), %r10b\n", i));
        result.push_str(&format!("  mov %r10b, {}(%rsp)\n", i));
    }
}

fn count_args(
    node: &Node,
    ret_buffer: Option<&Obj>,
    ret_ty_size: Option<i64>,
) -> (i32, i32, Vec<bool>, i32) {
    let mut gp: i32 = 0;
    let mut fp: i32 = 0;
    let mut stack: i32 = 0;
    let mut pass_by_stack = Vec::new();

    if ret_buffer.is_some() && ret_ty_size.unwrap_or(0) > 16 {
        gp += 1;
    }

    let mut arg: Option<&Node> = Some(node);
    while let Some(arg_node) = arg {
        let ty = arg_node.ty.as_ref().unwrap();
        match ty.kind {
            TypeKind::Struct | TypeKind::Union => {
                if ty.size > 16 {
                    pass_by_stack.push(true);
                    let sz = ((ty.size + 7) / 8) as i32;
                    stack += sz;
                } else {
                    let fp1 = has_flonum1(ty);
                    let fp2 = if ty.size > 8 { has_flonum2(ty) } else { false };
                    if (fp + fp1 as i32 + fp2 as i32) < FP_MAX
                        && (gp + !fp1 as i32 + !fp2 as i32) < GP_MAX
                    {
                        fp += fp1 as i32 + fp2 as i32;
                        gp += !fp1 as i32 + !fp2 as i32;
                        pass_by_stack.push(false);
                    } else {
                        pass_by_stack.push(true);
                        let sz = ((ty.size + 7) / 8) as i32;
                        stack += sz;
                    }
                }
            }
            TypeKind::Float | TypeKind::Double => {
                if fp >= FP_MAX {
                    pass_by_stack.push(true);
                    stack += 1;
                } else {
                    pass_by_stack.push(false);
                }
                fp += 1;
            }
            TypeKind::LDouble => {
                pass_by_stack.push(true);
                stack += 2;
            }
            _ => {
                if gp >= GP_MAX {
                    pass_by_stack.push(true);
                    stack += 1;
                } else {
                    pass_by_stack.push(false);
                }
                gp += 1;
            }
        }
        arg = arg_node.next.as_deref();
    }
    (gp, fp, pass_by_stack, stack)
}

#[allow(clippy::too_many_arguments)]
fn push_args2(
    node: &Node,
    first_pass: bool,
    pass_by_stack: &[bool],
    pos: usize,
    result: &mut String,
    files: &[File],
    current_fn: &Obj,
    depth: &mut i32,
) -> Result<(), String> {
    if let Some(next) = node.next.as_ref() {
        push_args2(
            next,
            first_pass,
            pass_by_stack,
            pos + 1,
            result,
            files,
            current_fn,
            depth,
        )?;
    }

    let is_stack = pass_by_stack[pos];

    if (first_pass && !is_stack) || (!first_pass && is_stack) {
        return Ok(());
    }

    gen_expr(node, result, files, current_fn, depth)?;
    match node.ty.as_ref().unwrap().kind {
        TypeKind::Struct | TypeKind::Union => {
            push_struct(node.ty.as_ref().unwrap(), result, depth);
        }
        TypeKind::Float | TypeKind::Double => {
            pushf(result, depth);
        }
        TypeKind::LDouble => {
            result.push_str("  sub $16, %rsp\n");
            result.push_str("  fstpt (%rsp)\n");
            *depth += 2;
        }
        _ => {
            result.push_str("  push %rax\n");
            *depth += 1;
        }
    }
    Ok(())
}

fn push_args(
    node: &Node,
    result: &mut String,
    files: &[File],
    current_fn: &Obj,
    depth: &mut i32,
    ret_buffer: Option<&Obj>,
    ret_ty_size: Option<i64>,
) -> Result<(i32, i32), String> {
    let (_, fp, pass_by_stack, stack) = count_args(node, ret_buffer, ret_ty_size);

    let mut pad = 0;
    if (*depth + stack) % 2 == 1 {
        result.push_str("  sub $8, %rsp\n");
        *depth += 1;
        pad = 1;
    }

    push_args2(
        node,
        true,
        &pass_by_stack,
        0,
        result,
        files,
        current_fn,
        depth,
    )?;
    push_args2(
        node,
        false,
        &pass_by_stack,
        0,
        result,
        files,
        current_fn,
        depth,
    )?;

    if let Some(var) = ret_buffer
        && ret_ty_size.unwrap_or(0) > 16
    {
        result.push_str(&format!("  lea {}(%rbp), %rax\n", var.offset));
        result.push_str("  push %rax\n");
        *depth += 1;
    }

    Ok((stack + pad, fp))
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
const F80: usize = 10;

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
        TypeKind::LDouble => F80,
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
            TypeKind::LDouble => {
                cmp_zero(from, result);
                result.push_str("  setne %al\n");
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
    let i32f80: &str = "mov %eax, -4(%rsp); fildl -4(%rsp)";

    let u32i64: &str = "mov %eax, %eax";
    let u32f32: &str = "mov %eax, %eax; cvtsi2ssq %rax, %xmm0";
    let u32f64: &str = "mov %eax, %eax; cvtsi2sdq %rax, %xmm0";
    let u32f80: &str = "mov %eax, %eax; mov %rax, -8(%rsp); fildll -8(%rsp)";

    let i64f32: &str = "cvtsi2ssq %rax, %xmm0";
    let i64f64: &str = "cvtsi2sdq %rax, %xmm0";
    let i64f80: &str = "movq %rax, -8(%rsp); fildll -8(%rsp)";

    let u64f32: &str = "cvtsi2ssq %rax, %xmm0";
    let u64f64: &str = "test %rax,%rax; js 1f; pxor %xmm0,%xmm0; cvtsi2sd %rax,%xmm0; jmp 2f; 1: \
                        mov %rax,%rdi; and $1,%eax; pxor %xmm0,%xmm0; shr %rdi; or %rax,%rdi; \
                        cvtsi2sd %rdi,%xmm0; addsd %xmm0,%xmm0; 2:";
    let u64f80: &str = "mov %rax, -8(%rsp); fildq -8(%rsp); test %rax, %rax; jns 1f; mov \
                        $1602224128, %eax; mov %eax, -4(%rsp); fadds -4(%rsp); 1:";

    let f32i8: &str = "cvttss2sil %xmm0, %eax; movsbl %al, %eax";
    let f32u8: &str = "cvttss2sil %xmm0, %eax; movzbl %al, %eax";
    let f32i16: &str = "cvttss2sil %xmm0, %eax; movswl %ax, %eax";
    let f32u16: &str = "cvttss2sil %xmm0, %eax; movzwl %ax, %eax";
    let f32i32: &str = "cvttss2sil %xmm0, %eax";
    let f32u32: &str = "cvttss2siq %xmm0, %rax";
    let f32i64: &str = "cvttss2siq %xmm0, %rax";
    let f32u64: &str = "cvttss2siq %xmm0, %rax";
    let f32f64: &str = "cvtss2sd %xmm0, %xmm0";
    let f32f80: &str = "movss %xmm0, -4(%rsp); flds -4(%rsp)";

    let f64i8: &str = "cvttsd2sil %xmm0, %eax; movsbl %al, %eax";
    let f64u8: &str = "cvttsd2sil %xmm0, %eax; movzbl %al, %eax";
    let f64i16: &str = "cvttsd2sil %xmm0, %eax; movswl %ax, %eax";
    let f64u16: &str = "cvttsd2sil %xmm0, %eax; movzwl %ax, %eax";
    let f64i32: &str = "cvttsd2sil %xmm0, %eax";
    let f64u32: &str = "cvttsd2siq %xmm0, %rax";
    let f64f32: &str = "cvtsd2ss %xmm0, %xmm0";
    let f64f80: &str = "movsd %xmm0, -8(%rsp); fldl -8(%rsp)";
    let f64i64: &str = "cvttsd2siq %xmm0, %rax";
    let f64u64: &str = "cvttsd2siq %xmm0, %rax";

    let f80i8: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, -12(%rsp); \
                       fldcw -12(%rsp); fistps -24(%rsp); fldcw -10(%rsp); movsbl -24(%rsp), %eax";
    let f80u8: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, -12(%rsp); \
                       fldcw -12(%rsp); fistps -24(%rsp); fldcw -10(%rsp); movzbl -24(%rsp), %eax";
    let f80i16: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, \
                        -12(%rsp); fldcw -12(%rsp); fistps -24(%rsp); fldcw -10(%rsp); movswl \
                        -24(%rsp), %eax";
    let f80u16: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, \
                        -12(%rsp); fldcw -12(%rsp); fistpl -24(%rsp); fldcw -10(%rsp); movswl \
                        -24(%rsp), %eax";
    let f80i32: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, \
                        -12(%rsp); fldcw -12(%rsp); fistpl -24(%rsp); fldcw -10(%rsp); mov \
                        -24(%rsp), %eax";
    let f80u32: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, \
                        -12(%rsp); fldcw -12(%rsp); fistpl -24(%rsp); fldcw -10(%rsp); mov \
                        -24(%rsp), %eax";
    let f80i64: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, \
                        -12(%rsp); fldcw -12(%rsp); fistpq -24(%rsp); fldcw -10(%rsp); mov \
                        -24(%rsp), %rax";
    let f80u64: &str = "fnstcw -10(%rsp); movzwl -10(%rsp), %eax; or $12, %ah; mov %ax, \
                        -12(%rsp); fldcw -12(%rsp); fistpq -24(%rsp); fldcw -10(%rsp); mov \
                        -24(%rsp), %rax";
    let f80f32: &str = "fstps -8(%rsp); movss -8(%rsp), %xmm0";
    let f80f64: &str = "fstpl -8(%rsp); movsd -8(%rsp), %xmm0";

    let cast_table: [[Option<&str>; 11]; 11] = [
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
            Some(i32f80),
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
            Some(i32f80),
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
            Some(i32f80),
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
            Some(i64f80),
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
            Some(i32f80),
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
            Some(i32f80),
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
            Some(u32f80),
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
            Some(u64f80),
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
            Some(f32f80),
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
            Some(f64f80),
        ],
        [
            Some(f80i8),
            Some(f80i16),
            Some(f80i32),
            Some(f80i64),
            Some(f80u8),
            Some(f80u16),
            Some(f80u32),
            Some(f80u64),
            Some(f80f32),
            Some(f80f64),
            None,
        ],
    ];

    if let Some(inst) = cast_table[t1][t2] {
        result.push_str(&format!("  {}\n", inst));
    }
}

fn copy_ret_buffer(var: &Obj, result: &mut String) {
    let ty = &var.ty;
    let mut gp: i32 = 0;
    let mut fp: i32 = 0;

    if has_flonum1(ty) {
        assert!(ty.size == 4 || ty.size >= 8);
        if ty.size == 4 {
            result.push_str(&format!("  movss %xmm0, {}(%rbp)\n", var.offset));
        } else {
            result.push_str(&format!("  movsd %xmm0, {}(%rbp)\n", var.offset));
        }
        fp += 1;
    } else {
        for i in 0..std::cmp::min(8, ty.size as usize) {
            result.push_str(&format!("  mov %al, {}(%rbp)\n", var.offset + i as i64));
            result.push_str("  shr $8, %rax\n");
        }
        gp += 1;
    }

    if ty.size > 8 {
        if has_flonum2(ty) {
            assert!(ty.size == 12 || ty.size == 16);
            if ty.size == 12 {
                result.push_str(&format!("  movss %xmm{}, {}(%rbp)\n", fp, var.offset + 8));
            } else {
                result.push_str(&format!("  movsd %xmm{}, {}(%rbp)\n", fp, var.offset + 8));
            }
        } else {
            let reg1 = if gp == 0 { "%al" } else { "%dl" };
            let reg2 = if gp == 0 { "%rax" } else { "%rdx" };
            for i in 8..std::cmp::min(16, ty.size as usize) {
                result.push_str(&format!(
                    "  mov {}, {}(%rbp)\n",
                    reg1,
                    var.offset + i as i64
                ));
                result.push_str(&format!("  shr $8, {}\n", reg2));
            }
        }
    }
}

fn copy_struct_reg(ty: &Type, result: &mut String) {
    let mut gp: i32 = 0;
    let mut fp: i32 = 0;

    result.push_str("  mov %rax, %rdi\n");

    if has_flonum1(ty) {
        assert!(ty.size == 4 || ty.size >= 8);
        if ty.size == 4 {
            result.push_str("  movss (%rdi), %xmm0\n");
        } else {
            result.push_str("  movsd (%rdi), %xmm0\n");
        }
        fp += 1;
    } else {
        result.push_str("  mov $0, %rax\n");
        for i in (0..std::cmp::min(8, ty.size as usize)).rev() {
            result.push_str("  shl $8, %rax\n");
            result.push_str(&format!("  mov {}(%rdi), %al\n", i));
        }
        gp += 1;
    }

    if ty.size > 8 {
        if has_flonum2(ty) {
            assert!(ty.size == 12 || ty.size == 16);
            if ty.size == 12 {
                result.push_str(&format!("  movss 8(%rdi), %xmm{}\n", fp));
            } else {
                result.push_str(&format!("  movsd 8(%rdi), %xmm{}\n", fp));
            }
        } else {
            let reg1 = if gp == 0 { "%al" } else { "%dl" };
            let reg2 = if gp == 0 { "%rax" } else { "%rdx" };
            result.push_str(&format!("  mov $0, {}\n", reg2));
            for i in (8..std::cmp::min(16, ty.size as usize)).rev() {
                result.push_str(&format!("  shl $8, {}\n", reg2));
                result.push_str(&format!("  mov {}(%rdi), {}\n", i, reg1));
            }
        }
    }
}

fn copy_struct_mem(ty: &Type, param_offset: i64, result: &mut String) {
    result.push_str(&format!("  mov {}(%rbp), %rdi\n", param_offset));

    for i in 0..ty.size {
        result.push_str(&format!("  mov {}(%rax), %dl\n", i));
        result.push_str(&format!("  mov %dl, {}(%rdi)\n", i));
    }
}

fn gen_expr(
    node: &Node,
    result: &mut String,
    files: &[File],
    current_fn: &Obj,
    depth: &mut i32,
) -> Result<(), String> {
    result.push_str(&format!("  .loc {} {}\n", node.file_no, node.line_no));

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
                TypeKind::LDouble => {
                    let bytes = crate::f64_to_x87_16(node.fval);
                    let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
                    let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                    result.push_str(&format!(
                        "  mov ${}, %rax # long double {}\n",
                        lo, node.fval
                    ));
                    result.push_str("  mov %rax, -16(%rsp)\n");
                    result.push_str(&format!("  mov ${}, %rax\n", hi));
                    result.push_str("  mov %rax, -8(%rsp)\n");
                    result.push_str("  fldt -16(%rsp)\n");
                }
                _ => {
                    result.push_str(&format!("  mov ${}, %rax\n", node.val));
                }
            }
            return Ok(());
        }
        NodeKind::Neg => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;

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
                TypeKind::LDouble => {
                    result.push_str("  fchs\n");
                    return Ok(());
                }
                _ => {}
            }

            result.push_str("  neg %rax\n");
            return Ok(());
        }
        NodeKind::Var | NodeKind::VlaPtr => {
            gen_addr(node, result, files, current_fn, depth)?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Member => {
            gen_addr(node, result, files, current_fn, depth)?;
            load(node.ty.as_ref().unwrap(), result);
            let mem = node.member.as_ref().unwrap();
            if mem.is_bitfield {
                result.push_str(&format!(
                    "  shl ${}, %rax\n",
                    64 - mem.bit_width - mem.bit_offset
                ));
                if mem.ty.is_unsigned {
                    result.push_str(&format!("  shr ${}, %rax\n", 64 - mem.bit_width));
                } else {
                    result.push_str(&format!("  sar ${}, %rax\n", 64 - mem.bit_width));
                }
            }
            return Ok(());
        }
        NodeKind::Addr => {
            gen_addr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            return Ok(());
        }
        NodeKind::Deref => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            load(node.ty.as_ref().unwrap(), result);
            return Ok(());
        }
        NodeKind::Not => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            cmp_zero(node.lhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str("  sete %al\n");
            result.push_str("  movzb %al, %rax\n");
            return Ok(());
        }
        NodeKind::BitNot => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            result.push_str("  not %rax\n");
            return Ok(());
        }
        NodeKind::LogAnd => {
            let l_false = new_unique_name();
            let l_end = new_unique_name();
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            cmp_zero(node.lhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je {}\n", l_false));
            gen_expr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            cmp_zero(node.rhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je {}\n", l_false));
            result.push_str("  mov $1, %rax\n");
            result.push_str(&format!("  jmp {}\n", l_end));
            result.push_str(&format!("{}:\n", l_false));
            result.push_str("  mov $0, %rax\n");
            result.push_str(&format!("{}:\n", l_end));
            return Ok(());
        }
        NodeKind::LogOr => {
            let l_true = new_unique_name();
            let l_end = new_unique_name();
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            cmp_zero(node.lhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  jne {}\n", l_true));
            gen_expr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            cmp_zero(node.rhs.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  jne {}\n", l_true));
            result.push_str("  mov $0, %rax\n");
            result.push_str(&format!("  jmp {}\n", l_end));
            result.push_str(&format!("{}:\n", l_true));
            result.push_str("  mov $1, %rax\n");
            result.push_str(&format!("{}:\n", l_end));
            return Ok(());
        }
        NodeKind::Assign => {
            gen_addr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            result.push_str("  push %rax\n");
            *depth += 1;
            gen_expr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;

            if node.lhs.as_ref().unwrap().kind == NodeKind::Member
                && node
                    .lhs
                    .as_ref()
                    .unwrap()
                    .member
                    .as_ref()
                    .unwrap()
                    .is_bitfield
            {
                let mem = node.lhs.as_ref().unwrap().member.as_ref().unwrap();
                let field_mask: u64 = ((1u64 << mem.bit_width as u32) - 1) << mem.bit_offset as u32;
                let clear_mask = !field_mask;

                result.push_str("  mov %rax, %r8\n");
                result.push_str("  mov %rax, %rdi\n");
                result.push_str(&format!("  and ${}, %rdi\n", (1_i64 << mem.bit_width) - 1));
                result.push_str(&format!("  shl ${}, %rdi\n", mem.bit_offset));
                result.push_str("  mov (%rsp), %rax\n");
                load(&mem.ty, result);
                result.push_str(&format!("  movabs $0x{:x}, %r9\n", clear_mask));
                result.push_str("  and %r9, %rax\n");
                result.push_str("  or %rdi, %rax\n");
                store(node.ty.as_ref().unwrap(), result, depth);
                result.push_str("  mov %r8, %rax\n");
                return Ok(());
            }

            store(node.ty.as_ref().unwrap(), result, depth);
            return Ok(());
        }
        NodeKind::FuncCall => {
            if node.lhs.as_ref().is_some_and(|lhs| {
                lhs.kind == NodeKind::Var
                    && lhs.var.as_ref().is_some_and(|var| var.name == "alloca")
            }) {
                gen_expr(
                    node.args.as_ref().unwrap(),
                    result,
                    files,
                    current_fn,
                    depth,
                )?;
                result.push_str("  mov %rax, %rdi\n");
                builtin_alloca(current_fn, result);
                return Ok(());
            }

            let argreg64 = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];

            let ret_buffer = node.ret_buffer.as_ref().map(|b| b.as_ref());
            let ret_ty_size = node.ty.as_ref().map(|t| t.size);

            let (stack_args, fp) = if let Some(args) = node.args.as_ref() {
                push_args(
                    args,
                    result,
                    files,
                    current_fn,
                    depth,
                    ret_buffer,
                    ret_ty_size,
                )?
            } else {
                let mut pad = 0;
                if *depth % 2 == 1 {
                    result.push_str("  sub $8, %rsp\n");
                    *depth += 1;
                    pad = 1;
                }
                if let Some(var) = ret_buffer
                    && ret_ty_size.unwrap_or(0) > 16
                {
                    result.push_str(&format!("  lea {}(%rbp), %rax\n", var.offset));
                    result.push_str("  push %rax\n");
                    *depth += 1;
                }
                (pad, 0)
            };

            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;

            let mut gp = 0;
            let mut fp_count = 0;

            if ret_buffer.is_some() && node.ty.as_ref().unwrap().size > 16 {
                result.push_str(&format!("  pop {}\n", argreg64[gp as usize]));
                *depth -= 1;
                gp += 1;
            }

            let mut arg = node.args.as_ref();
            while let Some(arg_node) = arg {
                let ty = arg_node.ty.as_ref().unwrap();
                match ty.kind {
                    TypeKind::Struct | TypeKind::Union => {
                        if ty.size > 16 {
                            arg = arg_node.next.as_ref();
                            continue;
                        }
                        let fp1 = has_flonum1(ty);
                        let fp2 = has_flonum2(ty);
                        if (fp_count + fp1 as i32 + fp2 as i32) < FP_MAX
                            && (gp + !fp1 as i32 + !fp2 as i32) < GP_MAX
                        {
                            if fp1 {
                                popf(result, depth, fp_count);
                                fp_count += 1;
                            } else {
                                result.push_str(&format!("  pop {}\n", argreg64[gp as usize]));
                                *depth -= 1;
                                gp += 1;
                            }
                            if ty.size > 8 {
                                if fp2 {
                                    popf(result, depth, fp_count);
                                    fp_count += 1;
                                } else {
                                    result.push_str(&format!("  pop {}\n", argreg64[gp as usize]));
                                    *depth -= 1;
                                    gp += 1;
                                }
                            }
                        }
                    }
                    TypeKind::Float | TypeKind::Double => {
                        if fp_count < FP_MAX {
                            popf(result, depth, fp_count);
                            fp_count += 1;
                        }
                    }
                    TypeKind::LDouble => {}
                    _ => {
                        if gp < GP_MAX {
                            result.push_str(&format!("  pop {}\n", argreg64[gp as usize]));
                            *depth -= 1;
                            gp += 1;
                        }
                    }
                }
                arg = arg_node.next.as_ref();
            }

            result.push_str("  mov %rax, %r10\n");
            result.push_str(&format!("  mov ${}, %rax\n", fp));
            result.push_str("  call *%r10\n");
            result.push_str(&format!("  add ${}, %rsp\n", stack_args * 8));
            *depth -= stack_args;

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

            if let Some(var) = ret_buffer {
                if node.ty.as_ref().unwrap().size <= 16 {
                    copy_ret_buffer(var, result);
                }
                result.push_str(&format!("  lea {}(%rbp), %rax\n", var.offset));
            }

            return Ok(());
        }
        NodeKind::StmtExpr => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                gen_stmt(stmt_node, result, files, current_fn, depth)?;
                n = stmt_node.next.as_ref();
            }
            return Ok(());
        }
        NodeKind::Asm => {
            return Err(error_at(
                files,
                node.file_no,
                node.tok_loc,
                "invalid expression",
            ));
        }
        NodeKind::Comma => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            gen_expr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            return Ok(());
        }
        NodeKind::Cast => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
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
            result.push_str(&format!("  lea {}(%rbp), %rdi\n", var.offset));
            result.push_str("  mov $0, %al\n");
            result.push_str("  rep stosb\n");
            return Ok(());
        }
        NodeKind::Cond => {
            let l_else = new_unique_name();
            let l_end = new_unique_name();
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je {}\n", l_else));
            gen_expr(
                node.then.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("  jmp {}\n", l_end));
            result.push_str(&format!("{}:\n", l_else));
            gen_expr(node.els.as_ref().unwrap(), result, files, current_fn, depth)?;
            result.push_str(&format!("{}:\n", l_end));
            return Ok(());
        }
        NodeKind::LabelVal => {
            result.push_str(&format!(
                "  lea {}(%rip), %rax\n",
                node.unique_label.as_ref().unwrap()
            ));
            return Ok(());
        }
        _ => {}
    }

    let lhs_ty = node.lhs.as_ref().unwrap().ty.as_ref().unwrap();
    match lhs_ty.kind {
        TypeKind::Float | TypeKind::Double => {
            gen_expr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            pushf(result, depth);
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
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
                    return Err(error_at(
                        files,
                        node.file_no,
                        node.tok_loc,
                        "invalid expression",
                    ));
                }
            }
        }
        TypeKind::LDouble => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            gen_expr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;

            match node.kind {
                NodeKind::Add => {
                    result.push_str("  faddp\n");
                    return Ok(());
                }
                NodeKind::Sub => {
                    result.push_str("  fsubrp\n");
                    return Ok(());
                }
                NodeKind::Mul => {
                    result.push_str("  fmulp\n");
                    return Ok(());
                }
                NodeKind::Div => {
                    result.push_str("  fdivrp\n");
                    return Ok(());
                }
                NodeKind::Eq | NodeKind::Ne | NodeKind::Lt | NodeKind::Le => {
                    result.push_str("  fcomip\n");
                    result.push_str("  fstp %st(0)\n");
                    match node.kind {
                        NodeKind::Eq => result.push_str("  sete %al\n"),
                        NodeKind::Ne => result.push_str("  setne %al\n"),
                        NodeKind::Lt => result.push_str("  seta %al\n"),
                        NodeKind::Le => result.push_str("  setae %al\n"),
                        _ => unreachable!(),
                    }
                    result.push_str("  movzb %al, %rax\n");
                    return Ok(());
                }
                _ => {
                    return Err(error_at(
                        files,
                        node.file_no,
                        node.tok_loc,
                        "invalid expression",
                    ));
                }
            }
        }
        _ => {}
    }

    gen_expr(node.rhs.as_ref().unwrap(), result, files, current_fn, depth)?;
    result.push_str("  push %rax\n");
    *depth += 1;
    gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
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
        | NodeKind::VlaPtr
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
        | NodeKind::GotoExpr
        | NodeKind::Label
        | NodeKind::LabelVal
        | NodeKind::Switch
        | NodeKind::Case
        | NodeKind::NullExpr
        | NodeKind::Memzero
        | NodeKind::Asm => unreachable!(),
    }
    Ok(())
}

fn gen_stmt(
    node: &Node,
    result: &mut String,
    files: &[File],
    current_fn: &Obj,
    depth: &mut i32,
) -> Result<(), String> {
    result.push_str(&format!("  .loc {} {}\n", node.file_no, node.line_no));

    match node.kind {
        NodeKind::If => {
            let l_else = new_unique_name();
            let l_end = new_unique_name();
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je {}\n", l_else));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("  jmp {}\n", l_end));
            result.push_str(&format!("{}:\n", l_else));
            if let Some(els) = node.els.as_ref() {
                gen_stmt(els, result, files, current_fn, depth)?;
            }
            result.push_str(&format!("{}:\n", l_end));
        }
        NodeKind::For => {
            let l_begin = new_unique_name();
            gen_stmt(
                node.init.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("{}:\n", l_begin));
            if let Some(cond) = node.cond.as_ref() {
                gen_expr(cond, result, files, current_fn, depth)?;
                cmp_zero(cond.ty.as_ref().unwrap(), result);
                result.push_str(&format!("  je {}\n", node.brk_label.as_ref().unwrap()));
            }
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("{}:\n", node.cont_label.as_ref().unwrap()));
            if let Some(inc) = node.inc.as_ref() {
                gen_expr(inc, result, files, current_fn, depth)?;
            }
            result.push_str(&format!("  jmp {}\n", l_begin));
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::While => {
            let l_begin = new_unique_name();
            result.push_str(&format!("{}:\n", node.cont_label.as_ref().unwrap()));
            result.push_str(&format!("{}:\n", l_begin));
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  je {}\n", node.brk_label.as_ref().unwrap()));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("  jmp {}\n", l_begin));
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::Do => {
            let l_begin = new_unique_name();
            result.push_str(&format!("{}:\n", l_begin));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("{}:\n", node.cont_label.as_ref().unwrap()));
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            cmp_zero(node.cond.as_ref().unwrap().ty.as_ref().unwrap(), result);
            result.push_str(&format!("  jne {}\n", l_begin));
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::Block => {
            let mut n = node.body.as_ref();
            while let Some(stmt_node) = n {
                gen_stmt(stmt_node, result, files, current_fn, depth)?;
                n = stmt_node.next.as_ref();
            }
        }
        NodeKind::Return => {
            if let Some(lhs) = node.lhs.as_ref() {
                gen_expr(lhs, result, files, current_fn, depth)?;

                let ty = lhs.ty.as_ref().unwrap();
                if ty.kind == TypeKind::Struct || ty.kind == TypeKind::Union {
                    if ty.size <= 16 {
                        copy_struct_reg(ty, result);
                    } else {
                        let param = &current_fn.params[0];
                        copy_struct_mem(ty, param.offset, result);
                    }
                }
            }
            let jmp = epilogue_lbl(&current_fn.name);
            result.push_str(&format!("  jmp {}\n", jmp));
        }
        NodeKind::ExprStmt => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
        }
        NodeKind::Asm => {
            result.push_str("  ");
            result.push_str(node.asm_str.as_ref().unwrap());
            result.push('\n');
        }
        NodeKind::Goto => {
            result.push_str(&format!("  jmp {}\n", node.unique_label.as_ref().unwrap()));
        }
        NodeKind::GotoExpr => {
            gen_expr(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
            result.push_str("  jmp *%rax\n");
        }
        NodeKind::Label => {
            result.push_str(&format!("{}:\n", node.unique_label.as_ref().unwrap()));
            gen_stmt(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
        }
        NodeKind::Switch => {
            gen_expr(
                node.cond.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;

            let is_64 = node.cond.as_ref().unwrap().ty.as_ref().unwrap().size == 8;
            let ax = if is_64 { "%rax" } else { "%eax" };
            let di = if is_64 { "%rdi" } else { "%edi" };

            let mut case_node = node.case_next.as_ref();
            while let Some(cn) = case_node {
                if cn.begin == cn.end {
                    result.push_str(&format!("  cmp ${}, {}\n", cn.begin, ax));
                    result.push_str(&format!("  je {}\n", cn.label.as_ref().unwrap()));
                } else {
                    result.push_str(&format!("  mov {}, {}\n", ax, di));
                    result.push_str(&format!("  sub ${}, {}\n", cn.begin, di));
                    result.push_str(&format!("  cmp ${}, {}\n", cn.end - cn.begin, di));
                    result.push_str(&format!("  jbe {}\n", cn.label.as_ref().unwrap()));
                }
                case_node = cn.case_next.as_ref();
            }

            if let Some(default) = node.default_case.as_ref() {
                result.push_str(&format!("  jmp {}\n", default.label.as_ref().unwrap()));
            }

            result.push_str(&format!("  jmp {}\n", node.brk_label.as_ref().unwrap()));
            gen_stmt(
                node.then.as_ref().unwrap(),
                result,
                files,
                current_fn,
                depth,
            )?;
            result.push_str(&format!("{}:\n", node.brk_label.as_ref().unwrap()));
        }
        NodeKind::Case => {
            result.push_str(&format!("{}:\n", node.label.as_ref().unwrap()));
            gen_stmt(node.lhs.as_ref().unwrap(), result, files, current_fn, depth)?;
        }
        _ => {
            return Err(error_at(
                files,
                node.file_no,
                node.tok_loc,
                "invalid statement",
            ));
        }
    }
    Ok(())
}

fn store_gp(r: usize, offset: i64, sz: i64, result: &mut String) {
    let argreg8 = ["%dil", "%sil", "%dl", "%cl", "%r8b", "%r9b"];
    let argreg16 = ["%di", "%si", "%dx", "%cx", "%r8w", "%r9w"];
    let argreg32 = ["%edi", "%esi", "%edx", "%ecx", "%r8d", "%r9d"];
    let argreg64 = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
    match sz {
        1 => result.push_str(&format!("  mov {}, {}(%rbp)\n", argreg8[r], offset)),
        2 => result.push_str(&format!("  mov {}, {}(%rbp)\n", argreg16[r], offset)),
        4 => result.push_str(&format!("  mov {}, {}(%rbp)\n", argreg32[r], offset)),
        8 => result.push_str(&format!("  mov {}, {}(%rbp)\n", argreg64[r], offset)),
        _ => {
            for i in 0..sz {
                result.push_str(&format!("  mov {}, {}(%rbp)\n", argreg8[r], offset + i));
                result.push_str(&format!("  shr $8, {}\n", argreg64[r]));
            }
        }
    }
}

fn store_fp(r: usize, offset: i64, sz: i64, result: &mut String) {
    match sz {
        4 => result.push_str(&format!("  movss %xmm{}, {}(%rbp)\n", r, offset)),
        8 => result.push_str(&format!("  movsd %xmm{}, {}(%rbp)\n", r, offset)),
        _ => unreachable!(),
    }
}

fn align_to(n: i64, align: i64) -> i64 {
    (n + align - 1) / align * align
}

fn effective_var_align(var: &Obj) -> i64 {
    if matches!(var.ty.kind, TypeKind::Array) && var.ty.size >= 16 {
        var.align.max(16)
    } else {
        var.align
    }
}

fn fix_var_offsets(node: &mut Node, locals: &[Obj]) {
    if let Some(var) = &mut node.var
        && let Some(lv) = locals.iter().find(|l| l.unique_id == var.unique_id)
    {
        var.offset = lv.offset;
    }
    if let Some(ret_buffer) = &mut node.ret_buffer
        && let Some(lv) = locals.iter().find(|l| l.unique_id == ret_buffer.unique_id)
    {
        ret_buffer.offset = lv.offset;
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

pub fn emit_assembly(opt_include: &[String]) -> Result<String, String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(String::from(
            "Unsupported target architecture: require x86_64",
        ));
    }

    let base_file =
        std::env::var("CC_RS_BASE_FILE").map_err(|_| "base file not set".to_string())?;
    let tok = tokenize_input(&base_file, opt_include)?;
    reset_counter();
    let mut tok = preprocess(tok)?;

    let mut globals: Vec<Obj> = Vec::new();
    declare_builtin_functions(&mut globals);
    let mut tag_scope_stack: Vec<Vec<TagScope>> = vec![Vec::new()];
    let mut scope_stack: Vec<Vec<VarScope>> = vec![Vec::new()];

    let files = get_input_files();

    let mut empty_locals: Vec<Obj> = Vec::new();
    while tok.kind != TokenKind::Eof {
        empty_locals.clear();
        let mut attr = VarAttr::default();
        let (basety, new_tok) = declspec(
            &files,
            &tok,
            &mut tag_scope_stack,
            &mut scope_stack,
            Some(&mut attr),
            &mut empty_locals,
            &mut globals,
        )?;
        tok = new_tok;

        if attr.is_typedef {
            tok = parse_typedef(
                &files,
                &tok,
                basety,
                &mut tag_scope_stack,
                &mut scope_stack,
                &mut empty_locals,
                &mut globals,
            )?;
            continue;
        }

        if is_function(&files, &tok, &scope_stack)? {
            let (_, new_tok) = function(
                &files,
                &tok,
                basety,
                &mut globals,
                &mut tag_scope_stack,
                &mut scope_stack,
                &attr,
            )?;
            tok = new_tok;
        } else {
            tok = global_variable(
                &files,
                &tok,
                basety,
                &mut globals,
                &mut tag_scope_stack,
                &mut scope_stack,
                &attr,
            )?;
        }
    }

    mark_live_globals(&mut globals);
    scan_globals(&mut globals);

    let mut result = String::new();
    let files = get_input_files();

    for file in &files {
        result.push_str(&format!("  .file {} \"{}\"\n", file.file_no, file.name));
    }

    for var in globals.iter() {
        if var.is_function || !var.is_definition {
            continue;
        }
        if var.is_static {
            result.push_str(&format!("  .local {}\n", var.name));
        } else {
            result.push_str(&format!("  .globl {}\n", var.name));
        }

        let align = effective_var_align(var);

        if get_opt_fcommon() && var.is_tentative {
            result.push_str(&format!(
                "  .comm {}, {}, {}\n",
                var.name, var.ty.size, align
            ));
            continue;
        }

        if let Some(init_data) = &var.init_data {
            if var.is_tls {
                result.push_str("  .section .tdata,\"awT\",@progbits\n");
            } else {
                result.push_str("  .data\n");
            }
            result.push_str(&format!("  .type {}, @object\n", var.name));
            result.push_str(&format!("  .size {}, {}\n", var.name, var.ty.size));
            result.push_str(&format!("  .align {}\n", align));
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

        if var.is_tls {
            result.push_str("  .section .tbss,\"awT\",@nobits\n");
        } else {
            result.push_str("  .bss\n");
        }
        result.push_str(&format!("  .align {}\n", align));
        result.push_str(&format!("{}:\n", var.name));
        result.push_str(&format!("  .zero {}\n", var.ty.size));
    }

    for func in globals.iter_mut() {
        if !func.is_function || !func.is_definition {
            continue;
        }

        if !func.is_live {
            continue;
        }

        let mut top = 16;
        let mut bottom = 0;

        let mut gp = 0;
        let mut fp = 0;

        for var in func.locals.iter_mut() {
            var.offset = 0;
        }

        for var in func.params.iter_mut() {
            var.offset = 0;
        }

        for var in func.params.iter_mut() {
            let ty = &var.ty;
            match ty.kind {
                TypeKind::Struct | TypeKind::Union => {
                    if ty.size > 16 {
                        top = align_to(top, 8);
                        var.offset = top;
                        if let Some(local_var) = func.locals.iter_mut().find(|l| l.name == var.name)
                        {
                            local_var.offset = top;
                        }
                        top += ty.size;
                    } else {
                        let fp1 = has_flonum1(ty);
                        let fp2 = if ty.size > 8 { has_flonum2(ty) } else { false };
                        if (fp + fp1 as i32 + fp2 as i32) < FP_MAX
                            && (gp + !fp1 as i32 + !fp2 as i32) < GP_MAX
                        {
                            fp += fp1 as i32 + fp2 as i32;
                            gp += !fp1 as i32 + !fp2 as i32;
                        } else {
                            top = align_to(top, 8);
                            var.offset = top;
                            if let Some(local_var) =
                                func.locals.iter_mut().find(|l| l.name == var.name)
                            {
                                local_var.offset = top;
                            }
                            top += ty.size;
                        }
                    }
                }
                TypeKind::Float | TypeKind::Double => {
                    if fp < FP_MAX {
                        fp += 1;
                    } else {
                        top = align_to(top, 8);
                        var.offset = top;
                        if let Some(local_var) = func.locals.iter_mut().find(|l| l.name == var.name)
                        {
                            local_var.offset = top;
                        }
                        top += ty.size;
                    }
                }
                TypeKind::LDouble => {
                    top = align_to(top, ty.align);
                    var.offset = top;
                    if let Some(local_var) = func.locals.iter_mut().find(|l| l.name == var.name) {
                        local_var.offset = top;
                    }
                    top += ty.size;
                }
                _ => {
                    if gp < GP_MAX {
                        gp += 1;
                    } else {
                        top = align_to(top, 8);
                        var.offset = top;
                        if let Some(local_var) = func.locals.iter_mut().find(|l| l.name == var.name)
                        {
                            local_var.offset = top;
                        }
                        top += ty.size;
                    }
                }
            }
        }

        for var in func.locals.iter_mut().rev() {
            if var.offset != 0 {
                continue;
            }

            let size = if matches!(var.ty.kind, TypeKind::Struct | TypeKind::Union)
                && var.ty.size <= 16
                && func.params.iter().any(|p| p.name == var.name)
            {
                16
            } else {
                var.ty.size
            };

            bottom += size;
            bottom = align_to(bottom, effective_var_align(var));
            var.offset = -bottom;
        }

        let stack_size = align_to(bottom, 16);

        let locals = func.locals.clone();
        if let Some(va_area) = &mut func.va_area
            && let Some(lv) = locals.iter().find(|l| l.unique_id == va_area.unique_id)
        {
            va_area.offset = lv.offset;
        }
        if let Some(alloca_bottom) = &mut func.alloca_bottom
            && let Some(lv) = locals
                .iter()
                .find(|l| l.unique_id == alloca_bottom.unique_id)
        {
            alloca_bottom.offset = lv.offset;
        }
        if let Some(body) = &mut func.body {
            fix_var_offsets(body, &locals);
        }

        if func.is_static {
            result.push_str(&format!("  .local {}\n", func.name));
        } else {
            result.push_str(&format!("  .globl {}\n", func.name));
        }
        result.push_str("  .text\n");
        result.push_str(&format!("  .type {}, @function\n", func.name));
        result.push_str(&format!("{}:\n", func.name));

        result.push_str("  push %rbp\n");
        result.push_str("  mov %rsp, %rbp\n");
        result.push_str(&format!("  sub ${}, %rsp\n", stack_size));

        if let Some(alloca_bottom) = &func.alloca_bottom {
            result.push_str(&format!("  mov %rsp, {}(%rbp)\n", alloca_bottom.offset));
        }

        if let Some(va_area) = &func.va_area {
            let off = va_area.offset;
            let mut gp = 0;
            let mut fp = 0;
            for var in func.params.iter() {
                if crate::is_sse_flonum(&var.ty) {
                    fp += 1;
                } else {
                    gp += 1;
                }
            }

            result.push_str(&format!("  movl ${}, {}(%rbp)\n", gp * 8, off));
            result.push_str(&format!("  movl ${}, {}(%rbp)\n", fp * 8 + 48, off + 4));
            result.push_str("  lea 16(%rbp), %rax\n");
            result.push_str(&format!("  movq %rax, {}(%rbp)\n", off + 8));
            result.push_str(&format!("  lea {}(%rbp), %rax\n", off + 24));
            result.push_str(&format!("  movq %rax, {}(%rbp)\n", off + 16));

            result.push_str(&format!("  movq %rdi, {}(%rbp)\n", off + 24));
            result.push_str(&format!("  movq %rsi, {}(%rbp)\n", off + 32));
            result.push_str(&format!("  movq %rdx, {}(%rbp)\n", off + 40));
            result.push_str(&format!("  movq %rcx, {}(%rbp)\n", off + 48));
            result.push_str(&format!("  movq %r8, {}(%rbp)\n", off + 56));
            result.push_str(&format!("  movq %r9, {}(%rbp)\n", off + 64));
            result.push_str(&format!("  movsd %xmm0, {}(%rbp)\n", off + 72));
            result.push_str(&format!("  movsd %xmm1, {}(%rbp)\n", off + 80));
            result.push_str(&format!("  movsd %xmm2, {}(%rbp)\n", off + 88));
            result.push_str(&format!("  movsd %xmm3, {}(%rbp)\n", off + 96));
            result.push_str(&format!("  movsd %xmm4, {}(%rbp)\n", off + 104));
            result.push_str(&format!("  movsd %xmm5, {}(%rbp)\n", off + 112));
            result.push_str(&format!("  movsd %xmm6, {}(%rbp)\n", off + 120));
            result.push_str(&format!("  movsd %xmm7, {}(%rbp)\n", off + 128));
        }

        let mut gp = 0;
        let mut fp = 0;
        for var in func.params.iter_mut() {
            let local_var = func.locals.iter().find(|l| l.name == var.name);
            if let Some(lv) = local_var {
                var.offset = lv.offset;
            }

            if var.offset > 0 {
                continue;
            }

            let ty = &var.ty;
            match ty.kind {
                TypeKind::Struct | TypeKind::Union => {
                    if ty.size > 16 {
                        continue;
                    }
                    let fp1 = has_flonum1(ty);
                    let fp2 = if ty.size > 8 { has_flonum2(ty) } else { false };
                    if fp1 {
                        store_fp(fp, var.offset, 8.min(ty.size), &mut result);
                        fp += 1;
                    } else {
                        store_gp(gp, var.offset, 8.min(ty.size), &mut result);
                        gp += 1;
                    }
                    if ty.size > 8 {
                        if fp2 {
                            store_fp(fp, var.offset + 8, ty.size - 8, &mut result);
                            fp += 1;
                        } else {
                            store_gp(gp, var.offset + 8, ty.size - 8, &mut result);
                            gp += 1;
                        }
                    }
                }
                TypeKind::Float | TypeKind::Double => {
                    store_fp(fp, var.offset, ty.size, &mut result);
                    fp += 1;
                }
                TypeKind::LDouble => {}
                _ => {
                    store_gp(gp, var.offset, ty.size, &mut result);
                    gp += 1;
                }
            }
        }

        let mut depth: i32 = 0;
        gen_stmt(
            func.body.as_ref().unwrap(),
            &mut result,
            &files,
            func,
            &mut depth,
        )?;
        assert!(depth == 0, "depth should be 0 after function body");

        if func.name == "main" {
            result.push_str("  movq $0, %rax\n");
        }

        let epilogue = epilogue_lbl(&func.name);
        result.push_str(&format!("{}:\n", epilogue));
        result.push_str("  mov %rbp, %rsp\n");
        result.push_str("  pop %rbp\n");
        result.push_str("  ret\n");
    }

    Ok(result)
}

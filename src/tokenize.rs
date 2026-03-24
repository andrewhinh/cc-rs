use crate::{Token, TokenKind, Type, error_at, error_tok};

pub fn new_token(kind: TokenKind, start: usize, end: usize) -> Token {
    Token {
        kind,
        next: None,
        val: 0,
        fval: 0.0,
        loc: start,
        len: end - start,
        ty: None,
        str: None,
        line_no: 0,
    }
}

fn read_escaped_char(chars: &[char], pos: usize) -> Result<(char, usize), String> {
    if pos < chars.len() && chars[pos] >= '0' && chars[pos] <= '7' {
        let mut c = (chars[pos] as i64) - ('0' as i64);
        let mut consumed = 1;

        if pos + 1 < chars.len() && chars[pos + 1] >= '0' && chars[pos + 1] <= '7' {
            c = c * 8 + (chars[pos + 1] as i64) - ('0' as i64);
            consumed = 2;

            if pos + 2 < chars.len() && chars[pos + 2] >= '0' && chars[pos + 2] <= '7' {
                c = c * 8 + (chars[pos + 2] as i64) - ('0' as i64);
                consumed = 3;
            }
        }

        return Ok((char::from_u32(c as u32).unwrap_or('\0'), consumed));
    }

    if pos >= chars.len() {
        return Ok(('\0', 0));
    }

    if chars[pos] == 'x' {
        let mut c: u32 = 0;
        let mut consumed = 0;
        let mut i = pos + 1;

        while i < chars.len() {
            if let Some(digit) = chars[i].to_digit(16) {
                c = (c << 4) + digit;
                consumed += 1;
                i += 1;
            } else {
                break;
            }
        }

        if consumed == 0 {
            return Err("invalid hex escape sequence".to_string());
        }

        return Ok((char::from_u32(c).unwrap_or('\0'), consumed + 1));
    }

    let c = match chars[pos] {
        'a' => '\x07',
        'b' => '\x08',
        't' => '\x09',
        'n' => '\x0A',
        'v' => '\x0B',
        'f' => '\x0C',
        'r' => '\x0D',
        'e' => '\x1B',
        other => other,
    };
    Ok((c, 1))
}

fn read_punct(chars: &[char], pos: usize) -> Option<usize> {
    let remaining: String = chars[pos..].iter().collect();
    if remaining.starts_with("<<=") || remaining.starts_with(">>=") || remaining.starts_with("...")
    {
        return Some(3);
    }
    if remaining.starts_with("==")
        || remaining.starts_with("!=")
        || remaining.starts_with("<=")
        || remaining.starts_with(">=")
        || remaining.starts_with("->")
        || remaining.starts_with("+=")
        || remaining.starts_with("-=")
        || remaining.starts_with("*=")
        || remaining.starts_with("/=")
        || remaining.starts_with("%=")
        || remaining.starts_with("&=")
        || remaining.starts_with("|=")
        || remaining.starts_with("^=")
        || remaining.starts_with("++")
        || remaining.starts_with("--")
        || remaining.starts_with("&&")
        || remaining.starts_with("||")
        || remaining.starts_with("<<")
        || remaining.starts_with(">>")
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
        "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "int"
            | "sizeof"
            | "char"
            | "short"
            | "struct"
            | "union"
            | "long"
            | "void"
            | "typedef"
            | "_Bool"
            | "enum"
            | "static"
            | "goto"
            | "break"
            | "continue"
            | "switch"
            | "case"
            | "default"
            | "extern"
            | "_Alignof"
            | "_Alignas"
            | "do"
            | "signed"
            | "unsigned"
            | "const"
            | "volatile"
            | "auto"
            | "register"
            | "restrict"
            | "__restrict"
            | "__restrict__"
            | "_Noreturn"
            | "float"
            | "double"
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

fn add_line_numbers(src: &str, tok: &mut Token) {
    let mut p = 0;
    let mut n = 1;
    let mut cur = tok;

    loop {
        if p == cur.loc {
            cur.line_no = n;
            if cur.next.is_none() {
                break;
            }
            cur = cur.next.as_mut().unwrap();
        }
        if src.as_bytes().get(p) == Some(&b'\n') {
            n += 1;
        }
        p += 1;
    }
}

fn read_int_literal(chars: &[char], pos: usize) -> Result<(i64, usize, Type), String> {
    let mut p = pos;

    let base = if p + 2 < chars.len()
        && chars[p] == '0'
        && (chars[p + 1] == 'x' || chars[p + 1] == 'X')
        && chars[p + 2].is_ascii_hexdigit()
    {
        p += 2;
        16
    } else if p + 2 < chars.len()
        && chars[p] == '0'
        && (chars[p + 1] == 'b' || chars[p + 1] == 'B')
        && (chars[p + 2] == '0' || chars[p + 2] == '1')
    {
        p += 2;
        2
    } else if chars[p] == '0' {
        8
    } else {
        10
    };

    let mut num_str = String::new();
    match base {
        16 => {
            while p < chars.len() && chars[p].is_ascii_hexdigit() {
                num_str.push(chars[p]);
                p += 1;
            }
        }
        10 => {
            while p < chars.len() && chars[p].is_ascii_digit() {
                num_str.push(chars[p]);
                p += 1;
            }
        }
        8 => {
            while p < chars.len() && chars[p] >= '0' && chars[p] <= '7' {
                num_str.push(chars[p]);
                p += 1;
            }
        }
        2 => {
            while p < chars.len() && (chars[p] == '0' || chars[p] == '1') {
                num_str.push(chars[p]);
                p += 1;
            }
        }
        _ => unreachable!(),
    }

    let val = i64::from_str_radix(&num_str, base).map_err(|_| "invalid digit".to_string())?;

    let mut l = false;
    let mut u = false;

    let suffix: String = chars[p..]
        .iter()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let suffix_upper = suffix.to_uppercase();

    if suffix_upper == "LLU"
        || suffix_upper == "ULL"
        || suffix_upper == "UL"
        || suffix_upper == "LU"
    {
        p += suffix.len();
        l = true;
        u = true;
    } else if suffix_upper == "LL" || suffix_upper == "L" {
        p += suffix.len();
        l = true;
    } else if suffix_upper == "U" {
        p += suffix.len();
        u = true;
    }

    let ty = if base == 10 {
        if l && u {
            Type::new_ulong()
        } else if l {
            Type::new_long()
        } else if u {
            if (val as u64) >> 32 != 0 {
                Type::new_ulong()
            } else {
                Type::new_uint()
            }
        } else if (val as u64) >> 31 != 0 {
            Type::new_long()
        } else {
            Type::new_int()
        }
    } else if l && u {
        Type::new_ulong()
    } else if l {
        if (val as u64) >> 63 != 0 {
            Type::new_ulong()
        } else {
            Type::new_long()
        }
    } else if u {
        if (val as u64) >> 32 != 0 {
            Type::new_ulong()
        } else {
            Type::new_uint()
        }
    } else if (val as u64) >> 63 != 0 {
        Type::new_ulong()
    } else if (val as u64) >> 32 != 0 {
        Type::new_long()
    } else if (val as u64) >> 31 != 0 {
        Type::new_uint()
    } else {
        Type::new_int()
    };

    Ok((val, p, ty))
}

fn read_number(chars: &[char], pos: usize) -> Result<(Token, usize), String> {
    let start = pos;

    if chars[pos] == '.' {
        let num_str: String = chars[start..]
            .iter()
            .take_while(|c| {
                c.is_ascii_digit()
                    || **c == '.'
                    || **c == 'e'
                    || **c == 'E'
                    || **c == 'p'
                    || **c == 'P'
                    || **c == '+'
                    || **c == '-'
            })
            .collect();

        let mut p = start + num_str.len();
        let fval = parse_float(&num_str)?;

        let ty = if p < chars.len() && (chars[p] == 'f' || chars[p] == 'F') {
            p += 1;
            Type::new_float()
        } else if p < chars.len() && (chars[p] == 'l' || chars[p] == 'L') {
            p += 1;
            Type::new_double()
        } else {
            Type::new_double()
        };

        let mut tok = new_token(TokenKind::Num, start, p);
        tok.fval = fval;
        tok.ty = Some(ty);
        return Ok((tok, p));
    }

    let (val, end, ty) = read_int_literal(chars, pos)?;

    if end < chars.len() && ['.', 'e', 'E', 'f', 'F', 'p', 'P'].contains(&chars[end]) {
        let num_str: String = chars[start..]
            .iter()
            .take_while(|c| {
                c.is_ascii_digit()
                    || **c == '.'
                    || **c == 'e'
                    || **c == 'E'
                    || **c == 'p'
                    || **c == 'P'
                    || **c == '+'
                    || **c == '-'
                    || **c == 'x'
                    || **c == 'X'
            })
            .collect();

        let mut p = start + num_str.len();
        let fval = parse_float(&num_str)?;

        let ty = if p < chars.len() && (chars[p] == 'f' || chars[p] == 'F') {
            p += 1;
            Type::new_float()
        } else if p < chars.len() && (chars[p] == 'l' || chars[p] == 'L') {
            p += 1;
            Type::new_double()
        } else {
            Type::new_double()
        };

        let mut tok = new_token(TokenKind::Num, start, p);
        tok.fval = fval;
        tok.ty = Some(ty);
        return Ok((tok, p));
    }

    let mut tok = new_token(TokenKind::Num, start, end);
    tok.val = val;
    tok.ty = Some(ty);
    Ok((tok, end))
}

fn parse_float(s: &str) -> Result<f64, String> {
    if s.starts_with("0x") || s.starts_with("0X") {
        parse_hex_float(s)
    } else {
        s.parse::<f64>()
            .map_err(|_| "invalid floating point number".to_string())
    }
}

fn parse_hex_float(s: &str) -> Result<f64, String> {
    let s = &s[2..];
    let mut result: f64 = 0.0;
    let mut pos = 0;
    let mut has_dot = false;
    let mut frac_divisor: f64 = 1.0;

    while pos < s.len() {
        let c = s.chars().nth(pos).unwrap();
        if c == '.' {
            has_dot = true;
            pos += 1;
            continue;
        }
        if c == 'p' || c == 'P' {
            break;
        }
        if let Some(digit) = c.to_digit(16) {
            result = result * 16.0 + digit as f64;
            if has_dot {
                frac_divisor *= 16.0;
            }
            pos += 1;
        } else {
            break;
        }
    }

    result /= frac_divisor;

    if pos < s.len() {
        let c = s.chars().nth(pos).unwrap();
        if c == 'p' || c == 'P' {
            pos += 1;
            let exp_str: String = s
                .chars()
                .skip(pos)
                .take_while(|c| c.is_ascii_digit() || *c == '+' || *c == '-')
                .collect();
            let exp: i32 = exp_str.parse().map_err(|_| "invalid exponent")?;
            result *= 2_f64.powi(exp);
        }
    }

    Ok(result)
}

pub fn tokenize(filename: &str, src: &str) -> Result<Token, String> {
    let mut head = Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        fval: 0.0,
        loc: 0,
        len: 0,
        ty: None,
        str: None,
        line_no: 0,
    };
    let mut cur = &mut head;
    let chars: Vec<char> = src.chars().collect();
    let mut pos = 0;

    while pos < chars.len() {
        if chars[pos].is_whitespace() {
            pos += 1;
            continue;
        }

        if pos + 1 < chars.len() && chars[pos] == '/' && chars[pos + 1] == '/' {
            pos += 2;
            while pos < chars.len() && chars[pos] != '\n' {
                pos += 1;
            }
            continue;
        }

        if pos + 1 < chars.len() && chars[pos] == '/' && chars[pos + 1] == '*' {
            let start = pos;
            pos += 2;
            let mut found = false;
            while pos + 1 < chars.len() {
                if chars[pos] == '*' && chars[pos + 1] == '/' {
                    pos += 2;
                    found = true;
                    break;
                }
                pos += 1;
            }
            if !found {
                return Err(error_at(filename, src, start, "unclosed block comment"));
            }
            continue;
        }

        if chars[pos] == '"' {
            let start = pos;
            pos += 1;
            let mut str_content: Vec<u8> = Vec::new();
            while pos < chars.len() && chars[pos] != '"' {
                if chars[pos] == '\n' || chars[pos] == '\0' {
                    return Err(error_at(filename, src, start, "unclosed string literal"));
                }
                if chars[pos] == '\\' {
                    pos += 1;
                    if pos >= chars.len() {
                        return Err(error_at(filename, src, start, "unclosed string literal"));
                    }
                    let (escaped, consumed) = read_escaped_char(&chars, pos)
                        .map_err(|e| error_at(filename, src, pos, &e))?;
                    str_content.push(escaped as u8);
                    pos += consumed;
                    continue;
                } else {
                    str_content.push(chars[pos] as u8);
                }
                pos += 1;
            }
            if pos >= chars.len() {
                return Err(error_at(filename, src, start, "unclosed string literal"));
            }
            pos += 1;
            let mut tok = new_token(TokenKind::Str, start, pos);
            let len = str_content.len() + 1;
            tok.ty = Some(Type::new_array(Type::new_char(), len as i64));
            tok.str = Some(str_content);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            continue;
        }

        if chars[pos] == '\'' {
            let start = pos;
            pos += 1;
            if pos >= chars.len() {
                return Err(error_at(filename, src, start, "unclosed char literal"));
            }
            let c: i64;
            if chars[pos] == '\\' {
                pos += 1;
                if pos >= chars.len() {
                    return Err(error_at(filename, src, start, "unclosed char literal"));
                }
                let (escaped, consumed) =
                    read_escaped_char(&chars, pos).map_err(|e| error_at(filename, src, pos, &e))?;
                c = (escaped as u8) as i8 as i64;
                pos += consumed;
            } else {
                c = (chars[pos] as u8) as i8 as i64;
                pos += 1;
            }
            if pos >= chars.len() || chars[pos] != '\'' {
                return Err(error_at(filename, src, pos, "unclosed char literal"));
            }
            pos += 1;
            let mut tok = new_token(TokenKind::Num, start, pos);
            tok.val = c;
            tok.ty = Some(Type::new_int());
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            continue;
        }

        if chars[pos].is_ascii_digit()
            || (chars[pos] == '.' && pos + 1 < chars.len() && chars[pos + 1].is_ascii_digit())
        {
            let (tok, end) =
                read_number(&chars, pos).map_err(|e| error_at(filename, src, pos, &e))?;
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            pos = end;
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
    add_line_numbers(src, &mut tok);
    convert_keywords(src, &mut tok);
    Ok(*tok)
}

pub fn equal(src: &str, tok: &Token, s: &str) -> bool {
    (tok.kind == TokenKind::Punct || tok.kind == TokenKind::Keyword)
        && tok.len == s.len()
        && src.chars().skip(tok.loc).take(tok.len).eq(s.chars())
}

pub fn skip(filename: &str, src: &str, tok: &Token, s: &str) -> Result<Token, String> {
    if equal(src, tok, s) {
        return Ok(*tok.next.as_ref().unwrap().clone());
    }
    Err(error_tok(filename, src, tok, &format!("expected '{s}'")))
}

pub fn consume(src: &str, tok: &Token, s: &str) -> (bool, Token) {
    if equal(src, tok, s) {
        (true, *tok.next.as_ref().unwrap().clone())
    } else {
        (false, tok.clone())
    }
}

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{File, Token, TokenKind, Type};

static FILE_NO: AtomicUsize = AtomicUsize::new(0);

pub fn get_file_no() -> usize {
    FILE_NO.fetch_add(1, Ordering::SeqCst) + 1
}

fn char_index_to_byte_offset(s: &str, char_index: usize) -> usize {
    s.chars().take(char_index).map(|c| c.len_utf8()).sum()
}

fn byte_offset_and_line_at(s: &str, char_index: usize) -> (usize, usize) {
    let mut b = 0;
    let mut line = 1;
    for (i, c) in s.chars().enumerate() {
        if i >= char_index {
            break;
        }
        if c == '\n' {
            line += 1;
        }
        b += c.len_utf8();
    }
    (b, line)
}

fn read_wide_escaped_char(chars: &[char], pos: usize) -> Result<(i64, usize), String> {
    if pos < chars.len() && ('0'..='7').contains(&chars[pos]) {
        let mut c: u32 = u32::from((chars[pos] as u8) - b'0');
        let mut n = 1;
        if pos + 1 < chars.len() && ('0'..='7').contains(&chars[pos + 1]) {
            c = (c << 3) + u32::from((chars[pos + 1] as u8) - b'0');
            n = 2;
            if pos + 2 < chars.len() && ('0'..='7').contains(&chars[pos + 2]) {
                c = (c << 3) + u32::from((chars[pos + 2] as u8) - b'0');
                n = 3;
            }
        }
        return Ok((c as i32 as i64, n));
    }

    if pos >= chars.len() {
        return Ok((0, 0));
    }

    if chars[pos] == 'x' {
        let mut c: u32 = 0;
        let mut consumed: usize = 0;
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
        return Ok((c as i32 as i64, 1 + consumed));
    }

    let v = match chars[pos] {
        'a' => 0x07i64,
        'b' => 0x08,
        't' => 0x09,
        'n' => 0x0A,
        'v' => 0x0B,
        'f' => 0x0C,
        'r' => 0x0D,
        'e' => 0x1B,
        other => other as u8 as i64,
    };
    Ok((v, 1))
}

pub fn new_token(
    kind: TokenKind,
    start: usize,
    end: usize,
    at_bol: bool,
    has_space: bool,
    file_no: usize,
) -> Token {
    Token {
        kind,
        next: None,
        val: 0,
        fval: 0.0,
        loc: start,
        len: end - start,
        ty: None,
        str: None,
        file_no,
        line_no: 0,
        at_bol,
        has_space,
        hideset: HashSet::new(),
        origin: None,
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

fn encode_utf8(c: u32, buf: &mut [u8; 4]) -> usize {
    if c <= 0x7F {
        buf[0] = c as u8;
        return 1;
    }
    if c <= 0x7FF {
        buf[0] = 0b1100_0000 | ((c >> 6) as u8);
        buf[1] = 0b1000_0000 | ((c & 0b11_1111) as u8);
        return 2;
    }
    if c <= 0xFFFF {
        buf[0] = 0b1110_0000 | ((c >> 12) as u8);
        buf[1] = 0b1000_0000 | (((c >> 6) & 0b11_1111) as u8);
        buf[2] = 0b1000_0000 | ((c & 0b11_1111) as u8);
        return 3;
    }
    buf[0] = 0b1111_0000 | ((c >> 18) as u8);
    buf[1] = 0b1000_0000 | (((c >> 12) & 0b11_1111) as u8);
    buf[2] = 0b1000_0000 | (((c >> 6) & 0b11_1111) as u8);
    buf[3] = 0b1000_0000 | ((c & 0b11_1111) as u8);
    4
}

fn from_hex(b: u8) -> u32 {
    match b {
        b'0'..=b'9' => u32::from(b - b'0'),
        b'a'..=b'f' => 10 + u32::from(b - b'a'),
        b'A'..=b'F' => 10 + u32::from(b - b'A'),
        _ => 0,
    }
}

fn read_universal_char(s: &[u8], len: usize) -> u32 {
    if s.len() < len {
        return 0;
    }
    let mut c: u32 = 0;
    for &b in s.iter().take(len) {
        if !b.is_ascii_hexdigit() {
            return 0;
        }
        c = (c << 4) | from_hex(b);
    }
    c
}

fn convert_universal_chars(contents: &mut String) {
    let b = std::mem::take(contents).into_bytes();
    let mut p: usize = 0;
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut enc = [0u8; 4];
    while p < b.len() {
        if p + 1 < b.len() && b[p] == b'\\' && b[p + 1] == b'u' {
            let c = if p + 6 <= b.len() {
                read_universal_char(&b[p + 2..p + 6], 4)
            } else {
                0
            };
            if c != 0 {
                p += 6;
                let n = encode_utf8(c, &mut enc);
                out.extend_from_slice(&enc[..n]);
            } else {
                out.push(b[p]);
                p += 1;
            }
        } else if p + 1 < b.len() && b[p] == b'\\' && b[p + 1] == b'U' {
            let c = if p + 10 <= b.len() {
                read_universal_char(&b[p + 2..p + 10], 8)
            } else {
                0
            };
            if c != 0 {
                p += 10;
                let n = encode_utf8(c, &mut enc);
                out.extend_from_slice(&enc[..n]);
            } else {
                out.push(b[p]);
                p += 1;
            }
        } else if b[p] == b'\\' && p + 1 < b.len() {
            out.push(b[p]);
            p += 1;
            out.push(b[p]);
            p += 1;
        } else {
            out.push(b[p]);
            p += 1;
        }
    }
    *contents = String::from_utf8(out).expect("convert_universal_chars");
}

fn push_char_utf8(str_content: &mut Vec<u8>, c: char) {
    let mut buf = [0u8; 4];
    str_content.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
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
        || remaining.starts_with("##")
    {
        return Some(2);
    }
    if chars[pos].is_ascii_punctuation() {
        return Some(1);
    }
    None
}

fn add_line_numbers(src: &str, first: &mut Token) {
    let chars: Vec<char> = src.chars().collect();
    let max_p = chars.len();
    let mut p: usize = 0;
    let mut n: usize = 1;
    let mut cur: &mut Token = first;

    loop {
        if p == cur.loc {
            cur.line_no = n;
            if cur.next.is_none() {
                break;
            }
            cur = cur.next.as_mut().unwrap();
        }
        if p >= max_p {
            break;
        }
        if chars[p] == '\n' {
            n += 1;
        }
        p += 1;
    }
}

fn try_parse_int_literal(s: &str) -> Option<(i64, Type)> {
    let chars: Vec<char> = s.chars().collect();
    let mut p = 0;

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

    let val = u64::from_str_radix(&num_str, base).ok()?;

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

    if p != s.len() {
        return None;
    }

    let ty = if base == 10 {
        if l && u {
            Type::new_ulong()
        } else if l {
            Type::new_long()
        } else if u {
            if val >> 32 != 0 {
                Type::new_ulong()
            } else {
                Type::new_uint()
            }
        } else if val >> 31 != 0 {
            Type::new_long()
        } else {
            Type::new_int()
        }
    } else if l && u {
        Type::new_ulong()
    } else if l {
        if val >> 63 != 0 {
            Type::new_ulong()
        } else {
            Type::new_long()
        }
    } else if u {
        if val >> 32 != 0 {
            Type::new_ulong()
        } else {
            Type::new_uint()
        }
    } else if val >> 63 != 0 {
        Type::new_ulong()
    } else if val >> 32 != 0 {
        Type::new_long()
    } else if val >> 31 != 0 {
        Type::new_uint()
    } else {
        Type::new_int()
    };

    Some((val as i64, ty))
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

pub fn convert_pp_number(files: &[File], tok: &mut Token) -> Result<(), String> {
    let file = match files.iter().find(|f| f.file_no == tok.file_no) {
        Some(f) => f,
        None => return Err("file not found".to_string()),
    };
    let s: String = file.contents.chars().skip(tok.loc).take(tok.len).collect();

    if let Some((val, ty)) = try_parse_int_literal(&s) {
        tok.kind = TokenKind::Num;
        tok.val = val;
        tok.ty = Some(ty);
        return Ok(());
    }

    let chars: Vec<char> = s.chars().collect();
    let fval = parse_float(&s)?;

    let ty =
        if !chars.is_empty() && (chars[chars.len() - 1] == 'f' || chars[chars.len() - 1] == 'F') {
            Type::new_float()
        } else {
            Type::new_double()
        };

    tok.kind = TokenKind::Num;
    tok.fval = fval;
    tok.ty = Some(ty);
    Ok(())
}

static INPUT_FILES: Mutex<Vec<File>> = Mutex::new(Vec::new());

pub fn get_input_files() -> Vec<File> {
    INPUT_FILES.lock().unwrap().clone()
}

pub fn add_input_file(file: File) {
    INPUT_FILES.lock().unwrap().push(file);
}

pub fn new_file(name: String, file_no: usize, contents: String) -> File {
    File {
        name,
        file_no,
        contents,
    }
}

pub fn tokenize_file(path: &str) -> Option<Token> {
    let contents = read_file(path)?;
    let file_no = get_file_no();
    let file = new_file(path.to_string(), file_no, contents);

    INPUT_FILES.lock().unwrap().push(file.clone());

    Some(tokenize(&file))
}

fn canonicalize_newline(contents: &mut String) {
    let mut b = std::mem::take(contents).into_bytes();
    let mut i = 0;
    let mut j = 0;
    let blen = b.len();
    while i < blen {
        if b[i] == b'\r' && i + 1 < blen && b[i + 1] == b'\n' {
            i += 2;
            b[j] = b'\n';
            j += 1;
        } else if b[i] == b'\r' {
            i += 1;
            b[j] = b'\n';
            j += 1;
        } else {
            b[j] = b[i];
            i += 1;
            j += 1;
        }
    }
    b.truncate(j);
    *contents = String::from_utf8(b).expect("canonicalize_newline preserves UTF-8");
}

fn remove_backslash_newline(contents: &mut String) {
    let bytes = unsafe { contents.as_bytes_mut() };
    let mut i = 0;
    let mut j = 0;
    let mut n = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            i += 2;
            n += 1;
        } else if bytes[i] == b'\n' {
            bytes[j] = bytes[i];
            j += 1;
            i += 1;
            for _ in 0..n {
                bytes[j] = b'\n';
                j += 1;
            }
            n = 0;
        } else {
            bytes[j] = bytes[i];
            j += 1;
            i += 1;
        }
    }

    for _ in 0..n {
        bytes[j] = b'\n';
        j += 1;
    }

    contents.truncate(j);
}

fn read_file(path: &str) -> Option<String> {
    let contents = if path == "-" {
        use std::io::Read;
        let mut contents = String::new();
        std::io::stdin().read_to_string(&mut contents).ok()?;
        contents
    } else {
        std::fs::read_to_string(path).ok()?
    };
    let mut contents = contents;
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    canonicalize_newline(&mut contents);
    remove_backslash_newline(&mut contents);
    convert_universal_chars(&mut contents);
    Some(contents)
}

pub fn tokenize(file: &File) -> Token {
    let file_no = file.file_no;
    let src = &file.contents;
    let mut head = Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        fval: 0.0,
        loc: 0,
        len: 0,
        ty: None,
        str: None,
        file_no,
        line_no: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
    };
    let mut cur = &mut head;
    let chars: Vec<char> = src.chars().collect();
    let mut pos = 0;
    let mut at_bol = true;
    let mut has_space = false;

    while pos < chars.len() {
        if chars[pos] == '\n' {
            pos += 1;
            at_bol = true;
            has_space = false;
            continue;
        }

        if chars[pos].is_whitespace() {
            pos += 1;
            has_space = true;
            continue;
        }

        if pos + 1 < chars.len() && chars[pos] == '/' && chars[pos + 1] == '/' {
            pos += 2;
            while pos < chars.len() && chars[pos] != '\n' {
                pos += 1;
            }
            has_space = true;
            continue;
        }

        if pos + 1 < chars.len() && chars[pos] == '/' && chars[pos + 1] == '*' {
            pos += 2;
            while pos + 1 < chars.len() {
                if chars[pos] == '*' && chars[pos + 1] == '/' {
                    pos += 2;
                    break;
                }
                pos += 1;
            }
            has_space = true;
            continue;
        }

        if chars[pos] == '"' {
            let start = pos;
            pos += 1;
            let mut str_content: Vec<u8> = Vec::new();
            while pos < chars.len() && chars[pos] != '"' {
                if chars[pos] == '\n' || chars[pos] == '\0' {
                    cur.next = Some(Box::new(make_error_token(
                        file_no,
                        start,
                        "unclosed string literal",
                    )));
                    return *head.next.unwrap();
                }
                if chars[pos] == '\\' {
                    pos += 1;
                    if pos >= chars.len() {
                        cur.next = Some(Box::new(make_error_token(
                            file_no,
                            start,
                            "unclosed string literal",
                        )));
                        return *head.next.unwrap();
                    }
                    match read_escaped_char(&chars, pos) {
                        Ok((escaped, consumed)) => {
                            push_char_utf8(&mut str_content, escaped);
                            pos += consumed;
                        }
                        Err(e) => {
                            cur.next = Some(Box::new(make_error_token(file_no, pos, &e)));
                            return *head.next.unwrap();
                        }
                    }
                    continue;
                } else {
                    push_char_utf8(&mut str_content, chars[pos]);
                }
                pos += 1;
            }
            if pos >= chars.len() {
                cur.next = Some(Box::new(make_error_token(
                    file_no,
                    start,
                    "unclosed string literal",
                )));
                return *head.next.unwrap();
            }
            pos += 1;
            let mut tok = new_token(TokenKind::Str, start, pos, at_bol, has_space, file_no);
            let len = str_content.len() + 1;
            tok.ty = Some(Type::new_array(Type::new_char(), len as i64));
            tok.str = Some(str_content);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            at_bol = false;
            has_space = false;
            continue;
        }

        if pos + 1 < chars.len() && matches!((chars[pos], chars[pos + 1]), ('L' | 'u', '\'')) {
            let utf16 = chars[pos] == 'u';
            let start = pos;
            pos += 2;
            if pos >= chars.len() {
                cur.next = Some(Box::new(make_error_token(
                    file_no,
                    start,
                    "unclosed char literal",
                )));
                return *head.next.unwrap();
            }
            let c: i64;
            if chars[pos] == '\\' {
                pos += 1;
                if pos >= chars.len() {
                    cur.next = Some(Box::new(make_error_token(
                        file_no,
                        start,
                        "unclosed char literal",
                    )));
                    return *head.next.unwrap();
                }
                match read_wide_escaped_char(&chars, pos) {
                    Ok((v, consumed)) => {
                        c = v;
                        pos += consumed;
                    }
                    Err(e) => {
                        cur.next = Some(Box::new(make_error_token(file_no, pos, &e)));
                        return *head.next.unwrap();
                    }
                }
            } else {
                c = (chars[pos] as u32 as i32) as i64;
                pos += 1;
            }
            if pos >= chars.len() || chars[pos] != '\'' {
                cur.next = Some(Box::new(make_error_token(
                    file_no,
                    pos,
                    "unclosed char literal",
                )));
                return *head.next.unwrap();
            }
            pos += 1;
            let mut tok = new_token(TokenKind::Num, start, pos, at_bol, has_space, file_no);
            if utf16 {
                tok.val = c & 0xffff;
                tok.ty = Some(Type::new_ushort());
            } else {
                tok.val = c;
                tok.ty = Some(Type::new_int());
            }
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            at_bol = false;
            has_space = false;
            continue;
        }

        if chars[pos] == '\'' {
            let start = pos;
            pos += 1;
            if pos >= chars.len() {
                cur.next = Some(Box::new(make_error_token(
                    file_no,
                    start,
                    "unclosed char literal",
                )));
                return *head.next.unwrap();
            }
            let c: i64;
            if chars[pos] == '\\' {
                pos += 1;
                if pos >= chars.len() {
                    cur.next = Some(Box::new(make_error_token(
                        file_no,
                        start,
                        "unclosed char literal",
                    )));
                    return *head.next.unwrap();
                }
                match read_escaped_char(&chars, pos) {
                    Ok((escaped, consumed)) => {
                        c = (escaped as u8) as i8 as i64;
                        pos += consumed;
                    }
                    Err(e) => {
                        cur.next = Some(Box::new(make_error_token(file_no, pos, &e)));
                        return *head.next.unwrap();
                    }
                }
            } else {
                c = ((chars[pos] as u32) as u8 as i8) as i64;
                pos += 1;
            }
            if pos >= chars.len() || chars[pos] != '\'' {
                cur.next = Some(Box::new(make_error_token(
                    file_no,
                    pos,
                    "unclosed char literal",
                )));
                return *head.next.unwrap();
            }
            pos += 1;
            let mut tok = new_token(TokenKind::Num, start, pos, at_bol, has_space, file_no);
            tok.val = c;
            tok.ty = Some(Type::new_int());
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            at_bol = false;
            has_space = false;
            continue;
        }

        if chars[pos].is_ascii_digit()
            || (chars[pos] == '.' && pos + 1 < chars.len() && chars[pos + 1].is_ascii_digit())
        {
            let start = pos;
            pos += 1;
            loop {
                if pos + 1 < chars.len()
                    && matches!(chars[pos], 'e' | 'E' | 'p' | 'P')
                    && matches!(chars[pos + 1], '+' | '-')
                {
                    pos += 2;
                } else if pos < chars.len()
                    && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '.')
                {
                    pos += 1;
                } else {
                    break;
                }
            }
            let tok = new_token(TokenKind::PpNum, start, pos, at_bol, has_space, file_no);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            at_bol = false;
            has_space = false;
            continue;
        }

        if chars[pos].is_ascii_alphabetic() || chars[pos] == '_' {
            let start = pos;
            while pos < chars.len() && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_') {
                pos += 1;
            }
            let tok = new_token(TokenKind::Ident, start, pos, at_bol, has_space, file_no);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            at_bol = false;
            has_space = false;
            continue;
        }

        if let Some(len) = read_punct(&chars, pos) {
            let tok = new_token(TokenKind::Punct, pos, pos + len, at_bol, has_space, file_no);
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            pos += len;
            at_bol = false;
            has_space = false;
            continue;
        }

        cur.next = Some(Box::new(make_error_token(file_no, pos, "invalid token")));
        return *head.next.unwrap();
    }

    cur.next = Some(Box::new(new_token(
        TokenKind::Eof,
        pos,
        pos,
        at_bol,
        has_space,
        file_no,
    )));
    let mut tok = head.next.unwrap();
    add_line_numbers(src, &mut tok);
    *tok
}

fn make_error_token(file_no: usize, loc: usize, msg: &str) -> Token {
    Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        fval: 0.0,
        loc,
        len: msg.len(),
        ty: None,
        str: Some(msg.as_bytes().to_vec()),
        file_no,
        line_no: 0,
        at_bol: false,
        has_space: false,
        hideset: HashSet::new(),
        origin: None,
    }
}

pub fn equal(files: &[File], tok: &Token, s: &str) -> bool {
    let file = match files.iter().find(|f| f.file_no == tok.file_no) {
        Some(f) => f,
        None => return false,
    };
    (matches!(
        tok.kind,
        TokenKind::Punct | TokenKind::Keyword | TokenKind::Ident
    )) && file
        .contents
        .chars()
        .skip(tok.loc)
        .take(tok.len)
        .eq(s.chars())
}

pub fn skip(files: &[File], tok: &Token, s: &str) -> Result<Token, String> {
    if equal(files, tok, s) {
        return Ok(*tok.next.as_ref().unwrap().clone());
    }
    Err(error_tok(files, tok, &format!("expected '{s}'")))
}

pub fn consume(files: &[File], tok: &Token, s: &str) -> (bool, Token) {
    if equal(files, tok, s) {
        (true, *tok.next.as_ref().unwrap().clone())
    } else {
        (false, tok.clone())
    }
}

pub fn warn_tok(files: &[File], tok: &Token, msg: &str) {
    let warning = error_tok(files, tok, msg);
    eprint!("warning: {}", warning);
}

pub fn error_tok(files: &[File], tok: &Token, msg: &str) -> String {
    let file = files.iter().find(|f| f.file_no == tok.file_no).unwrap();
    let src = &file.contents;
    let filename = &file.name;

    let b = char_index_to_byte_offset(src, tok.loc);
    let mut line_start = b;
    while line_start > 0 && src.as_bytes()[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    let mut line_end = b;
    while line_end < src.len() && src.as_bytes()[line_end] != b'\n' {
        line_end += 1;
    }

    let line = &src[line_start..line_end];
    let indent = format!("{filename}:{}: ", tok.line_no).len();
    let pos = b - line_start + indent;

    format!(
        "{filename}:{}: {line}\n{:width$}^ {msg}\n",
        tok.line_no,
        "",
        width = pos
    )
}

pub fn error_at(files: &[File], file_no: usize, loc: usize, msg: &str) -> String {
    let file = files.iter().find(|f| f.file_no == file_no).unwrap();
    let src = &file.contents;
    let filename = &file.name;

    let (b, line_no) = byte_offset_and_line_at(src, loc);

    let mut line_start = b;
    while line_start > 0 && src.as_bytes()[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    let mut line_end = b;
    while line_end < src.len() && src.as_bytes()[line_end] != b'\n' {
        line_end += 1;
    }

    let line = &src[line_start..line_end];
    let indent = format!("{filename}:{line_no}: ").len();
    let pos = b - line_start + indent;

    format!(
        "{filename}:{line_no}: {line}\n{:width$}^ {msg}\n",
        "",
        width = pos
    )
}

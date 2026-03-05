use crate::{Token, TokenKind, Type, error_at, error_tok};

pub fn new_token(kind: TokenKind, start: usize, end: usize) -> Token {
    Token {
        kind,
        next: None,
        val: 0,
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
    if remaining.starts_with("==")
        || remaining.starts_with("!=")
        || remaining.starts_with("<=")
        || remaining.starts_with(">=")
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
        "return" | "if" | "else" | "for" | "while" | "int" | "sizeof" | "char"
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

pub fn tokenize(filename: &str, src: &str) -> Result<Token, String> {
    let mut head = Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
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

        if chars[pos].is_ascii_digit() {
            let start = pos;
            let mut num_str = String::new();
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                num_str.push(chars[pos]);
                pos += 1;
            }
            let val = num_str
                .parse::<i64>()
                .map_err(|_| format!("Invalid number: {num_str}"))?;
            let mut tok = new_token(TokenKind::Num, start, pos);
            tok.val = val;
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
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

use crate::{Token, TokenKind, equal, error_tok};

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

fn is_hash(src: &str, tok: &Token) -> bool {
    tok.at_bol && equal(src, tok, "#")
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

fn preprocess2(filename: &str, src: &str, tok: Token) -> Result<Token, String> {
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
        at_bol: false,
    };
    let mut cur = &mut head;
    let mut tok = tok;

    loop {
        if tok.kind == TokenKind::Eof {
            break;
        }

        if !is_hash(src, &tok) {
            let next = tok.next.take();
            cur.next = Some(Box::new(tok));
            cur = cur.next.as_mut().unwrap();
            tok = *next.unwrap();
            continue;
        }

        tok = *tok.next.unwrap();

        if tok.at_bol {
            continue;
        }

        return Err(error_tok(
            filename,
            src,
            &tok,
            "invalid preprocessor directive",
        ));
    }

    cur.next = Some(Box::new(Token {
        kind: TokenKind::Eof,
        next: None,
        val: 0,
        fval: 0.0,
        loc: 0,
        len: 0,
        ty: None,
        str: None,
        line_no: 0,
        at_bol: false,
    }));

    Ok(*head.next.unwrap())
}

pub fn preprocess(filename: &str, src: &str, tok: Token) -> Result<Token, String> {
    let tok = preprocess2(filename, src, tok)?;
    let mut tok = tok;
    convert_keywords(src, &mut tok);
    Ok(tok)
}

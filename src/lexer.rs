use crate::span::Span;
use crate::token::{keyword, Token, TokenKind};

#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

pub struct Lexer<'a> {
    source: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            source: source.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    /// Tokenizes the entire source, collecting every lexical error
    /// instead of stopping at the first one.
    pub fn tokenize(mut self) -> (Vec<Token>, Vec<LexError>) {
        let mut tokens = Vec::new();
        let mut errors = Vec::new();

        loop {
            self.skip_whitespace_and_comments();
            let start_pos = self.pos;
            let start_line = self.line;
            let start_col = self.col;

            let Some(c) = self.peek() else {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: Span::new(self.pos, self.pos, self.line, self.col),
                });
                break;
            };

            let kind = if c.is_ascii_digit() {
                self.read_number()
            } else if c == b'"' {
                match self.read_string() {
                    Ok(k) => k,
                    Err(msg) => {
                        errors.push(LexError {
                            message: msg,
                            span: Span::new(start_pos, self.pos, start_line, start_col),
                        });
                        continue;
                    }
                }
            } else if c.is_ascii_alphabetic() || c == b'_' {
                self.read_identifier_or_keyword()
            } else {
                match self.read_operator_or_punct() {
                    Ok(k) => k,
                    Err(msg) => {
                        errors.push(LexError {
                            message: msg,
                            span: Span::new(start_pos, self.pos, start_line, start_col),
                        });
                        continue;
                    }
                }
            };

            tokens.push(Token {
                kind,
                span: Span::new(start_pos, self.pos, start_line, start_col),
            });
        }

        (tokens, errors)
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<u8> {
        self.source.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        if c == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') => {
                    self.advance();
                }
                Some(b'/') if self.peek_next() == Some(b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn read_number(&mut self) -> TokenKind {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.advance();
        }
        if self.peek() == Some(b'.') && self.peek_next().is_some_and(|c| c.is_ascii_digit()) {
            self.advance();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.advance();
            }
        }
        let text = std::str::from_utf8(&self.source[start..self.pos]).unwrap();
        TokenKind::Number(text.parse().unwrap())
    }

    fn read_string(&mut self) -> Result<TokenKind, String> {
        self.advance(); // opening quote
        let start = self.pos;
        loop {
            match self.peek() {
                None => return Err("unterminated string literal".to_string()),
                Some(b'"') => break,
                _ => {
                    self.advance();
                }
            }
        }
        let text = std::str::from_utf8(&self.source[start..self.pos])
            .unwrap()
            .to_string();
        self.advance(); // closing quote
        Ok(TokenKind::String(text))
    }

    fn read_identifier_or_keyword(&mut self) -> TokenKind {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            self.advance();
        }
        let text = std::str::from_utf8(&self.source[start..self.pos]).unwrap();
        keyword(text).unwrap_or_else(|| TokenKind::Identifier(text.to_string()))
    }

    fn read_operator_or_punct(&mut self) -> Result<TokenKind, String> {
        let c = self.advance().unwrap();
        Ok(match c {
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b',' => TokenKind::Comma,
            b':' => TokenKind::Colon,
            b'+' => TokenKind::Plus,
            b'*' => TokenKind::Star,
            b'/' => TokenKind::Slash,
            b'%' => TokenKind::Percent,
            b'-' => {
                if self.peek() == Some(b'>') {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            b'=' => {
                if self.peek() == Some(b'=') {
                    self.advance();
                    TokenKind::EqualEqual
                } else {
                    TokenKind::Equal
                }
            }
            b'!' => {
                if self.peek() == Some(b'=') {
                    self.advance();
                    TokenKind::BangEqual
                } else {
                    TokenKind::Bang
                }
            }
            b'<' => {
                if self.peek() == Some(b'=') {
                    self.advance();
                    TokenKind::LessEqual
                } else {
                    TokenKind::Less
                }
            }
            b'>' => {
                if self.peek() == Some(b'=') {
                    self.advance();
                    TokenKind::GreaterEqual
                } else {
                    TokenKind::Greater
                }
            }
            other => {
                return Err(format!("unexpected character '{}'", other as char));
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (tokens, errors) = Lexer::new(src).tokenize();
        assert!(errors.is_empty(), "unexpected lex errors: {errors:?}");
        tokens.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn empty_source_is_just_eof() {
        assert_eq!(kinds(""), vec![TokenKind::Eof]);
    }

    #[test]
    fn numbers_integer_and_float() {
        assert_eq!(
            kinds("42 3.14"),
            vec![TokenKind::Number(42.0), TokenKind::Number(3.14), TokenKind::Eof]
        );
    }

    #[test]
    fn string_literal() {
        assert_eq!(
            kinds(r#""hello world""#),
            vec![TokenKind::String("hello world".to_string()), TokenKind::Eof]
        );
    }

    #[test]
    fn unterminated_string_is_reported_not_panicked() {
        let (_, errors) = Lexer::new("\"unterminated").tokenize();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("unterminated"));
    }

    #[test]
    fn keywords_vs_identifiers() {
        assert_eq!(
            kinds("fn let return if else true false foo"),
            vec![
                TokenKind::Fn,
                TokenKind::Let,
                TokenKind::Return,
                TokenKind::If,
                TokenKind::Else,
                TokenKind::True,
                TokenKind::False,
                TokenKind::Identifier("foo".to_string()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn two_char_operators() {
        assert_eq!(
            kinds("-> == != <= >="),
            vec![
                TokenKind::Arrow,
                TokenKind::EqualEqual,
                TokenKind::BangEqual,
                TokenKind::LessEqual,
                TokenKind::GreaterEqual,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            kinds("1 // this is a comment\n2"),
            vec![TokenKind::Number(1.0), TokenKind::Number(2.0), TokenKind::Eof]
        );
    }

    #[test]
    fn function_signature() {
        let src = "fn fib(n: Int) -> Int {";
        assert_eq!(
            kinds(src),
            vec![
                TokenKind::Fn,
                TokenKind::Identifier("fib".to_string()),
                TokenKind::LParen,
                TokenKind::Identifier("n".to_string()),
                TokenKind::Colon,
                TokenKind::Identifier("Int".to_string()),
                TokenKind::RParen,
                TokenKind::Arrow,
                TokenKind::Identifier("Int".to_string()),
                TokenKind::LBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn spans_track_line_and_column() {
        let (tokens, _) = Lexer::new("fn\nfoo").tokenize();
        // "fn" on line 1
        assert_eq!(tokens[0].span.line, 1);
        // "foo" on line 2
        assert_eq!(tokens[1].span.line, 2);
    }

    #[test]
    fn unexpected_character_does_not_panic() {
        let (_, errors) = Lexer::new("@").tokenize();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn fuzz_no_panic_on_random_bytes() {
        // Not a real fuzzer, but guards against panics on arbitrary
        // non-UTF8-adjacent byte sequences during quick local checks.
        let inputs = [
            "\0\0\0",
            "\"\"\"\"\"",
            "----->>>>",
            "1.2.3.4",
            "____",
            "\n\n\n\t\t\t",
        ];
        for input in inputs {
            let _ = Lexer::new(input).tokenize();
        }
    }
}

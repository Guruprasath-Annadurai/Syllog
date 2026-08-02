use crate::ast::*;
use crate::span::Span;
use crate::token::{Token, TokenKind};
use std::mem::discriminant;

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    errors: Vec<ParseError>,
}

type PResult<T> = Result<T, ()>;

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            pos: 0,
            errors: Vec::new(),
        }
    }

    /// Parses the whole token stream, collecting as many errors as
    /// possible instead of stopping at the first one (same philosophy
    /// as the lexer).
    pub fn parse_program(mut self) -> (Program, Vec<ParseError>) {
        let mut items = Vec::new();
        while !self.is_at_end() {
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(()) => self.synchronize_item(),
            }
        }
        (Program { items }, self.errors)
    }

    // ---- token stream helpers ----

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens[self.pos].clone();
        if !self.is_at_end() {
            self.pos += 1;
        }
        tok
    }

    fn check(&self, kind: &TokenKind) -> bool {
        discriminant(&self.peek().kind) == discriminant(kind)
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.errors.push(ParseError {
            message: message.into(),
            span,
        });
    }

    fn expect_kind(&mut self, kind: TokenKind, context: &str) -> PResult<Token> {
        if self.check(&kind) {
            Ok(self.advance())
        } else {
            let tok = self.peek().clone();
            self.error(
                format!("{context}: expected {kind:?}, found {:?}", tok.kind),
                tok.span,
            );
            Err(())
        }
    }

    fn expect_identifier(&mut self, context: &str) -> PResult<String> {
        match &self.peek().kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            other => {
                let span = self.peek().span;
                self.error(format!("{context}: expected identifier, found {other:?}"), span);
                Err(())
            }
        }
    }

    /// Skips tokens until a plausible start of the next top-level item,
    /// so one bad declaration doesn't stop the whole file from parsing.
    fn synchronize_item(&mut self) {
        self.advance();
        while !self.is_at_end() {
            if matches!(self.peek().kind, TokenKind::Fn | TokenKind::Let) {
                return;
            }
            self.advance();
        }
    }

    /// Same idea, but for statements inside a block — stops before a
    /// closing brace so the block parser can still terminate cleanly.
    fn synchronize_stmt(&mut self) {
        self.advance();
        while !self.is_at_end() {
            match self.peek().kind {
                TokenKind::Let
                | TokenKind::Return
                | TokenKind::If
                | TokenKind::LBrace
                | TokenKind::RBrace => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ---- items ----

    fn parse_item(&mut self) -> PResult<Item> {
        match self.peek().kind {
            TokenKind::Fn => self.parse_function().map(Item::Function),
            TokenKind::Let => self.parse_let().map(Item::Let),
            _ => {
                let tok = self.peek().clone();
                self.error(
                    format!("expected 'fn' or 'let' at top level, found {:?}", tok.kind),
                    tok.span,
                );
                Err(())
            }
        }
    }

    fn parse_function(&mut self) -> PResult<FunctionDecl> {
        let span = self.peek().span;
        self.expect_kind(TokenKind::Fn, "function declaration")?;
        let name = self.expect_identifier("function name")?;
        self.expect_kind(TokenKind::LParen, "function parameters")?;

        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let pname = self.expect_identifier("parameter name")?;
                self.expect_kind(TokenKind::Colon, "parameter type annotation")?;
                let ty = self.expect_identifier("parameter type")?;
                params.push(Param { name: pname, ty });
                if self.check(&TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect_kind(TokenKind::RParen, "function parameters")?;

        let return_type = if self.check(&TokenKind::Arrow) {
            self.advance();
            Some(self.expect_identifier("return type")?)
        } else {
            None
        };

        let body = self.parse_block()?;
        Ok(FunctionDecl {
            name,
            params,
            return_type,
            body,
            span,
        })
    }

    fn parse_let(&mut self) -> PResult<LetDecl> {
        let span = self.peek().span;
        self.expect_kind(TokenKind::Let, "let binding")?;
        let name = self.expect_identifier("variable name")?;
        let ty = if self.check(&TokenKind::Colon) {
            self.advance();
            Some(self.expect_identifier("variable type")?)
        } else {
            None
        };
        self.expect_kind(TokenKind::Equal, "let binding")?;
        let value = self.parse_expression()?;
        Ok(LetDecl { name, ty, value, span })
    }

    // ---- statements ----

    fn parse_block(&mut self) -> PResult<Block> {
        self.expect_kind(TokenKind::LBrace, "block")?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            match self.parse_statement() {
                Ok(s) => stmts.push(s),
                Err(()) => self.synchronize_stmt(),
            }
        }
        self.expect_kind(TokenKind::RBrace, "block")?;
        Ok(stmts)
    }

    fn parse_statement(&mut self) -> PResult<Stmt> {
        match self.peek().kind {
            TokenKind::Let => self.parse_let().map(Stmt::Let),
            TokenKind::Return => self.parse_return(),
            TokenKind::If => self.parse_if().map(Stmt::If),
            TokenKind::LBrace => self.parse_block().map(Stmt::Block),
            _ => self.parse_expression().map(Stmt::Expr),
        }
    }

    fn parse_return(&mut self) -> PResult<Stmt> {
        let span = self.peek().span;
        self.expect_kind(TokenKind::Return, "return statement")?;
        if self.check(&TokenKind::RBrace) {
            Ok(Stmt::Return(None, span))
        } else {
            let expr = self.parse_expression()?;
            Ok(Stmt::Return(Some(expr), span))
        }
    }

    fn parse_if(&mut self) -> PResult<IfStmt> {
        let span = self.peek().span;
        self.expect_kind(TokenKind::If, "if statement")?;
        let cond = self.parse_expression()?;
        let then_block = self.parse_block()?;
        let else_branch = if self.check(&TokenKind::Else) {
            self.advance();
            if self.check(&TokenKind::If) {
                Some(ElseBranch::If(Box::new(self.parse_if()?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            None
        };
        Ok(IfStmt {
            cond,
            then_block,
            else_branch,
            span,
        })
    }

    // ---- expressions (precedence climbing) ----

    fn parse_expression(&mut self) -> PResult<Expr> {
        self.parse_equality()
    }

    fn parse_equality(&mut self) -> PResult<Expr> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::EqualEqual => BinaryOp::Eq,
                TokenKind::BangEqual => BinaryOp::NotEq,
                _ => break,
            };
            let op_span = self.advance().span;
            let right = self.parse_comparison()?;
            left = combine(left, op, right, op_span);
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> PResult<Expr> {
        let mut left = self.parse_term()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Less => BinaryOp::Lt,
                TokenKind::LessEqual => BinaryOp::LtEq,
                TokenKind::Greater => BinaryOp::Gt,
                TokenKind::GreaterEqual => BinaryOp::GtEq,
                _ => break,
            };
            let op_span = self.advance().span;
            let right = self.parse_term()?;
            left = combine(left, op, right, op_span);
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> PResult<Expr> {
        let mut left = self.parse_factor()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            let op_span = self.advance().span;
            let right = self.parse_factor()?;
            left = combine(left, op, right, op_span);
        }
        Ok(left)
    }

    fn parse_factor(&mut self) -> PResult<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek().kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            let op_span = self.advance().span;
            let right = self.parse_unary()?;
            left = combine(left, op, right, op_span);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let op = match self.peek().kind {
            TokenKind::Bang => Some(UnaryOp::Not),
            TokenKind::Minus => Some(UnaryOp::Neg),
            _ => None,
        };
        if let Some(op) = op {
            let op_span = self.advance().span;
            let operand = self.parse_unary()?;
            let span = Span::new(op_span.start, operand.span().end, op_span.line, op_span.col);
            return Ok(Expr::Unary {
                op,
                operand: Box::new(operand),
                span,
            });
        }
        self.parse_call()
    }

    fn parse_call(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.check(&TokenKind::LParen) {
                self.advance();
                let mut args = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    loop {
                        args.push(self.parse_expression()?);
                        if self.check(&TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                let end_tok = self.expect_kind(TokenKind::RParen, "call arguments")?;
                let span = Span::new(
                    expr.span().start,
                    end_tok.span.end,
                    expr.span().line,
                    expr.span().col,
                );
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                    span,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Number(n) => {
                self.advance();
                Ok(Expr::Number(n, tok.span))
            }
            TokenKind::String(s) => {
                self.advance();
                Ok(Expr::String(s, tok.span))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Bool(true, tok.span))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Bool(false, tok.span))
            }
            TokenKind::Identifier(name) => {
                self.advance();
                Ok(Expr::Identifier(name, tok.span))
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect_kind(TokenKind::RParen, "parenthesized expression")?;
                Ok(expr)
            }
            other => {
                self.error(format!("expected an expression, found {other:?}"), tok.span);
                Err(())
            }
        }
    }
}

fn combine(left: Expr, op: BinaryOp, right: Expr, op_span: Span) -> Expr {
    let span = Span::new(
        left.span().start,
        right.span().end,
        left.span().line,
        left.span().col,
    );
    let _ = op_span;
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;

    fn parse(src: &str) -> (Program, Vec<ParseError>) {
        let (tokens, lex_errors) = Lexer::new(src).tokenize();
        assert!(lex_errors.is_empty(), "unexpected lex errors: {lex_errors:?}");
        Parser::new(tokens).parse_program()
    }

    #[test]
    fn parses_empty_function() {
        let (program, errors) = parse("fn main() { }");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Function(f) => {
                assert_eq!(f.name, "main");
                assert!(f.params.is_empty());
                assert!(f.return_type.is_none());
                assert!(f.body.is_empty());
            }
            _ => panic!("expected function"),
        }
    }

    #[test]
    fn parses_params_and_return_type() {
        let (program, errors) = parse("fn add(a: Int, b: Int) -> Int { return a + b }");
        assert!(errors.is_empty(), "{errors:?}");
        let Item::Function(f) = &program.items[0] else {
            panic!("expected function")
        };
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "a");
        assert_eq!(f.params[0].ty, "Int");
        assert_eq!(f.return_type.as_deref(), Some("Int"));
        assert_eq!(f.body.len(), 1);
        assert!(matches!(f.body[0], Stmt::Return(Some(_), _)));
    }

    #[test]
    fn binary_operator_precedence() {
        // 1 + 2 * 3 should parse as 1 + (2 * 3)
        let (program, errors) = parse("let x = 1 + 2 * 3");
        assert!(errors.is_empty(), "{errors:?}");
        let Item::Let(decl) = &program.items[0] else {
            panic!("expected let")
        };
        match &decl.value {
            Expr::Binary { op: BinaryOp::Add, left, right, .. } => {
                assert!(matches!(**left, Expr::Number(1.0, _)));
                match &**right {
                    Expr::Binary { op: BinaryOp::Mul, .. } => {}
                    other => panic!("expected multiplication on the right, got {other:?}"),
                }
            }
            other => panic!("expected top-level addition, got {other:?}"),
        }
    }

    #[test]
    fn comparison_binds_looser_than_arithmetic() {
        let (program, errors) = parse("let x = 1 + 2 < 4 - 1");
        assert!(errors.is_empty(), "{errors:?}");
        let Item::Let(decl) = &program.items[0] else {
            panic!("expected let")
        };
        assert!(matches!(
            decl.value,
            Expr::Binary { op: BinaryOp::Lt, .. }
        ));
    }

    #[test]
    fn parses_function_calls_with_args() {
        let (program, errors) = parse("let x = fib(n - 1)");
        assert!(errors.is_empty(), "{errors:?}");
        let Item::Let(decl) = &program.items[0] else {
            panic!("expected let")
        };
        match &decl.value {
            Expr::Call { callee, args, .. } => {
                assert!(matches!(**callee, Expr::Identifier(ref n, _) if n == "fib"));
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected call, got {other:?}"),
        }
    }

    #[test]
    fn parses_if_else_if_chain() {
        let src = "fn f() { if a { return 1 } else if b { return 2 } else { return 3 } }";
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "{errors:?}");
        let Item::Function(f) = &program.items[0] else {
            panic!("expected function")
        };
        let Stmt::If(if_stmt) = &f.body[0] else {
            panic!("expected if statement")
        };
        assert!(matches!(if_stmt.else_branch, Some(ElseBranch::If(_))));
    }

    #[test]
    fn parses_full_fib_example() {
        let src = r#"
            fn fib(n: Int) -> Int {
                if n < 2 {
                    return n
                }
                return fib(n - 1) + fib(n - 2)
            }

            fn main() {
                let result = fib(10)
                print(result)
            }
        "#;
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(program.items.len(), 2);
    }

    #[test]
    fn unary_operators() {
        let (program, errors) = parse("let x = -5");
        assert!(errors.is_empty(), "{errors:?}");
        let Item::Let(decl) = &program.items[0] else {
            panic!("expected let")
        };
        assert!(matches!(
            decl.value,
            Expr::Unary { op: UnaryOp::Neg, .. }
        ));
    }

    #[test]
    fn reports_error_without_panicking_on_missing_paren() {
        let (_, errors) = parse("fn f( { }");
        assert!(!errors.is_empty());
    }

    #[test]
    fn recovers_and_reports_multiple_errors_across_items() {
        // First function is broken (missing return type after '->'),
        // second function is fine — parser should still find it and
        // report only the first error, not cascade into false ones.
        let src = "fn broken() -> { } fn ok() { let x = 1 }";
        let (program, errors) = parse(src);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            program.items.iter().any(|i| matches!(i, Item::Function(f) if f.name == "ok")),
            "parser should recover and still find the second function"
        );
    }

    #[test]
    fn fuzz_no_panic_on_malformed_input() {
        let inputs = [
            "fn",
            "fn (",
            "let",
            "let x =",
            "((((",
            "))))",
            "fn f() { if }",
            "fn f() { return return return }",
            "1 + + + 2",
        ];
        for src in inputs {
            let (tokens, _) = Lexer::new(src).tokenize();
            let _ = Parser::new(tokens).parse_program();
        }
    }
}

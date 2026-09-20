//! `serve on port ... { route ... }`.

use crate::ast::{
    ExpectingDecl, Name, ReplyKind, ReplyStmt, RouteDecl, RouteSegment, ServeDecl,
    ServeErrorHandler, TypeExpr, TypeKind,
};
use crate::diagnostic::Diagnostic;
use crate::span::Span;
use crate::token::TokenKind;

use super::{Parse, Parser};

impl<'src> Parser<'src> {
    pub(crate) fn serve_declaration(&mut self) -> Parse<ServeDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Serve, "the word `serve`")?;
        self.expect_literal("on", "the word `on` after `serve`")?;
        self.expect(TokenKind::Port, "the word `port`")?;
        let port = self.expression()?;
        self.expect(TokenKind::OpenBrace, "a `{` to open the server block")?;

        let mut before = None;
        let mut routes = Vec::new();
        let mut error_handler = None;

        while !self.check(TokenKind::CloseBrace) && !self.at_end() {
            self.skip_newlines();
            if self.check(TokenKind::CloseBrace) || self.at_end() {
                break;
            }
            if self.check(TokenKind::Route) {
                routes.push(self.route_declaration()?);
            } else if self.check(TokenKind::Before) {
                before = Some(self.before_hook()?);
            } else if self.check(TokenKind::When) {
                error_handler = Some(self.serve_error_handler()?);
            } else {
                self.report(
                    Diagnostic::error(
                        "unexpected-in-serve",
                        "only `before`, `route` and `when anything fails` belong inside a `serve` block",
                    )
                    .at(self.current().span, "this does not belong here"),
                );
                self.recover_to_statement_boundary();
            }
        }

        self.expect(TokenKind::CloseBrace, "a `}` to close the server block")?;
        Ok(ServeDecl {
            port,
            before,
            routes,
            error_handler,
            span: start.to(self.previous_span()),
        })
    }

    fn before_hook(&mut self) -> Parse<crate::ast::Block> {
        self.expect(TokenKind::Before, "the word `before`")?;
        self.expect(TokenKind::Every, "the word `every`")?;
        self.expect_literal("request", "the word `request`")?;
        self.block()
    }

    fn serve_error_handler(&mut self) -> Parse<ServeErrorHandler> {
        self.expect(TokenKind::When, "the word `when`")?;
        self.expect(TokenKind::Anything, "the word `anything`")?;
        self.expect(TokenKind::Fails, "the word `fails`")?;
        self.expect(TokenKind::With, "the word `with`")?;
        let error_type = self.type_expression()?;
        self.expect(TokenKind::As, "the word `as`")?;
        let binding = self.name("a name for the error")?;
        Ok(ServeErrorHandler { error_type, binding, body: self.block()? })
    }

    fn route_declaration(&mut self) -> Parse<RouteDecl> {
        let start = self.current().span;
        self.expect(TokenKind::Route, "the word `route`")?;
        let method = self.method_name()?;
        let path = self.route_path()?;
        let expecting = if self.eat(TokenKind::Expecting) {
            Some(self.expecting_clause()?)
        } else {
            None
        };
        Ok(RouteDecl {
            method,
            path,
            expecting,
            body: self.block()?,
            span: start.to(self.previous_span()),
        })
    }

    fn method_name(&mut self) -> Parse<Name> {
        let token = self.current();
        if token.kind != TokenKind::Identifier {
            return Err(self.unexpected("an HTTP method such as `get` or `post`"));
        }
        self.advance();
        Ok(Name::new(token.text(self.source()), token.span))
    }

    fn route_path(&mut self) -> Parse<Vec<RouteSegment>> {
        let token = self.current();
        if token.kind != TokenKind::Text {
            return Err(self.unexpected("a path in double quotes"));
        }
        self.advance();
        Ok(parse_route_path(token.text(self.source()), token.span))
    }

    fn expecting_clause(&mut self) -> Parse<ExpectingDecl> {
        Ok(ExpectingDecl {
            declared: self.type_expression()?,
            binding: {
                self.expect(TokenKind::As, "the word `as`")?;
                self.name("a name for the request body")?
            },
        })
    }

    pub(crate) fn reply_statement(&mut self) -> Parse<ReplyStmt> {
        let start = self.current().span;
        self.expect(TokenKind::Reply, "the word `reply`")?;
        let kind = if self.eat(TokenKind::With) {
            let value = self.expression()?;
            let status = if self.eat(TokenKind::Status) {
                Some(self.expression()?)
            } else {
                None
            };
            ReplyKind::With { value, status }
        } else if self.eat(TokenKind::Explain) {
            ReplyKind::Explain(self.expression()?)
        } else {
            return Err(self.unexpected("`reply with value` or `reply explain error`"));
        };
        Ok(ReplyStmt { kind, span: start.to(self.previous_span()) })
    }

    fn expect_literal(&mut self, word: &str, expectation: &str) -> Parse<()> {
        let token = self.current();
        if token.kind == TokenKind::Identifier && token.text(self.source()) == word {
            self.advance();
            Ok(())
        } else {
            Err(self.unexpected(expectation))
        }
    }
}

fn parse_route_path(text: &str, span: Span) -> Vec<RouteSegment> {
    let inner = text.trim_matches('"');
    let mut segments = Vec::new();
    let mut rest = inner;
    while !rest.is_empty() {
        if rest.starts_with('{') {
            let end = rest.find('}').map_or(rest.len(), |index| index);
            let inside = &rest[1..end];
            let (name, declared) = if let Some((name, type_text)) = inside.split_once(':') {
                (
                    Name::new(name.trim(), span),
                    Some(TypeExpr {
                        kind: TypeKind::Named(Name::new(type_text.trim(), span)),
                        span,
                    }),
                )
            } else {
                (Name::new(inside.trim(), span), None)
            };
            segments.push(RouteSegment::Param { name, declared });
            rest = &rest[end.saturating_add(1)..];
        } else {
            let end = rest.find('{').map_or(rest.len(), |index| index);
            segments.push(RouteSegment::Literal(rest[..end].to_string()));
            rest = &rest[end..];
        }
    }
    segments
}

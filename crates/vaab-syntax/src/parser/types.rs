//! Types as they are written in the source.
//!
//! Vaab spells its types in words: `list of Text`, `map of Text to Int`,
//! `maybe Int`, `to(Int) returns Text`. The words involved (`list`, `map`, `of`,
//! `maybe`, …) are soft keywords, so they are only read this way here, inside a
//! type; everywhere else they remain ordinary names.

use crate::ast::{TypeExpr, TypeKind};
use crate::diagnostic::Diagnostic;
use crate::token::TokenKind;

use super::{Parse, Parser};

impl<'src> Parser<'src> {
    /// A complete type, including a trailing `or fails E`.
    pub(crate) fn type_expression(&mut self) -> Parse<TypeExpr> {
        self.enter()?;
        let result = self.type_expression_body();
        self.leave();
        result
    }

    fn type_expression_body(&mut self) -> Parse<TypeExpr> {
        let ok = self.type_term()?;

        // `T or fails E`. `or` is also the boolean operator, but a type is never
        // an expression, so there is nothing to confuse it with here.
        if self.check(TokenKind::Or) && self.peek_at(1) == TokenKind::Fails {
            // Step over `or` and `fails`.
            self.advance();
            self.advance();
            let error = self.type_term()?;
            let span = ok.span.to(error.span);
            return Ok(TypeExpr {
                kind: TypeKind::Fallible { ok: Box::new(ok), error: Box::new(error) },
                span,
            });
        }

        // A lone `or` in a type is almost always a half-written `or fails`.
        if self.check(TokenKind::Or) {
            let span = self.current().span;
            self.report(
                Diagnostic::error("incomplete-or-fails", "`or` on its own is not a type")
                    .at(span, "Vaab expected `or fails` and then an error type")
                    .with_help("write `or fails SomeError` to say how this can fail")
                    .with_note("Vaab has no union types; a value that may fail is `T or fails E`"),
            );
            return Err(super::Failed);
        }

        Ok(ok)
    }

    /// A single type, with no trailing `or fails`.
    fn type_term(&mut self) -> Parse<TypeExpr> {
        use TokenKind::*;
        let start = self.current().span;

        match self.peek() {
            List => {
                self.advance();
                self.expect_word(Of, "the word `of`, as in `list of Text`")?;
                let item = self.type_term()?;
                Ok(TypeExpr { span: start.to(item.span), kind: TypeKind::List(Box::new(item)) })
            }
            Map => {
                self.advance();
                self.expect_word(Of, "the word `of`, as in `map of Text to Int`")?;
                let key = self.type_term()?;
                // Here `to` is a connector, not the start of a function.
                self.expect(To, "the word `to`, as in `map of Text to Int`")?;
                let value = self.type_term()?;
                Ok(TypeExpr {
                    span: start.to(value.span),
                    kind: TypeKind::Map { key: Box::new(key), value: Box::new(value) },
                })
            }
            Maybe => {
                self.advance();
                let item = self.type_term()?;
                Ok(TypeExpr { span: start.to(item.span), kind: TypeKind::Maybe(Box::new(item)) })
            }
            Channel => {
                self.advance();
                self.expect_word(Of, "the word `of`, as in `channel of Text`")?;
                let item = self.type_term()?;
                Ok(TypeExpr { span: start.to(item.span), kind: TypeKind::Channel(Box::new(item)) })
            }
            Task => {
                self.advance();
                self.expect_word(Of, "the word `of`, as in `task of Int`")?;
                let item = self.type_term()?;
                Ok(TypeExpr { span: start.to(item.span), kind: TypeKind::Task(Box::new(item)) })
            }
            Shared => {
                self.advance();
                let item = self.type_term()?;
                Ok(TypeExpr { span: start.to(item.span), kind: TypeKind::Shared(Box::new(item)) })
            }
            To => self.function_type(),
            OpenParen => self.tuple_type(),
            _ => {
                let name = self.name("a type")?;
                let span = name.span;
                Ok(TypeExpr { kind: TypeKind::Named(name), span })
            }
        }
    }

    /// `to(Int, Text) returns Bool`
    fn function_type(&mut self) -> Parse<TypeExpr> {
        let start = self.expect(TokenKind::To, "the word `to`")?.span;
        self.expect(TokenKind::OpenParen, "a `(` listing what this function takes")?;

        let mut parameters = Vec::new();
        let close = self.with_braces_as_literals(|parser| {
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseParen) {
                parameters.push(parser.type_expression()?);
                parser.skip_newlines();
                if !parser.eat(TokenKind::Comma) {
                    break;
                }
                parser.skip_newlines();
            }
            parser.expect(TokenKind::CloseParen, "a `)` to close this function type")
        })?;

        // `to(Int) returns Text or fails E` reads the whole result, including the
        // failure, as the function's result.
        let (returns, end) = if self.eat(TokenKind::Returns) {
            let result = self.type_expression()?;
            let span = result.span;
            (Some(Box::new(result)), span)
        } else {
            (None, close.span)
        };

        Ok(TypeExpr { kind: TypeKind::Function { parameters, returns }, span: start.to(end) })
    }

    /// `(Int, Text)`
    fn tuple_type(&mut self) -> Parse<TypeExpr> {
        let open = self.expect(TokenKind::OpenParen, "a `(`")?;
        let mut items = Vec::new();

        let close = self.with_braces_as_literals(|parser| {
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseParen) {
                items.push(parser.type_expression()?);
                parser.skip_newlines();
                if !parser.eat(TokenKind::Comma) {
                    break;
                }
                parser.skip_newlines();
            }
            parser.expect(TokenKind::CloseParen, "a `)` to close this tuple type")
        })?;

        let span = open.span.to(close.span);
        if items.len() == 1 {
            // `(Int)` is just `Int` written with brackets round it.
            let mut only = items.remove(0);
            only.span = span;
            return Ok(only);
        }

        Ok(TypeExpr { kind: TypeKind::Tuple(items), span })
    }
}

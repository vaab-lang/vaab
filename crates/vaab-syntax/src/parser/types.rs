//! Types as they are written in the source.
//!
//! Vaab spells its types in words: `list of Text`, `map of Text to Int`,
//! `maybe Int`, `(Int) returns Text`. Sugar: `[Text]` and `{Text: Int}`.
//! Soft keywords (`list`, `map`, `of`, `maybe`, …) are only read this way here.

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

        if self.check(TokenKind::Or) && self.peek_at(1) == TokenKind::Fails {
            self.advance();
            self.advance();
            let error = self.type_term()?;
            let span = ok.span.to(error.span);
            return Ok(TypeExpr {
                kind: TypeKind::Fallible { ok: Box::new(ok), error: Box::new(error) },
                span,
            });
        }

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
            // Legacy `to(Int) returns Text`; prefer `(Int) returns Text`.
            To => self.function_type_with_to(),
            OpenParen => self.paren_type(),
            // Sugar: `[Text]` for `list of Text`.
            OpenBracket => {
                self.advance();
                let item = self.type_expression()?;
                let close = self.expect(CloseBracket, "a `]` to close this list type")?;
                Ok(TypeExpr {
                    span: start.to(close.span),
                    kind: TypeKind::List(Box::new(item)),
                })
            }
            // Sugar: `{Text: Int}` for `map of Text to Int`.
            OpenBrace => {
                self.advance();
                let key = self.type_expression()?;
                self.expect(Colon, "a `:` between the key and value types, as in `{Text: Int}`")?;
                let value = self.type_expression()?;
                let close = self.expect(CloseBrace, "a `}` to close this map type")?;
                Ok(TypeExpr {
                    span: start.to(close.span),
                    kind: TypeKind::Map { key: Box::new(key), value: Box::new(value) },
                })
            }
            _ => {
                let name = self.name("a type")?;
                let span = name.span;
                Ok(TypeExpr { kind: TypeKind::Named(name), span })
            }
        }
    }

    /// Legacy `to(Int, Text) returns Bool`.
    fn function_type_with_to(&mut self) -> Parse<TypeExpr> {
        let start = self.expect(TokenKind::To, "the word `to`")?.span;
        self.expect(TokenKind::OpenParen, "a `(` listing what this function takes")?;
        let (parameters, close) = self.function_type_params()?;
        self.finish_function_type(start, parameters, close)
    }

    /// `(Int, Text) returns Bool`, `(Int)` grouping, or `(Int, Text)` tuple.
    fn paren_type(&mut self) -> Parse<TypeExpr> {
        let open = self.expect(TokenKind::OpenParen, "a `(`")?;
        let (parameters, close) = self.function_type_params()?;

        // `(Int) returns Text` / `() returns Text` / `(Int, Text) returns Bool`
        if self.check(TokenKind::Returns) {
            return self.finish_function_type(open.span, parameters, close);
        }

        let span = open.span.to(close.span);
        if parameters.len() == 1 {
            let mut only = parameters.into_iter().next().unwrap();
            only.span = span;
            return Ok(only);
        }

        Ok(TypeExpr { kind: TypeKind::Tuple(parameters), span })
    }

    fn function_type_params(&mut self) -> Parse<(Vec<TypeExpr>, crate::token::Token)> {
        self.with_braces_as_literals(|parser| {
            let mut parameters = Vec::new();
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseParen) {
                parameters.push(parser.type_expression()?);
                parser.skip_newlines();
                if !parser.eat(TokenKind::Comma) {
                    break;
                }
                parser.skip_newlines();
            }
            let close =
                parser.expect(TokenKind::CloseParen, "a `)` to close this type")?;
            Ok((parameters, close))
        })
    }

    fn finish_function_type(
        &mut self,
        start: crate::span::Span,
        parameters: Vec<TypeExpr>,
        close: crate::token::Token,
    ) -> Parse<TypeExpr> {
        let (returns, end) = if self.eat(TokenKind::Returns) {
            let result = self.type_expression()?;
            let span = result.span;
            (Some(Box::new(result)), span)
        } else {
            (None, close.span)
        };

        Ok(TypeExpr {
            kind: TypeKind::Function { parameters, returns },
            span: start.to(end),
        })
    }
}

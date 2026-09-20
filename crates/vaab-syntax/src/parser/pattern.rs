//! Patterns, as used by `match` arms and `for each`.
//!
//! A bare word is either a new binding or a choice variant. Vaab decides by
//! capitalisation, the same rule the language already uses to tell types from
//! values: `when frozen` binds everything to `frozen`, while `when Frozen` matches
//! the variant of that name. This is worth knowing, because getting it wrong
//! silently turns an exhaustive match into a catch-all in most languages that
//! allow it.

use crate::ast::{Pattern, PatternKind, TextPart};
use crate::diagnostic::Diagnostic;
use crate::lexer::strip_digit_separators;
use crate::span::Span;
use crate::token::TokenKind;

use super::{Failed, Parse, Parser};

impl<'src> Parser<'src> {
    pub(crate) fn pattern(&mut self) -> Parse<Pattern> {
        self.enter()?;
        let result = self.pattern_body();
        self.leave();
        result
    }

    fn pattern_body(&mut self) -> Parse<Pattern> {
        use TokenKind::*;
        let start = self.current().span;

        match self.peek() {
            Minus => {
                self.advance();
                let inner = self.pattern_body()?;
                let span = start.to(inner.span);
                match inner.kind {
                    PatternKind::Int(value) => {
                        Ok(Pattern { kind: PatternKind::Int(-value), span })
                    }
                    PatternKind::Float(value) => {
                        Ok(Pattern { kind: PatternKind::Float(-value), span })
                    }
                    _ => {
                        self.report(
                            Diagnostic::error("bad-pattern", "`-` can only precede a number here")
                                .at(inner.span, "a pattern matches a value, it does not compute one")
                                .with_help("match on the number itself, or use a guard: `when n if -n > 0`"),
                        );
                        Err(Failed)
                    }
                }
            }
            Int => {
                let span = self.advance().span;
                let text = strip_digit_separators(span.slice(self.source()));
                match text.parse::<i64>() {
                    Ok(value) => Ok(Pattern { kind: PatternKind::Int(value), span }),
                    Err(_) => {
                        self.report(
                            Diagnostic::error(
                                "number-too-large",
                                "this number is too large for an Int",
                            )
                            .at(span, "Vaab could not hold this value")
                            .with_note(format!("the largest Int is {}", i64::MAX)),
                        );
                        Err(Failed)
                    }
                }
            }
            Float => {
                let span = self.advance().span;
                let text = strip_digit_separators(span.slice(self.source()));
                match text.parse::<f64>() {
                    Ok(value) => Ok(Pattern { kind: PatternKind::Float(value), span }),
                    Err(_) => Err(self.unexpected("a number Vaab can hold")),
                }
            }
            Text => {
                let span = self.advance().span;
                let parts = self.text_parts(span)?;
                let text = literal_text(&parts).ok_or_else(|| {
                    self.report(
                        Diagnostic::error(
                            "interpolation-in-pattern",
                            "a pattern cannot contain `{...}`",
                        )
                        .at(span, "this text is built while the program runs")
                        .with_help("match a fixed piece of text, or bind a name and compare inside the arm"),
                    );
                    Failed
                })?;
                Ok(Pattern { kind: PatternKind::Text(text), span })
            }
            Yes => {
                let span = self.advance().span;
                Ok(Pattern { kind: PatternKind::Bool(true), span })
            }
            No => {
                let span = self.advance().span;
                Ok(Pattern { kind: PatternKind::Bool(false), span })
            }
            Nothing => {
                let span = self.advance().span;
                Ok(Pattern { kind: PatternKind::Nothing, span })
            }
            Found => self.wrapping_pattern(PatternKind::Found, "found"),
            Success => self.wrapping_pattern(PatternKind::Success, "success"),
            Failure => self.wrapping_pattern(PatternKind::Failure, "failure"),
            OpenBracket => self.list_pattern(),
            OpenParen => self.tuple_pattern(),
            _ => self.name_pattern(),
        }
    }

    /// `found x`, `success x`, `failure e`
    fn wrapping_pattern(
        &mut self,
        build: fn(Box<Pattern>) -> PatternKind,
        word: &str,
    ) -> Parse<Pattern> {
        let start = self.advance().span;
        if self.pattern_has_ended() {
            self.report(
                Diagnostic::error("bad-pattern", format!("`{word}` needs something after it"))
                    .at(Span::empty_at(start.end), "Vaab expected a name or a value here")
                    .with_help(format!("write `{word} value` to name what is inside")),
            );
            return Err(Failed);
        }
        let inner = self.pattern_body()?;
        let span = start.to(inner.span);
        Ok(Pattern { kind: build(Box::new(inner)), span })
    }

    /// Whether the pattern is finished, so that `found` with nothing after it can
    /// be reported as such rather than dragged into the next word.
    fn pattern_has_ended(&self) -> bool {
        use TokenKind::*;
        matches!(
            self.peek(),
            Then | If | In | Comma | CloseParen | CloseBracket | Newline | EndOfFile
        )
    }

    /// `[first, second]` or `[first, ...]`
    fn list_pattern(&mut self) -> Parse<Pattern> {
        let open = self.expect(TokenKind::OpenBracket, "a `[`")?;
        let mut elements = Vec::new();
        let mut rest = false;

        self.skip_newlines();
        while !self.check(TokenKind::CloseBracket) {
            if self.check(TokenKind::Ellipsis) {
                let span = self.advance().span;
                rest = true;
                self.skip_newlines();
                if self.check(TokenKind::Comma) {
                    self.report(
                        Diagnostic::error("rest-not-last", "`...` must come last")
                            .at(span, "this stands for everything that is left")
                            .with_help("move `...` to the end of the pattern"),
                    );
                    return Err(Failed);
                }
                break;
            }

            elements.push(self.pattern()?);
            self.skip_newlines();
            if !self.eat(TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
        }

        let close = self.expect(TokenKind::CloseBracket, "a `]` to close this pattern")?;
        Ok(Pattern {
            kind: PatternKind::List { elements, rest },
            span: open.span.to(close.span),
        })
    }

    /// `(a, b)`
    fn tuple_pattern(&mut self) -> Parse<Pattern> {
        let open = self.expect(TokenKind::OpenParen, "a `(`")?;
        let mut items = Vec::new();

        self.skip_newlines();
        while !self.check(TokenKind::CloseParen) {
            items.push(self.pattern()?);
            self.skip_newlines();
            if !self.eat(TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
        }

        let close = self.expect(TokenKind::CloseParen, "a `)` to close this pattern")?;
        Ok(Pattern { kind: PatternKind::Tuple(items), span: open.span.to(close.span) })
    }

    /// A bare word: a new binding, or a choice variant, possibly with a payload.
    fn name_pattern(&mut self) -> Parse<Pattern> {
        let first = self.name("a pattern")?;

        let is_variant = first.looks_like_a_type() || self.check(TokenKind::Dot);
        if !is_variant {
            let span = first.span;
            return Ok(Pattern { kind: PatternKind::Binding(first), span });
        }

        let mut path = vec![first];
        while self.eat(TokenKind::Dot) {
            path.push(self.any_word_as_name("a variant name after `.`")?);
        }

        let mut fields = Vec::new();
        let mut end = path.last().map(|name| name.span).unwrap_or_default();

        if self.check(TokenKind::OpenParen) {
            self.advance();
            self.skip_newlines();
            while !self.check(TokenKind::CloseParen) {
                fields.push(self.pattern()?);
                self.skip_newlines();
                if !self.eat(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
            end = self
                .expect(TokenKind::CloseParen, "a `)` to close this variant's parts")?
                .span;
        }

        let span = path.first().map(|name| name.span).unwrap_or(end).to(end);
        Ok(Pattern { kind: PatternKind::Variant { path, fields }, span })
    }
}

/// The text of a string that has no `{...}` holes in it.
fn literal_text(parts: &[TextPart]) -> Option<String> {
    let mut text = String::new();
    for part in parts {
        match part {
            TextPart::Literal(piece) => text.push_str(piece),
            TextPart::Interpolation(_) => return None,
        }
    }
    Some(text)
}

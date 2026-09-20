//! Expressions.
//!
//! Precedence is handled by Pratt parsing: every infix operator has a left and a
//! right binding power, and [`Parser::expression_from`] keeps consuming operators
//! while they bind at least as tightly as its caller allows.

use crate::ast::{
    Argument, BinaryOp, Block, ElseBranch, Expr, ExprKind, FunctionBody, IfExpr, MapEntry,
    MatchArm, MatchExpr, Name, SelectArm, SelectExpr, UnaryOp,
};
use crate::diagnostic::Diagnostic;
use crate::lexer::strip_digit_separators;
use crate::span::Span;
use crate::token::TokenKind;

use super::{Failed, Parse, Parser};

// Binding powers. Higher numbers bind more tightly. Left-associative operators
// give their right side `power + 1` so an equal operator stops the recursion.
//
//   arrow        n -> body          loosest: a closure body swallows the rest
//   otherwise    a otherwise b
//   or / and
//   comparison   a == b             does not chain
//   range        1..10              does not chain
//   + -
//   * / %
//   prefix, then calls, fields and indexing
const POWER_ARROW: u8 = 1;
const POWER_OTHERWISE: u8 = 2;
const POWER_OR: u8 = 4;
const POWER_AND: u8 = 6;
const POWER_COMPARISON: u8 = 8;
const POWER_RANGE: u8 = 10;
const POWER_SUM: u8 = 12;
const POWER_PRODUCT: u8 = 14;

/// The operator an infix token stands for, and how tightly it binds.
fn infix_operator(kind: TokenKind) -> Option<(BinaryOp, u8, u8)> {
    use TokenKind::*;
    let (operator, power) = match kind {
        Or => (BinaryOp::Or, POWER_OR),
        And => (BinaryOp::And, POWER_AND),
        EqualsEquals => (BinaryOp::Equals, POWER_COMPARISON),
        NotEquals => (BinaryOp::NotEquals, POWER_COMPARISON),
        Less => (BinaryOp::Less, POWER_COMPARISON),
        LessEquals => (BinaryOp::LessOrEqual, POWER_COMPARISON),
        Greater => (BinaryOp::Greater, POWER_COMPARISON),
        GreaterEquals => (BinaryOp::GreaterOrEqual, POWER_COMPARISON),
        Plus => (BinaryOp::Add, POWER_SUM),
        Minus => (BinaryOp::Subtract, POWER_SUM),
        Star => (BinaryOp::Multiply, POWER_PRODUCT),
        Slash => (BinaryOp::Divide, POWER_PRODUCT),
        Percent => (BinaryOp::Remainder, POWER_PRODUCT),
        _ => return None,
    };
    Some((operator, power, power + 1))
}

fn is_comparison(operator: BinaryOp) -> bool {
    matches!(
        operator,
        BinaryOp::Equals
            | BinaryOp::NotEquals
            | BinaryOp::Less
            | BinaryOp::LessOrEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterOrEqual
    )
}

impl<'src> Parser<'src> {
    /// Parses a complete expression.
    pub(crate) fn expression(&mut self) -> Parse<Expr> {
        self.expression_from(0)
    }

    /// Parses an expression, stopping at the first operator that binds more loosely
    /// than `minimum_power`.
    fn expression_from(&mut self, minimum_power: u8) -> Parse<Expr> {
        self.enter()?;
        let result = self.expression_body(minimum_power);
        self.leave();
        result
    }

    fn expression_body(&mut self, minimum_power: u8) -> Parse<Expr> {
        let mut left = self.prefix_expression()?;
        // Remembers the operator that produced `left`, so that `a < b < c` can be
        // rejected with a message about chaining rather than a type error later.
        let mut previous_operator: Option<BinaryOp> = None;

        loop {
            match self.peek() {
                // `->` turns whatever came before it into a closure's parameters.
                TokenKind::Arrow if POWER_ARROW >= minimum_power => {
                    left = self.closure(left)?;
                    previous_operator = None;
                }
                // `otherwise` is an operator here and a catch-all arm inside
                // `match` and `select`; the two never meet, because an arm always
                // begins a line.
                TokenKind::Otherwise if POWER_OTHERWISE >= minimum_power => {
                    self.advance();
                    let fallback = self.expression_from(POWER_OTHERWISE + 1)?;
                    let span = left.span.to(fallback.span);
                    left = self.expression_node(
                        ExprKind::Otherwise {
                            value: Box::new(left),
                            fallback: Box::new(fallback),
                        },
                        span,
                    );
                    previous_operator = None;
                }
                TokenKind::DotDot if POWER_RANGE >= minimum_power => {
                    let operator_span = self.advance().span;
                    if matches!(left.kind, ExprKind::Range { .. }) {
                        self.report(range_cannot_chain(operator_span));
                        return Err(Failed);
                    }
                    let end = self.expression_from(POWER_RANGE + 1)?;
                    let span = left.span.to(end.span);
                    left = self.expression_node(
                        ExprKind::Range { start: Box::new(left), end: Box::new(end) },
                        span,
                    );
                    previous_operator = None;
                }
                kind => {
                    let Some((operator, left_power, right_power)) = infix_operator(kind) else {
                        break;
                    };
                    if left_power < minimum_power {
                        break;
                    }
                    let operator_span = self.advance().span;

                    if is_comparison(operator) && previous_operator.is_some_and(is_comparison) {
                        self.report(comparison_cannot_chain(operator, operator_span));
                        return Err(Failed);
                    }

                    let right = self.expression_from(right_power)?;
                    let span = left.span.to(right.span);
                    left = self.expression_node(
                        ExprKind::Binary {
                            operator,
                            left: Box::new(left),
                            right: Box::new(right),
                        },
                        span,
                    );
                    previous_operator = Some(operator);
                }
            }
        }

        Ok(left)
    }

    /// Prefix forms.
    ///
    /// Two groups, deliberately given different reach:
    ///
    /// * `not`, `found`, `success` and `failure` wrap a *value*, so they take a
    ///   whole comparison-level expression: `found count + 1` wraps the sum, and
    ///   `not ready == waiting` negates the comparison.
    /// * `-`, `try` and `receive from` act on a single thing, so they bind tightly:
    ///   `try parse(text) + 1` adds one to what `try` produced.
    fn prefix_expression(&mut self) -> Parse<Expr> {
        use TokenKind::*;
        let start = self.current().span;

        Ok(match self.peek() {
            Not => {
                self.advance();
                let operand = self.expression_from(POWER_COMPARISON)?;
                let span = start.to(operand.span);
                self.expression_node(
                    ExprKind::Unary { operator: UnaryOp::Not, operand: Box::new(operand) },
                    span,
                )
            }
            Minus => {
                self.advance();
                let operand = self.prefix_expression()?;
                let span = start.to(operand.span);
                self.expression_node(
                    ExprKind::Unary { operator: UnaryOp::Negate, operand: Box::new(operand) },
                    span,
                )
            }
            Found => {
                self.advance();
                let operand = self.expression_from(POWER_COMPARISON)?;
                self.wrapping(start, operand, ExprKind::Found)
            }
            Success => {
                self.advance();
                let operand = self.expression_from(POWER_COMPARISON)?;
                self.wrapping(start, operand, ExprKind::Success)
            }
            Failure => {
                self.advance();
                let operand = self.expression_from(POWER_COMPARISON)?;
                self.wrapping(start, operand, ExprKind::Failure)
            }
            Try => {
                self.advance();
                let operand = self.prefix_expression()?;
                self.wrapping(start, operand, ExprKind::Try)
            }
            Receive => {
                self.advance();
                self.expect_word(From, "the word `from`, as in `receive from inbox`")?;
                let channel = self.prefix_expression()?;
                let span = start.to(channel.span);
                self.expression_node(ExprKind::Receive { channel: Box::new(channel) }, span)
            }
            _ => self.postfix_expression()?,
        })
    }

    /// Builds one of the prefix forms that simply wraps the expression after it.
    fn wrapping(
        &mut self,
        start: Span,
        operand: Expr,
        build: fn(Box<Expr>) -> ExprKind,
    ) -> Expr {
        let span = start.to(operand.span);
        self.expression_node(build(Box::new(operand)), span)
    }

    /// Calls, field access and indexing, which all bind tighter than any operator.
    fn postfix_expression(&mut self) -> Parse<Expr> {
        let mut expr = self.primary_expression()?;
        loop {
            match self.peek() {
                TokenKind::Dot => {
                    self.advance();
                    // Any word may follow a `.`, so `value.with(...)` works even
                    // though `with` would be a poor variable name.
                    let name = self.any_word_as_name("a field or function name after `.`")?;
                    let span = expr.span.to(name.span);
                    expr = self
                        .expression_node(ExprKind::Member { target: Box::new(expr), name }, span);
                }
                TokenKind::OpenParen => {
                    let (arguments, span) = self.call_arguments()?;
                    let span = expr.span.to(span);
                    expr = self.expression_node(
                        ExprKind::Call { callee: Box::new(expr), arguments },
                        span,
                    );
                }
                TokenKind::OpenBracket => {
                    self.advance();
                    let index = self.with_braces_as_literals(|parser| parser.expression())?;
                    let close = self.expect(TokenKind::CloseBracket, "a `]` to close the index")?;
                    let span = expr.span.to(close.span);
                    expr = self.expression_node(
                        ExprKind::Index { target: Box::new(expr), index: Box::new(index) },
                        span,
                    );
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn primary_expression(&mut self) -> Parse<Expr> {
        use TokenKind::*;
        let token = self.current();

        let kind = match token.kind {
            Int => {
                self.advance();
                return self.integer_literal(token.span);
            }
            Float => {
                self.advance();
                return self.float_literal(token.span);
            }
            Text => {
                self.advance();
                ExprKind::Text(self.text_parts(token.span)?)
            }
            Yes => {
                self.advance();
                ExprKind::Bool(true)
            }
            No => {
                self.advance();
                ExprKind::Bool(false)
            }
            Nothing => {
                self.advance();
                ExprKind::Nothing
            }
            SelfValue => {
                self.advance();
                ExprKind::SelfValue
            }
            OpenParen => return self.parenthesised(),
            OpenBracket => return self.list_literal(),
            OpenBrace => {
                if self.brace_starts_block() {
                    // We are in a header position, such as an `if` condition, where
                    // the `{` belongs to the block that follows.
                    return Err(self.unexpected("a value"));
                }
                return self.map_literal();
            }
            If => return self.if_expression(),
            Match => return self.match_expression(),
            Select => return self.select_expression(),
            Start => {
                let start = self.advance().span;
                let body = self.block()?;
                let span = start.to(body.span);
                return Ok(self.expression_node(ExprKind::Start(Box::new(body)), span));
            }
            _ => {
                let name = self.name("a value")?;
                let span = name.span;
                return Ok(self.expression_node(ExprKind::Name(name), span));
            }
        };

        Ok(self.expression_node(kind, token.span))
    }

    fn integer_literal(&mut self, span: Span) -> Parse<Expr> {
        let text = strip_digit_separators(span.slice(self.source()));
        match text.parse::<i64>() {
            Ok(value) => Ok(self.expression_node(ExprKind::Int(value), span)),
            Err(_) => {
                self.report(
                    Diagnostic::error("number-too-large", "this number is too large for an Int")
                        .at(span, "Vaab could not hold this value")
                        .with_note(format!("the largest Int is {}", i64::MAX)),
                );
                Err(Failed)
            }
        }
    }

    fn float_literal(&mut self, span: Span) -> Parse<Expr> {
        let text = strip_digit_separators(span.slice(self.source()));
        match text.parse::<f64>() {
            Ok(value) if value.is_finite() => {
                Ok(self.expression_node(ExprKind::Float(value), span))
            }
            _ => {
                self.report(
                    Diagnostic::error("number-too-large", "this number is too large for a Float")
                        .at(span, "Vaab could not hold this value")
                        .with_note("Floats hold values up to about 1.8e308"),
                );
                Err(Failed)
            }
        }
    }

    /// `(expr)`, `(a, b)`, and the empty `()` that starts a no-argument closure.
    fn parenthesised(&mut self) -> Parse<Expr> {
        let open = self.expect(TokenKind::OpenParen, "a `(`")?;
        self.with_braces_as_literals(|parser| {
            parser.skip_newlines();
            if parser.check(TokenKind::CloseParen) {
                let close = parser.advance();
                let span = open.span.to(close.span);
                return Ok(parser.expression_node(ExprKind::Tuple(Vec::new()), span));
            }

            let first = parser.expression()?;
            parser.skip_newlines();

            if !parser.check(TokenKind::Comma) {
                let close =
                    parser.expect(TokenKind::CloseParen, "a `)` to close this group")?;
                // Grouping parentheses do not survive into the tree, but the span
                // covers them so error messages underline what was written. The
                // inner node's id comes along, because there is only one node here.
                return Ok(Expr { span: open.span.to(close.span), ..first });
            }

            let mut items = vec![first];
            while parser.eat(TokenKind::Comma) {
                parser.skip_newlines();
                if parser.check(TokenKind::CloseParen) {
                    break;
                }
                items.push(parser.expression()?);
                parser.skip_newlines();
            }
            let close = parser.expect(TokenKind::CloseParen, "a `)` to close this tuple")?;
            let span = open.span.to(close.span);
            Ok(parser.expression_node(ExprKind::Tuple(items), span))
        })
    }

    fn list_literal(&mut self) -> Parse<Expr> {
        let open = self.expect(TokenKind::OpenBracket, "a `[`")?;
        self.with_braces_as_literals(|parser| {
            let mut items = Vec::new();
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseBracket) {
                items.push(parser.expression()?);
                parser.skip_newlines();
                if !parser.eat(TokenKind::Comma) {
                    break;
                }
                parser.skip_newlines();
            }
            let close = parser.expect(
                TokenKind::CloseBracket,
                "a `,` before the next item, or a `]` to close this list",
            )?;
            let span = open.span.to(close.span);
            Ok(parser.expression_node(ExprKind::List(items), span))
        })
    }

    fn map_literal(&mut self) -> Parse<Expr> {
        let open = self.expect(TokenKind::OpenBrace, "a `{`")?;
        self.with_braces_as_literals(|parser| {
            let mut entries = Vec::new();
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseBrace) {
                let key = parser.expression()?;
                parser.expect(TokenKind::Colon, "a `:` between the key and its value")?;
                let value = parser.expression()?;
                let span = key.span.to(value.span);
                entries.push(MapEntry { key, value, span });
                parser.skip_newlines();
                if !parser.eat(TokenKind::Comma) {
                    break;
                }
                parser.skip_newlines();
            }
            let close = parser.expect(
                TokenKind::CloseBrace,
                "a `,` before the next entry, or a `}` to close this map",
            )?;
            let span = open.span.to(close.span);
            Ok(parser.expression_node(ExprKind::Map(entries), span))
        })
    }

    /// `greet("Ada", greeting: "hi")`
    fn call_arguments(&mut self) -> Parse<(Vec<Argument>, Span)> {
        let open = self.expect(TokenKind::OpenParen, "a `(` to start the arguments")?;
        self.with_braces_as_literals(|parser| {
            let mut arguments = Vec::new();
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseParen) {
                let start = parser.current().span;

                // `name:` makes this a named argument. Any word may be a label,
                // since a label can never be confused with an expression.
                let name = if parser.peek_at(1) == TokenKind::Colon
                    && (parser.peek().is_name_like() || parser.peek().keyword_text().is_some())
                {
                    let name = parser.any_word_as_name("an argument name")?;
                    parser.advance();
                    Some(name)
                } else {
                    None
                };

                let value = parser.expression()?;
                arguments.push(Argument { name, span: start.to(value.span), value });

                parser.skip_newlines();
                if !parser.eat(TokenKind::Comma) {
                    break;
                }
                parser.skip_newlines();
            }
            let close = parser.expect(
                TokenKind::CloseParen,
                "a `,` before the next argument, or a `)` to close the arguments",
            )?;
            Ok((arguments, open.span.to(close.span)))
        })
    }

    /// Turns `left -> body` into a closure, where `left` is the parameter list that
    /// has just been parsed as an ordinary expression.
    fn closure(&mut self, left: Expr) -> Parse<Expr> {
        let arrow = self.advance().span;
        let parameters = self.closure_parameters(left)?;
        let start = parameters.first().map(|name| name.span).unwrap_or(arrow);

        // A `{` right after `->` always opens a block. To return a map from a
        // closure, wrap it: `n -> ({"n": n})`.
        let body = if self.check(TokenKind::OpenBrace) {
            FunctionBody::Block(self.block()?)
        } else {
            FunctionBody::Expr(self.expression_from(POWER_ARROW)?)
        };

        let end = match &body {
            FunctionBody::Block(block) => block.span,
            FunctionBody::Expr(expr) => expr.span,
        };
        let span = start.to(end);
        Ok(self.expression_node(ExprKind::Closure { parameters, body: Box::new(body) }, span))
    }

    /// Reinterprets the expression on the left of `->` as parameter names.
    fn closure_parameters(&mut self, left: Expr) -> Parse<Vec<Name>> {
        match left.kind {
            ExprKind::Name(name) => Ok(vec![name]),
            ExprKind::Tuple(items) => {
                let mut names = Vec::with_capacity(items.len());
                for item in items {
                    match item.kind {
                        ExprKind::Name(name) => names.push(name),
                        _ => {
                            self.report(not_a_parameter(item.span));
                            return Err(Failed);
                        }
                    }
                }
                Ok(names)
            }
            _ => {
                self.report(not_a_parameter(left.span));
                Err(Failed)
            }
        }
    }

    /// `if condition { ... } else if ... { ... } else { ... }`
    fn if_expression(&mut self) -> Parse<Expr> {
        let start = self.current().span;
        let parts = self.if_parts()?;
        let end = match &parts.else_branch {
            Some(ElseBranch::Block(block)) => block.span,
            Some(ElseBranch::If(nested)) => nested.then_block.span,
            None => parts.then_block.span,
        };
        let span = start.to(end);
        Ok(self.expression_node(ExprKind::If(Box::new(parts)), span))
    }

    fn if_parts(&mut self) -> Parse<IfExpr> {
        self.expect(TokenKind::If, "the word `if`")?;
        let condition = self.expression_before_block()?;
        let then_block = self.block()?;

        let else_branch = if self.eat(TokenKind::Else) {
            if self.check(TokenKind::If) {
                Some(ElseBranch::If(Box::new(self.if_parts()?)))
            } else {
                Some(ElseBranch::Block(self.block()?))
            }
        } else {
            None
        };

        Ok(IfExpr { condition, then_block, else_branch })
    }

    /// `match value { when pattern then result ... }`
    fn match_expression(&mut self) -> Parse<Expr> {
        let start = self.expect(TokenKind::Match, "the word `match`")?.span;
        let subject = self.expression_before_block()?;
        let open = self.expect(TokenKind::OpenBrace, "a `{` to start the match arms")?;

        // A broken arm is recovered from here rather than thrown to the enclosing
        // statement, so one bad arm does not make the whole `match` — and every
        // arm after it — look like nonsense.
        let mut recovered = false;
        let (arms, end) = self.with_braces_as_literals(|parser| {
            let mut arms = Vec::new();
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseBrace) && !parser.at_end() {
                let before = parser.position;
                match parser.match_arm() {
                    Ok(arm) => arms.push(arm),
                    Err(Failed) => {
                        recovered = true;
                        parser.recover_to_statement_boundary();
                    }
                }
                if parser.position == before {
                    parser.advance();
                }
                parser.skip_newlines();
            }
            let close = parser.expect(TokenKind::CloseBrace, "a `}` to close the match")?;
            Ok((arms, close.span))
        })?;

        if arms.is_empty() && !recovered {
            self.report(
                Diagnostic::error("empty-match", "this `match` has no arms")
                    .at(open.span.to(end), "there is nothing to match against")
                    .with_help("add at least one arm, such as `when nothing then ...`"),
            );
            return Err(Failed);
        }

        let span = start.to(end);
        Ok(self.expression_node(ExprKind::Match(Box::new(MatchExpr { subject, arms })), span))
    }

    fn match_arm(&mut self) -> Parse<MatchArm> {
        use crate::ast::{ArmBody, ArmPattern};

        let start = self.current().span;
        let pattern = if self.eat(TokenKind::Otherwise) {
            ArmPattern::Otherwise
        } else {
            self.expect(TokenKind::When, "the word `when` to start a match arm")?;
            ArmPattern::Pattern(self.pattern()?)
        };

        // `when n if n < 0 then ...`
        let guard = if self.eat(TokenKind::If) { Some(self.expression()?) } else { None };

        self.expect(TokenKind::Then, "the word `then`, followed by this arm's result")?;

        // As after `->`, a `{` here opens a block rather than a map literal.
        let body = if self.check(TokenKind::OpenBrace) {
            ArmBody::Block(self.block()?)
        } else {
            ArmBody::Expr(self.expression()?)
        };

        let end = match &body {
            ArmBody::Block(block) => block.span,
            ArmBody::Expr(expr) => expr.span,
        };
        Ok(MatchArm { pattern, guard, body, span: start.to(end) })
    }

    /// `select { when receive from inbox as message { ... } otherwise { ... } }`
    fn select_expression(&mut self) -> Parse<Expr> {
        let start = self.expect(TokenKind::Select, "the word `select`")?.span;
        self.expect(TokenKind::OpenBrace, "a `{` to start the select arms")?;

        let (arms, otherwise, end) = self.with_braces_as_literals(|parser| {
            let mut arms = Vec::new();
            let mut otherwise = None;
            parser.skip_newlines();

            while !parser.check(TokenKind::CloseBrace) && !parser.at_end() {
                if parser.eat(TokenKind::Otherwise) {
                    let block = parser.block()?;
                    if otherwise.is_some() {
                        parser.report(
                            Diagnostic::error(
                                "duplicate-otherwise",
                                "this `select` already has an `otherwise` arm",
                            )
                            .at(block.span, "only one `otherwise` is allowed")
                            .with_help("merge the two arms into one"),
                        );
                        return Err(Failed);
                    }
                    otherwise = Some(block);
                } else {
                    let before = parser.position;
                    match parser.select_arm() {
                        Ok(arm) => arms.push(arm),
                        Err(Failed) => parser.recover_to_statement_boundary(),
                    }
                    if parser.position == before {
                        parser.advance();
                    }
                }
                parser.skip_newlines();
            }

            let close = parser.expect(TokenKind::CloseBrace, "a `}` to close the select")?;
            Ok((arms, otherwise, close.span))
        })?;

        let span = start.to(end);
        Ok(self.expression_node(ExprKind::Select(Box::new(SelectExpr { arms, otherwise })), span))
    }

    fn select_arm(&mut self) -> Parse<SelectArm> {
        let start = self
            .expect(TokenKind::When, "the word `when` to start a select arm")?
            .span;

        if self.eat(TokenKind::Receive) {
            self.expect_word(TokenKind::From, "the word `from`, as in `receive from inbox`")?;
            let channel = self.prefix_expression()?;
            let binding = if self.eat(TokenKind::As) {
                Some(self.name("a name for the received value")?)
            } else {
                None
            };
            let body = self.block()?;
            let span = start.to(body.span);
            return Ok(SelectArm::Receive { channel, binding, body, span });
        }

        if self.eat(TokenKind::Timeout) {
            self.expect_word(TokenKind::After, "the word `after`, as in `timeout after 2 seconds`")?;
            let amount = self.expression_before_block()?;
            let unit = self.name("a unit of time, such as `seconds`")?;
            let body = self.block()?;
            let span = start.to(body.span);
            return Ok(SelectArm::Timeout { amount, unit, body, span });
        }

        Err(self.unexpected("`receive from ...` or `timeout after ...` after `when`"))
    }

    /// Expects a soft keyword, with a message naming the whole phrase it belongs to.
    pub(crate) fn expect_word(&mut self, kind: TokenKind, expectation: &str) -> Parse<()> {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.unexpected(expectation))
        }
    }

    /// Parses a `{ ... }` block. Declared here because expressions need it too.
    pub(crate) fn block(&mut self) -> Parse<Block> {
        self.enter()?;
        let result = self.block_body();
        self.leave();
        result
    }

    fn block_body(&mut self) -> Parse<Block> {
        let open = self.expect(TokenKind::OpenBrace, "a `{` to start a block")?;
        let mut statements = Vec::new();

        let close = self.with_braces_as_literals(|parser| {
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseBrace) {
                if parser.at_end() {
                    parser.report(
                        Diagnostic::error("unclosed-block", "this block is never closed")
                            .at(Span::empty_at(parser.current().span.start), "the file ends here")
                            .also_at(open.span, "the block opened here")
                            .with_help("add a `}` to close it"),
                    );
                    return Err(Failed);
                }

                let before = parser.position;
                match parser.statement() {
                    Ok(statement) => {
                        if parser.finish_statement().is_err() {
                            parser.recover_to_statement_boundary();
                        }
                        statements.push(statement);
                    }
                    Err(Failed) => parser.recover_to_statement_boundary(),
                }
                if parser.position == before {
                    parser.advance();
                }
                parser.skip_newlines();
            }
            parser.expect(TokenKind::CloseBrace, "a `}` to close this block")
        })?;

        Ok(Block { statements, span: open.span.to(close.span) })
    }
}

fn not_a_parameter(span: Span) -> Diagnostic {
    Diagnostic::error("bad-closure-parameters", "this is not a parameter name")
        .at(span, "`->` expects names on its left")
        .with_help("write `n -> ...` for one parameter, or `(a, b) -> ...` for several")
}

fn comparison_cannot_chain(operator: BinaryOp, span: Span) -> Diagnostic {
    let symbol = operator.spelling();
    Diagnostic::error("chained-comparison", "comparisons cannot be chained")
        .at(span, format!("this `{symbol}` compares the result of the comparison before it"))
        .with_help("write the two comparisons separately, joined with `and`")
        .with_note("for example, `low < value and value < high`")
}

fn range_cannot_chain(span: Span) -> Diagnostic {
    Diagnostic::error("chained-range", "ranges cannot be chained")
        .at(span, "this `..` follows another `..`")
        .with_help("a range has exactly two ends, as in `1..10`")
}

//! Statements and declarations.

use crate::ast::{
    AbilityDecl, AssignStmt, ChoiceDecl, Expr, ExprKind, Field, ForEachStmt, FunctionBody,
    FunctionDecl, LetStmt, Name, NeedImports, NeedSource, NeedStmt, Parameter, RepeatStmt,
    SendStmt, Stmt, StmtKind, TypeDecl, Variant, VariantField, WhileStmt,
};
use crate::diagnostic::Diagnostic;
use crate::span::Span;
use crate::token::TokenKind;

use super::{Failed, Parse, Parser};

impl<'src> Parser<'src> {
    /// Parses one statement, not including the line ending that follows it.
    pub(crate) fn statement(&mut self) -> Parse<Stmt> {
        use TokenKind::*;
        let start = self.current().span;

        let kind = match self.peek() {
            Let => StmtKind::Let(self.let_statement()?),
            Return => {
                self.advance();
                let value =
                    if self.at_statement_end() { None } else { Some(self.expression()?) };
                StmtKind::Return(value)
            }
            For => StmtKind::ForEach(self.for_each_statement()?),
            While => StmtKind::While(self.while_statement()?),
            Repeat => StmtKind::Repeat(self.repeat_statement()?),
            Send => StmtKind::Send(self.send_statement()?),
            Close => {
                self.advance();
                StmtKind::Close(self.expression()?)
            }
            Together => {
                self.advance();
                StmtKind::Together(self.block()?)
            }
            Type => StmtKind::Type(Box::new(self.type_declaration()?)),
            Choice => StmtKind::Choice(Box::new(self.choice_declaration()?)),
            Ability => StmtKind::Ability(Box::new(self.ability_declaration()?)),
            Serve => StmtKind::Serve(Box::new(self.serve_declaration()?)),
            Reply => StmtKind::Reply(self.reply_statement()?),
            Need => StmtKind::Need(self.need_statement()?),

            // `to` at the start of a statement always defines a function. Elsewhere
            // it is the connector in `send ... to ...` and in `map of K to V`.
            To => StmtKind::Function(Box::new(self.function_declaration(true)?)),
            Pure if self.peek_at(1) == To => {
                StmtKind::Function(Box::new(self.function_declaration(true)?))
            }

            _ => return self.expression_or_assignment(),
        };

        let end = self.previous_span();
        Ok(self.statement_node(kind, start.to(end)))
    }

    /// The span of the token just consumed, used to close off a statement's span.
    pub(crate) fn previous_span(&self) -> Span {
        let index = self.position.saturating_sub(1);
        self.tokens[index].span
    }

    fn let_statement(&mut self) -> Parse<LetStmt> {
        self.expect(TokenKind::Let, "the word `let`")?;
        let changing = self.eat(TokenKind::Changing);

        let name = self.name(if changing {
            "a name for this changeable value"
        } else {
            "a name for this value"
        })?;

        let declared_type =
            if self.eat(TokenKind::Colon) { Some(self.type_expression()?) } else { None };

        if !self.check(TokenKind::Equals) {
            // The commonest shape of this mistake is a declaration with no value,
            // so say that rather than listing the tokens that would have been legal.
            self.report(
                Diagnostic::error("let-without-value", "this `let` does not give a value")
                    .at(Span::empty_at(name.span.end), "Vaab expected an `=` and a value here")
                    .also_at(name.span, format!("`{}` is declared here", name.text))
                    .with_help(format!("write `let {} = ...`", name.text))
                    .with_note("every value in Vaab is given when it is declared, so there is no `nothing` to fall back on"),
            );
            return Err(Failed);
        }
        self.advance();

        let value = self.expression()?;
        Ok(LetStmt { changing, name, declared_type, value })
    }

    fn for_each_statement(&mut self) -> Parse<ForEachStmt> {
        self.expect(TokenKind::For, "the word `for`")?;
        self.expect_word(TokenKind::Each, "the word `each`, as in `for each item in items`")?;
        let pattern = self.pattern()?;
        self.expect_word(TokenKind::In, "the word `in`, as in `for each item in items`")?;
        let sequence = self.expression_before_block()?;
        let body = self.block()?;
        Ok(ForEachStmt { pattern, sequence, body })
    }

    fn while_statement(&mut self) -> Parse<WhileStmt> {
        self.expect(TokenKind::While, "the word `while`")?;
        let condition = self.expression_before_block()?;
        let body = self.block()?;
        Ok(WhileStmt { condition, body })
    }

    fn repeat_statement(&mut self) -> Parse<RepeatStmt> {
        self.expect(TokenKind::Repeat, "the word `repeat`")?;
        let count = self.expression_before_block()?;
        self.expect_word(TokenKind::Times, "the word `times`, as in `repeat 4 times { ... }`")?;
        let body = self.block()?;
        Ok(RepeatStmt { count, body })
    }

    fn send_statement(&mut self) -> Parse<SendStmt> {
        self.expect(TokenKind::Send, "the word `send`")?;
        let value = self.expression()?;
        self.expect(TokenKind::To, "the word `to`, as in `send \"ping\" to inbox`")?;
        let channel = self.expression()?;
        Ok(SendStmt { value, channel })
    }

    /// An expression used for its effect, or the left side of an assignment.
    fn expression_or_assignment(&mut self) -> Parse<Stmt> {
        let start = self.current().span;
        let target = self.expression()?;

        if !self.check(TokenKind::Equals) {
            let span = start.to(target.span);
            return Ok(self.statement_node(StmtKind::Expr(target), span));
        }

        let equals = self.advance().span;
        if !is_assignable(&target) {
            self.report(
                Diagnostic::error("cannot-assign", "this cannot be assigned to")
                    .at(target.span, "only a name, a field or an item can be assigned to")
                    .also_at(equals, "the assignment is here")
                    .with_help("give the result a name with `let`, or assign to a single name"),
            );
            return Err(Failed);
        }

        let value = self.expression()?;
        let span = start.to(value.span);
        Ok(self.statement_node(StmtKind::Assign(AssignStmt { target, value }), span))
    }

    // -----------------------------------------------------------------------
    // Declarations
    // -----------------------------------------------------------------------

    /// `pure to name(a: Int, b: Text = "x") returns T or fails E { ... }`
    ///
    /// `with_body` is false inside an `ability`, where only the signature is given.
    pub(crate) fn function_declaration(&mut self, with_body: bool) -> Parse<FunctionDecl> {
        let start = self.current().span;
        let pure = self.eat(TokenKind::Pure);
        self.expect(TokenKind::To, "the word `to` to define a function")?;

        let name = self.name("a name for this function")?;
        let parameters = self.parameter_list()?;

        let returns = if self.eat(TokenKind::Returns) {
            Some(self.type_expression()?)
        } else {
            None
        };

        if !with_body {
            if self.check(TokenKind::OpenBrace) || self.check(TokenKind::Equals) {
                let span = self.current().span;
                self.report(
                    Diagnostic::error(
                        "ability-with-body",
                        "an ability describes what a type must do, not how it does it",
                    )
                    .at(span, "this function should have no body")
                    .also_at(name.span, format!("`{}` is required here", name.text))
                    .with_help(format!(
                        "end the line after the signature, and write the body in each \
                         `type ... can ...` that provides `{}`",
                        name.text
                    )),
                );
                return Err(Failed);
            }
            let end = self.previous_span();
            return Ok(FunctionDecl {
                pure,
                name,
                parameters,
                returns,
                body: None,
                span: start.to(end),
            });
        }

        // The one-line form, `to double(n: Int) returns Int = n * 2`.
        let body = if self.eat(TokenKind::Equals) {
            FunctionBody::Expr(self.expression()?)
        } else {
            FunctionBody::Block(self.block()?)
        };

        let end = match &body {
            FunctionBody::Block(block) => block.span,
            FunctionBody::Expr(expr) => expr.span,
        };
        Ok(FunctionDecl { pure, name, parameters, returns, body: Some(body), span: start.to(end) })
    }

    fn parameter_list(&mut self) -> Parse<Vec<Parameter>> {
        self.expect(TokenKind::OpenParen, "a `(` to start the parameters")?;
        self.with_braces_as_literals(|parser| {
            let mut parameters = Vec::new();
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseParen) {
                let start = parser.current().span;
                let name = parser.name("a parameter name")?;

                if !parser.check(TokenKind::Colon) {
                    parser.report(missing_annotation(&name, "parameter"));
                    return Err(Failed);
                }
                parser.advance();

                let declared_type = parser.type_expression()?;
                let default =
                    if parser.eat(TokenKind::Equals) { Some(parser.expression()?) } else { None };

                let end = default.as_ref().map(|value| value.span).unwrap_or(declared_type.span);
                parameters.push(Parameter {
                    name,
                    declared_type,
                    default,
                    span: start.to(end),
                });

                parser.skip_newlines();
                if !parser.eat(TokenKind::Comma) {
                    break;
                }
                parser.skip_newlines();
            }
            parser.expect(
                TokenKind::CloseParen,
                "a `,` before the next parameter, or a `)` to close the parameters",
            )?;
            Ok(parameters)
        })
    }

    /// `type Account can Describable { ... }`
    fn type_declaration(&mut self) -> Parse<TypeDecl> {
        let start = self.expect(TokenKind::Type, "the word `type`")?.span;
        let name = self.name("a name for this type")?;

        let mut abilities = Vec::new();
        if self.eat(TokenKind::Can) {
            loop {
                abilities.push(self.name("the name of an ability")?);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
        }

        let open = self.expect(TokenKind::OpenBrace, "a `{` to start the type's body")?;
        let mut fields: Vec<Field> = Vec::new();
        let mut functions = Vec::new();

        let end = self.with_braces_as_literals(|parser| {
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseBrace) {
                if parser.at_end() {
                    parser.report(unclosed_body(open.span, parser.current().span, "type"));
                    return Err(Failed);
                }

                let before = parser.position;
                let outcome = if parser.starts_function() {
                    parser.function_declaration(true).map(|function| functions.push(function))
                } else {
                    parser.field().map(|field| {
                        // Fields are positional in `Type.raw(...)`, so a repeated
                        // name is a real problem rather than a shadowing question.
                        if let Some(first) =
                            fields.iter().find(|existing| existing.name.text == field.name.text)
                        {
                            let first_span = first.name.span;
                            let name = field.name.clone();
                            parser.report(duplicate_field(&name, first_span));
                        }
                        fields.push(field);
                    })
                };

                if outcome.is_err() || parser.finish_statement().is_err() {
                    parser.recover_to_statement_boundary();
                }
                if parser.position == before {
                    parser.advance();
                }
                parser.skip_newlines();
            }
            Ok(parser.expect(TokenKind::CloseBrace, "a `}` to close this type")?.span)
        })?;

        Ok(TypeDecl { name, abilities, fields, functions, span: start.to(end) })
    }

    /// Whether the member about to be read is a function rather than a field.
    fn starts_function(&self) -> bool {
        self.check(TokenKind::To)
            || (self.check(TokenKind::Pure) && self.peek_at(1) == TokenKind::To)
    }

    fn field(&mut self) -> Parse<Field> {
        let start = self.current().span;
        let name = self.name("a field name")?;

        if !self.check(TokenKind::Colon) {
            self.report(missing_annotation(&name, "field"));
            return Err(Failed);
        }
        self.advance();

        let declared_type = self.type_expression()?;
        let default = if self.eat(TokenKind::Equals) { Some(self.expression()?) } else { None };

        let end = default.as_ref().map(|value| value.span).unwrap_or(declared_type.span);
        Ok(Field { name, declared_type, default, span: start.to(end) })
    }

    /// `choice AccountError { InvalidAmount(amount: Int) Frozen }`
    fn choice_declaration(&mut self) -> Parse<ChoiceDecl> {
        let start = self.expect(TokenKind::Choice, "the word `choice`")?.span;
        let name = self.name("a name for this choice")?;
        let open = self.expect(TokenKind::OpenBrace, "a `{` to start the choice's variants")?;

        let mut variants: Vec<Variant> = Vec::new();
        let end = self.with_braces_as_literals(|parser| {
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseBrace) {
                if parser.at_end() {
                    parser.report(unclosed_body(open.span, parser.current().span, "choice"));
                    return Err(Failed);
                }

                let before = parser.position;
                match parser.variant() {
                    Ok(variant) => {
                        if let Some(first) =
                            variants.iter().find(|existing| existing.name.text == variant.name.text)
                        {
                            let first_span = first.name.span;
                            let name = variant.name.clone();
                            parser.report(duplicate_variant(&name, first_span));
                        }
                        variants.push(variant);
                        // Variants may be separated by line endings or by commas.
                        parser.eat(TokenKind::Comma);
                        if parser.finish_statement().is_err() {
                            parser.recover_to_statement_boundary();
                        }
                    }
                    Err(Failed) => parser.recover_to_statement_boundary(),
                }
                if parser.position == before {
                    parser.advance();
                }
                parser.skip_newlines();
            }
            Ok(parser.expect(TokenKind::CloseBrace, "a `}` to close this choice")?.span)
        })?;

        if variants.is_empty() {
            self.report(
                Diagnostic::error("empty-choice", format!("`{}` has no variants", name.text))
                    .at(name.span, "a choice must offer at least one thing to choose")
                    .with_help("add a variant, such as `Frozen`"),
            );
            return Err(Failed);
        }

        Ok(ChoiceDecl { name, variants, span: start.to(end) })
    }

    fn variant(&mut self) -> Parse<Variant> {
        let name = self.name("a variant name")?;
        let mut fields = Vec::new();
        let mut end = name.span;

        if self.check(TokenKind::OpenParen) {
            self.advance();
            self.with_braces_as_literals(|parser| {
                parser.skip_newlines();
                while !parser.check(TokenKind::CloseParen) {
                    let start = parser.current().span;
                    let field_name = parser.name("a name for this part of the variant")?;

                    if !parser.check(TokenKind::Colon) {
                        parser.report(missing_annotation(&field_name, "variant field"));
                        return Err(Failed);
                    }
                    parser.advance();

                    let declared_type = parser.type_expression()?;
                    fields.push(VariantField {
                        span: start.to(declared_type.span),
                        name: field_name,
                        declared_type,
                    });

                    parser.skip_newlines();
                    if !parser.eat(TokenKind::Comma) {
                        break;
                    }
                    parser.skip_newlines();
                }
                Ok(())
            })?;
            end = self.expect(TokenKind::CloseParen, "a `)` to close this variant")?.span;
        }

        Ok(Variant { span: name.span.to(end), name, fields })
    }

    /// `ability Describable { to describe() returns Text }`
    fn ability_declaration(&mut self) -> Parse<AbilityDecl> {
        let start = self.expect(TokenKind::Ability, "the word `ability`")?.span;
        let name = self.name("a name for this ability")?;
        let open = self.expect(TokenKind::OpenBrace, "a `{` to start the ability's body")?;

        let mut functions = Vec::new();
        let end = self.with_braces_as_literals(|parser| {
            parser.skip_newlines();
            while !parser.check(TokenKind::CloseBrace) {
                if parser.at_end() {
                    parser.report(unclosed_body(open.span, parser.current().span, "ability"));
                    return Err(Failed);
                }

                let before = parser.position;
                match parser.function_declaration(false) {
                    Ok(function) => {
                        functions.push(function);
                        if parser.finish_statement().is_err() {
                            parser.recover_to_statement_boundary();
                        }
                    }
                    Err(Failed) => parser.recover_to_statement_boundary(),
                }
                if parser.position == before {
                    parser.advance();
                }
                parser.skip_newlines();
            }
            Ok(parser.expect(TokenKind::CloseBrace, "a `}` to close this ability")?.span)
        })?;

        Ok(AbilityDecl { name, functions, span: start.to(end) })
    }

    /// `need json from ada`, `need colours from ./vendor/colours of decode, Error`.
    fn need_statement(&mut self) -> Parse<NeedStmt> {
        let start = self.expect(TokenKind::Need, "the word `need`")?.span;
        let name = self.name("the name of the riff to need")?;
        self.expect(TokenKind::From, "the word `from`")?;
        let source = self.need_source()?;

        let imports = if self.eat(TokenKind::Of) {
            let mut names = Vec::new();
            loop {
                names.push(self.name("a name to import from the riff")?);
                if !self.eat(TokenKind::Comma) {
                    break;
                }
            }
            NeedImports::Of(names)
        } else if self.eat(TokenKind::As) {
            NeedImports::As(self.name("an alias for the riff")?)
        } else {
            NeedImports::Qualified
        };

        let end = self.previous_span();
        Ok(NeedStmt { name, source, imports, span: start.to(end) })
    }

    fn need_source(&mut self) -> Parse<NeedSource> {
        use TokenKind::*;
        if self.check(Text) {
            let token = self.advance();
            return Ok(NeedSource::Path {
                path: token.text(self.source).trim_matches('"').to_string(),
                span: token.span,
            });
        }
        if self.check(Dot) || self.check(DotDot) {
            let start = self.current().span;
            let path = self.path_literal()?;
            return Ok(NeedSource::Path { path, span: start });
        }
        let owner = self.name("an owner, as in `need json from ada`")?;
        Ok(NeedSource::Registry { owner })
    }

    fn path_literal(&mut self) -> Parse<String> {
        use TokenKind::*;
        let mut path = String::new();
        while matches!(self.peek(), Dot | DotDot | Slash | Identifier) {
            path.push_str(self.advance().text(self.source));
        }
        if path.is_empty() {
            return Err(Failed);
        }
        Ok(path)
    }
}

/// Whether an expression names a place that can be assigned to.
fn is_assignable(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Name(_) | ExprKind::Member { .. } | ExprKind::Index { .. }
    )
}

fn missing_annotation(name: &Name, what: &str) -> Diagnostic {
    Diagnostic::error("missing-type", format!("this {what} has no type"))
        .at(Span::empty_at(name.span.end), "Vaab expected a `:` and a type here")
        .also_at(name.span, format!("`{}` is declared here", name.text))
        .with_help(format!("write `{}: Int`, or whichever type fits", name.text))
        .with_note("Vaab infers the types of local values, but never of a signature")
}

fn duplicate_field(name: &Name, first: Span) -> Diagnostic {
    Diagnostic::error("duplicate-field", format!("`{}` is declared twice", name.text))
        .at(name.span, "this field repeats an earlier one")
        .also_at(first, "first declared here")
        .with_help("give one of them a different name, or remove it")
}

fn duplicate_variant(name: &Name, first: Span) -> Diagnostic {
    Diagnostic::error("duplicate-variant", format!("`{}` is listed twice", name.text))
        .at(name.span, "this variant repeats an earlier one")
        .also_at(first, "first listed here")
        .with_help("give one of them a different name, or remove it")
}

fn unclosed_body(open: Span, end_of_file: Span, what: &str) -> Diagnostic {
    Diagnostic::error("unclosed-block", format!("this {what} is never closed"))
        .at(Span::empty_at(end_of_file.start), "the file ends here")
        .also_at(open, format!("the {what} opened here"))
        .with_help("add a `}` to close it")
}

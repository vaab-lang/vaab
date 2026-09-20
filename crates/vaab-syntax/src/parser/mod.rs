//! The Vaab parser: hand-written recursive descent, with Pratt parsing for
//! expressions.
//!
//! The parser never panics and never stops at the first problem. When a statement
//! cannot be understood it records a [`Diagnostic`], skips to the next statement
//! boundary, and carries on, so a single run reports as many real errors as it can.
//!
//! This module holds the shared machinery: the cursor over the token stream, the
//! "expected this, found that" helpers, and error recovery. The grammar itself
//! lives in the submodules:
//!
//! * [`stmt`] — statements and declarations
//! * [`expr`] — expressions, including operator precedence
//! * [`types`] — types as written in the source
//! * [`pattern`] — `match` and `for each` patterns
//! * [`string`] — decoding interpolated text

mod expr;
mod pattern;
mod serve;
mod stmt;
mod string;
mod types;

use std::collections::BTreeSet;

use crate::ast::{Expr, ExprKind, Module, Name, NodeId, Pattern, PatternKind, Stmt, StmtKind};
use crate::diagnostic::Diagnostic;
use crate::lexer;
use crate::span::{LineColumn, Span};
use crate::token::{Token, TokenKind};

/// A parsed file, together with anything that could not be understood.
///
/// The module is always returned, even when there are errors: it holds every
/// statement that *did* parse, which is what editors and later phases want.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub module: Module,
    pub diagnostics: Vec<Diagnostic>,
}

impl Parsed {
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

/// Parses a whole source file.
pub fn parse(source: &str) -> Parsed {
    let lexed = lexer::tokenize(source);
    let mut parser = Parser::new(source, lexed.tokens, lexed.diagnostics);
    let module = parser.parse_module();

    // Lexing finds its problems before parsing starts, so without this the reports
    // would arrive out of order. People read a file top to bottom; their errors
    // should arrive the same way.
    let mut diagnostics = parser.diagnostics;
    diagnostics.sort_by_key(|diagnostic| diagnostic.primary_span().start);

    Parsed { module, diagnostics }
}

/// Signals that a diagnostic has been recorded and the current construct should be
/// abandoned. It carries no payload: the explanation is already in the parser's
/// diagnostics list.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Failed;

pub(crate) type Parse<T> = Result<T, Failed>;

/// How deeply expressions and blocks may nest before the parser gives up.
///
/// Recursive descent uses the machine stack, so pathological input (a file of ten
/// thousand open brackets) would otherwise overflow it. This turns a crash into a
/// diagnostic.
const MAX_NESTING: usize = 128;

pub(crate) struct Parser<'src> {
    source: &'src str,
    tokens: Vec<Token>,
    /// Index of the token about to be read.
    position: usize,
    diagnostics: Vec<Diagnostic>,
    depth: usize,
    /// The next [`NodeId`] to hand out. See [`Parser::node_id`].
    next_node: u32,
    /// While set, a `{` closes the current expression instead of opening a map
    /// literal. See [`Parser::expression_before_block`].
    brace_starts_block: bool,
    /// Lines that already carry a diagnostic.
    ///
    /// One mistake usually confuses the parser about everything that follows it on
    /// the same line, and a wall of consequences buries the cause. So Vaab reports
    /// at most one problem per line and lets the next run find the rest.
    reported_lines: BTreeSet<usize>,
}

impl<'src> Parser<'src> {
    pub(crate) fn new(source: &'src str, tokens: Vec<Token>, diagnostics: Vec<Diagnostic>) -> Self {
        let reported_lines = diagnostics
            .iter()
            .map(|diagnostic| LineColumn::of(source, diagnostic.primary_span().start).line)
            .collect();

        Parser {
            source,
            tokens,
            position: 0,
            diagnostics,
            depth: 0,
            next_node: 0,
            brace_starts_block: false,
            reported_lines,
        }
    }

    // -----------------------------------------------------------------------
    // Building nodes
    // -----------------------------------------------------------------------

    /// Hands out the next [`NodeId`].
    ///
    /// Wrapping would break the uniqueness later phases rely on, so the counter
    /// saturates instead. A file with four billion nodes in it has other problems.
    pub(crate) fn node_id(&mut self) -> NodeId {
        let id = NodeId(self.next_node);
        self.next_node = self.next_node.saturating_add(1);
        id
    }

    pub(crate) fn expression_node(&mut self, kind: ExprKind, span: Span) -> Expr {
        Expr { id: self.node_id(), kind, span }
    }

    pub(crate) fn statement_node(&mut self, kind: StmtKind, span: Span) -> Stmt {
        Stmt { id: self.node_id(), kind, span }
    }

    pub(crate) fn pattern_node(&mut self, kind: PatternKind, span: Span) -> Pattern {
        Pattern { id: self.node_id(), kind, span }
    }

    // -----------------------------------------------------------------------
    // Looking at tokens
    // -----------------------------------------------------------------------

    /// The token about to be read.
    pub(crate) fn current(&self) -> Token {
        self.tokens[self.position.min(self.tokens.len() - 1)]
    }

    pub(crate) fn peek(&self) -> TokenKind {
        self.current().kind
    }

    /// The kind of the token `offset` places ahead, saturating at end of file.
    pub(crate) fn peek_at(&self, offset: usize) -> TokenKind {
        let index = (self.position + offset).min(self.tokens.len() - 1);
        self.tokens[index].kind
    }

    pub(crate) fn check(&self, kind: TokenKind) -> bool {
        self.peek() == kind
    }

    pub(crate) fn at_end(&self) -> bool {
        self.peek() == TokenKind::EndOfFile
    }

    pub(crate) fn source(&self) -> &'src str {
        self.source
    }

    // -----------------------------------------------------------------------
    // Moving through tokens
    // -----------------------------------------------------------------------

    /// Reads one token and moves on.
    pub(crate) fn advance(&mut self) -> Token {
        let token = self.current();
        if self.position < self.tokens.len() - 1 {
            self.position += 1;
        }
        token
    }

    /// Reads one token if it is of the given kind. Returns whether it did.
    pub(crate) fn eat(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Reads one token, insisting that it is of the given kind.
    ///
    /// `expectation` completes the sentence "Vaab expected ...", so it should read
    /// like a thing rather than a token name: `"a `(` to start the arguments"`.
    pub(crate) fn expect(&mut self, kind: TokenKind, expectation: &str) -> Parse<Token> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            Err(self.unexpected(expectation))
        }
    }

    /// Skips any line endings, used wherever blank lines are meaningless.
    pub(crate) fn skip_newlines(&mut self) {
        while self.check(TokenKind::Newline) {
            self.advance();
        }
    }

    // -----------------------------------------------------------------------
    // Names
    // -----------------------------------------------------------------------

    /// Reads a name.
    ///
    /// Soft keywords are accepted here, which is what keeps words like `list` and
    /// `each` usable as ordinary names. Hard keywords are rejected with a message
    /// that says so plainly.
    pub(crate) fn name(&mut self, expectation: &str) -> Parse<Name> {
        if self.peek().is_name_like() {
            let token = self.advance();
            return Ok(Name::new(token.text(self.source), token.span));
        }

        if let Some(word) = self.peek().keyword_text() {
            let span = self.current().span;
            self.report(
                Diagnostic::error("reserved-word", format!("`{word}` is a reserved word"))
                    .at(span, format!("Vaab expected {expectation} here"))
                    .with_help(format!(
                        "`{word}` has a fixed meaning in Vaab, so it cannot be used as a name; \
                         pick another name"
                    )),
            );
            return Err(Failed);
        }

        Err(self.unexpected(expectation))
    }

    /// Reads a name in a position where any word is unambiguous: after `.`, and as
    /// a named-argument label. `value.with(...)` and `route(type: "get")` both rely
    /// on this.
    pub(crate) fn any_word_as_name(&mut self, expectation: &str) -> Parse<Name> {
        if self.peek().is_name_like() || self.peek().keyword_text().is_some() {
            let token = self.advance();
            return Ok(Name::new(token.text(self.source), token.span));
        }
        Err(self.unexpected(expectation))
    }

    // -----------------------------------------------------------------------
    // Errors and recovery
    // -----------------------------------------------------------------------

    /// Records a diagnostic, unless this line already has one.
    pub(crate) fn report(&mut self, diagnostic: Diagnostic) {
        let line = LineColumn::of(self.source, diagnostic.primary_span().start).line;
        if self.reported_lines.insert(line) {
            self.diagnostics.push(diagnostic);
        }
    }

    /// Records the standard "expected X, found Y" error at the current token.
    pub(crate) fn unexpected(&mut self, expectation: &str) -> Failed {
        let token = self.current();
        let found = token.kind.describe();

        // A line ending is what the reader *cannot see*, so name it specially
        // rather than underlining an invisible character.
        let (span, at) = match token.kind {
            TokenKind::Newline | TokenKind::EndOfFile => {
                (Span::empty_at(token.span.start), "the line ends here".to_string())
            }
            _ => (token.span, format!("found {found}")),
        };

        self.report(
            Diagnostic::error("unexpected-token", format!("Vaab expected {expectation}"))
                .at(span, at),
        );
        Failed
    }

    /// Guards against unbounded recursion on deeply nested input.
    pub(crate) fn enter(&mut self) -> Parse<()> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            let span = self.current().span;
            self.report(
                Diagnostic::error("too-deeply-nested", "this is nested too deeply to read")
                    .at(span, "Vaab gave up here")
                    .with_help("split this into smaller pieces, each with a name"),
            );
            return Err(Failed);
        }
        Ok(())
    }

    pub(crate) fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Whether the current token can end a statement.
    pub(crate) fn at_statement_end(&self) -> bool {
        matches!(
            self.peek(),
            TokenKind::Newline | TokenKind::CloseBrace | TokenKind::EndOfFile
        )
    }

    /// Consumes the line ending after a statement, complaining if there is
    /// leftover text on the line.
    pub(crate) fn finish_statement(&mut self) -> Parse<()> {
        match self.peek() {
            TokenKind::Newline => {
                self.advance();
                Ok(())
            }
            TokenKind::CloseBrace | TokenKind::EndOfFile => Ok(()),
            _ => {
                let token = self.current();
                self.report(
                    Diagnostic::error(
                        "unexpected-token",
                        "this statement seems to have ended already",
                    )
                    .at(token.span, format!("{} was not expected here", token.kind.describe()))
                    .with_help("start a new line, or check for a missing operator or comma"),
                );
                Err(Failed)
            }
        }
    }

    /// Skips forward to somewhere a new statement could plausibly start.
    ///
    /// Bracket depth is tracked so that a broken statement containing `(` or `{`
    /// does not swallow the rest of the enclosing block.
    pub(crate) fn recover_to_statement_boundary(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.peek() {
                TokenKind::EndOfFile => return,
                TokenKind::OpenParen | TokenKind::OpenBracket | TokenKind::OpenBrace => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::CloseParen | TokenKind::CloseBracket => {
                    depth = depth.saturating_sub(1);
                    self.advance();
                }
                TokenKind::CloseBrace => {
                    if depth == 0 {
                        // Leave it for the enclosing block to consume.
                        return;
                    }
                    depth -= 1;
                    self.advance();
                }
                TokenKind::Newline => {
                    self.advance();
                    if depth == 0 {
                        return;
                    }
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // The brace-literal restriction
    // -----------------------------------------------------------------------

    /// Parses an expression that is immediately followed by a block, such as the
    /// condition of an `if`.
    ///
    /// `{` is genuinely ambiguous in Vaab: it opens a map literal in an expression,
    /// and a block after a header. In these header positions the block wins, so
    /// `if ready { ... }` tests `ready` rather than trying to compare it with a map.
    /// To use a map literal in a header, wrap it in parentheses.
    pub(crate) fn expression_before_block(&mut self) -> Parse<crate::ast::Expr> {
        let saved = std::mem::replace(&mut self.brace_starts_block, true);
        let result = self.expression();
        self.brace_starts_block = saved;
        result
    }

    /// Parses `body` with map literals allowed again, used once the parser is
    /// safely inside brackets where no block can appear.
    pub(crate) fn with_braces_as_literals<T>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Parse<T>,
    ) -> Parse<T> {
        let saved = std::mem::replace(&mut self.brace_starts_block, false);
        let result = body(self);
        self.brace_starts_block = saved;
        result
    }

    pub(crate) fn brace_starts_block(&self) -> bool {
        self.brace_starts_block
    }

    // -----------------------------------------------------------------------
    // Entry point
    // -----------------------------------------------------------------------

    pub(crate) fn parse_module(&mut self) -> Module {
        let start = self.current().span;
        let mut statements = Vec::new();

        self.skip_newlines();
        while !self.at_end() {
            // A stray `}` at the top level has no block to close; say so once and
            // step over it, rather than looping forever.
            if self.check(TokenKind::CloseBrace) {
                let span = self.current().span;
                self.report(
                    Diagnostic::error("unmatched-brace", "this `}` does not close anything")
                        .at(span, "there is no open `{` above it")
                        .with_help("remove it, or add the matching `{`"),
                );
                self.advance();
                self.skip_newlines();
                continue;
            }

            let before = self.position;
            match self.statement() {
                Ok(statement) => {
                    if self.finish_statement().is_err() {
                        self.recover_to_statement_boundary();
                    }
                    statements.push(statement);
                }
                Err(Failed) => self.recover_to_statement_boundary(),
            }

            // Belt and braces: every path above should consume at least one token,
            // but a parser that silently stops making progress is far worse than
            // one that reports a little too much.
            if self.position == before {
                self.advance();
            }
            self.skip_newlines();
        }

        let end = self.current().span;
        Module { statements, span: start.to(end) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_parses_to_an_empty_module() {
        let parsed = parse("");
        assert!(parsed.module.statements.is_empty());
        assert!(!parsed.has_errors());
    }

    #[test]
    fn a_file_of_comments_parses_to_an_empty_module() {
        let parsed = parse("# just thinking\n\n# out loud\n");
        assert!(parsed.module.statements.is_empty());
        assert!(!parsed.has_errors());
    }

    #[test]
    fn parsing_always_terminates_on_rubbish() {
        // The point is that these return at all, rather than what they return.
        for source in ["}", "{{{{", ")))", "let", "to", "= = =", "((((((((("] {
            let parsed = parse(source);
            assert!(parsed.has_errors(), "{source:?} should not parse cleanly");
        }
    }

    #[test]
    fn at_most_one_problem_is_reported_per_line() {
        // The `&&` confuses the rest of the line; only the cause is worth saying.
        let parsed = parse("let both = a && b\n");
        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].code, "unknown-character");
    }

    #[test]
    fn every_node_gets_its_own_id() {
        // The type checker keys the type of each expression by its id, so a
        // repeated id would silently give two expressions one type.
        let source = "\
let total = (1 + 2) * 3
to double(n: Int) returns Int = n * 2
match total {
    when 0 then print(\"zero\")
    otherwise then print(\"{total} and {double(total)}\")
}
";
        let parsed = parse(source);
        assert!(!parsed.has_errors());

        let mut seen = BTreeSet::new();
        for id in collect_ids(&parsed.module) {
            assert!(seen.insert(id), "{id:?} was handed out twice");
        }
        assert!(seen.len() > 20, "only found {} nodes", seen.len());
    }

    #[test]
    fn a_group_keeps_the_id_of_what_it_surrounds() {
        // `(1)` is one node, not two: the brackets widen its span and vanish.
        let bare = parse("let a = 1\n");
        let grouped = parse("let a = (1)\n");
        assert_eq!(collect_ids(&bare.module).len(), collect_ids(&grouped.module).len());
    }

    /// Every statement, expression and pattern id in a module, in no order.
    #[cfg(test)]
    fn collect_ids(module: &Module) -> Vec<NodeId> {
        // Walking the tree properly would mean a visitor; the printed shape is not
        // enough, so this leans on `Debug` instead. It is only a test.
        let text = format!("{module:?}");
        let mut ids = Vec::new();
        let mut rest = text.as_str();
        while let Some(at) = rest.find("NodeId(") {
            rest = &rest[at + "NodeId(".len()..];
            let end = rest.find(')').unwrap_or(0);
            if let Ok(value) = rest[..end].parse::<u32>() {
                ids.push(NodeId(value));
            }
            rest = &rest[end..];
        }
        ids
    }

    #[test]
    fn deep_nesting_is_reported_rather_than_crashing() {
        let source = format!("let a = {}1{}", "(".repeat(500), ")".repeat(500));
        let parsed = parse(&source);
        assert!(parsed.diagnostics.iter().any(|d| d.code == "too-deeply-nested"));
    }
}

//! Turning Vaab source text into tokens.
//!
//! Two steps happen here:
//!
//! 1. `logos` scans the raw characters into tokens, skipping spaces and `#` comments.
//! 2. A second pass applies Vaab's line-continuation rules, deleting the line
//!    endings that do not actually finish a statement. Everything downstream can
//!    then treat a surviving [`TokenKind::Newline`] as a real statement terminator.

use logos::Logos;

use crate::diagnostic::Diagnostic;
use crate::span::{LineColumn, Span};
use crate::token::{Token, TokenKind};

/// The result of scanning a file: the tokens, plus anything that could not be read.
#[derive(Clone, Debug)]
pub struct Lexed {
    /// Always ends with exactly one [`TokenKind::EndOfFile`].
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lexed {
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

/// The token shapes `logos` recognises. Keywords are *not* listed here: they are
/// scanned as names and looked up afterwards, which keeps the keyword table in one
/// place (see [`crate::token`]).
#[derive(Logos, Clone, Copy, PartialEq, Debug)]
#[logos(skip r"[ \t\r\u{000c}]+")]
#[logos(skip r"#[^\n]*")]
enum Raw {
    #[token("\n")]
    Newline,

    // Listed before `Int` only for readability; `logos` picks the longest match, so
    // `1..10` still scans as `1`, `..`, `10` rather than starting a float.
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*")]
    Float,
    #[regex(r"[0-9][0-9_]*")]
    Int,

    /// A double-quoted string.
    ///
    /// Scanned by hand rather than by a regular expression, because a `{...}` hole
    /// may itself contain text: `"Ada is {ages.get("Ada") otherwise 0}"` is one
    /// token, not three. Escapes are accepted here and validated later, when the
    /// contents are decoded, so a bad escape gets a message about escapes rather
    /// than a generic "unexpected character".
    #[token("\"", scan_text_token)]
    Text,

    #[regex(r"[A-Za-z_][A-Za-z0-9_]*")]
    Name,

    #[token("->")]
    Arrow,
    #[token("==")]
    EqualsEquals,
    #[token("!=")]
    NotEquals,
    #[token("<=")]
    LessEquals,
    #[token(">=")]
    GreaterEquals,
    #[token("...")]
    Ellipsis,
    #[token("..")]
    DotDot,
    #[token("=")]
    Equals,
    #[token("<")]
    Less,
    #[token(">")]
    Greater,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token(".")]
    Dot,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("(")]
    OpenParen,
    #[token(")")]
    CloseParen,
    #[token("[")]
    OpenBracket,
    #[token("]")]
    CloseBracket,
    #[token("{")]
    OpenBrace,
    #[token("}")]
    CloseBrace,
}

/// Consumes the rest of a text literal once `logos` has read its opening quote.
///
/// Returns false when the text never closes, which `logos` turns into an error
/// token pointing at the opening quote.
fn scan_text_token(lexer: &mut logos::Lexer<Raw>) -> bool {
    let start = lexer.span().start;
    match scan_text(lexer.source(), start) {
        Some(end) => {
            // The opening quote has already been consumed.
            lexer.bump(end - start - 1);
            true
        }
        None => false,
    }
}

/// Finds the end of the text literal whose opening `"` is at `start`, as a byte
/// index just past the closing quote.
///
/// Scanning is done over bytes. Every byte examined is ASCII, and the two
/// positions ever returned sit just after a `"`, so multi-byte characters pass
/// through untouched and the result is always on a character boundary.
fn scan_text(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start + 1;

    while index < bytes.len() {
        match bytes[index] {
            // `\{` is a literal brace, so an escape must be stepped over before
            // any hole is considered.
            b'\\' => index += 2,
            // Text stays on one line, so a line ending means it never closed.
            b'\n' => return None,
            b'"' => return Some(index + 1),
            b'{' => index = scan_hole(source, index)?,
            _ => index += 1,
        }
    }
    None
}

/// Finds the end of the `{...}` hole that opens at `start`, as a byte index just
/// past its closing `}`.
fn scan_hole(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start + 1;
    let mut depth = 1usize;

    while index < bytes.len() {
        match bytes[index] {
            b'\n' => return None,
            // Text inside a hole may contain braces of its own; let the text
            // scanner step over the whole thing.
            b'"' => index = scan_text(source, index)?,
            b'{' => {
                depth += 1;
                index += 1;
            }
            b'}' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index += 1,
        }
    }
    None
}

impl Raw {
    /// The public token kind this raw token becomes. `Name` is resolved separately,
    /// because it may turn out to be a keyword.
    fn kind(self, text: &str) -> TokenKind {
        match self {
            Raw::Newline => TokenKind::Newline,
            Raw::Float => TokenKind::Float,
            Raw::Int => TokenKind::Int,
            Raw::Text => TokenKind::Text,
            Raw::Name => TokenKind::keyword(text).unwrap_or(TokenKind::Identifier),
            Raw::Arrow => TokenKind::Arrow,
            Raw::EqualsEquals => TokenKind::EqualsEquals,
            Raw::NotEquals => TokenKind::NotEquals,
            Raw::LessEquals => TokenKind::LessEquals,
            Raw::GreaterEquals => TokenKind::GreaterEquals,
            Raw::Ellipsis => TokenKind::Ellipsis,
            Raw::DotDot => TokenKind::DotDot,
            Raw::Equals => TokenKind::Equals,
            Raw::Less => TokenKind::Less,
            Raw::Greater => TokenKind::Greater,
            Raw::Plus => TokenKind::Plus,
            Raw::Minus => TokenKind::Minus,
            Raw::Star => TokenKind::Star,
            Raw::Slash => TokenKind::Slash,
            Raw::Percent => TokenKind::Percent,
            Raw::Dot => TokenKind::Dot,
            Raw::Comma => TokenKind::Comma,
            Raw::Colon => TokenKind::Colon,
            Raw::OpenParen => TokenKind::OpenParen,
            Raw::CloseParen => TokenKind::CloseParen,
            Raw::OpenBracket => TokenKind::OpenBracket,
            Raw::CloseBracket => TokenKind::CloseBracket,
            Raw::OpenBrace => TokenKind::OpenBrace,
            Raw::CloseBrace => TokenKind::CloseBrace,
        }
    }
}

/// Scans `source` into tokens and applies the line-continuation rules.
pub fn tokenize(source: &str) -> Lexed {
    let Lexed { tokens, diagnostics } = scan(source);
    Lexed { tokens: join_continued_lines(&tokens), diagnostics }
}

/// Scans `source` without applying the line-continuation rules.
///
/// Useful for testing and for tooling that wants to see every line ending.
pub fn scan(source: &str) -> Lexed {
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
    let mut lexer = Raw::lexer(source);

    // Consecutive unreadable characters are gathered into one span so that, say,
    // `$$$` produces a single message rather than three.
    let mut pending_error: Option<Span> = None;

    while let Some(result) = lexer.next() {
        let span = Span::from(lexer.span());
        match result {
            Ok(raw) => {
                if let Some(bad) = pending_error.take() {
                    diagnostics.push(unreadable(bad, source));
                }
                tokens.push(Token::new(raw.kind(lexer.slice()), span));
            }
            Err(()) => {
                pending_error = Some(match pending_error {
                    Some(previous) if previous.end == span.start => previous.to(span),
                    Some(previous) => {
                        diagnostics.push(unreadable(previous, source));
                        span
                    }
                    None => span,
                });
            }
        }
    }

    if let Some(bad) = pending_error {
        diagnostics.push(unreadable(bad, source));
    }

    tokens.push(Token::new(TokenKind::EndOfFile, Span::empty_at(source.len())));
    Lexed { tokens, diagnostics: keep_one_per_line(source, diagnostics) }
}

/// Keeps only the first diagnostic on each line.
///
/// An unreadable character usually confuses everything after it on the same line
/// — an unclosed `{` swallows the quote that would have ended the text, and then
/// the text is unclosed too. Reporting the cause and stopping is far more useful
/// than reporting the cause and all of its consequences.
fn keep_one_per_line(source: &str, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut seen = std::collections::BTreeSet::new();
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            seen.insert(LineColumn::of(source, diagnostic.primary_span().start).line)
        })
        .collect()
}

/// Builds the message for characters the lexer could not make sense of.
///
/// Several of these are worth recognising by hand, because a programmer arriving
/// from another language will reach for them and deserves to be pointed at the
/// Vaab spelling rather than told "unexpected character".
fn unreadable(span: Span, source: &str) -> Diagnostic {
    let text = span.slice(source);

    if text.starts_with('"') {
        // The text scanner gave up. Usually the closing quote is simply missing,
        // but an unclosed `{` swallows the quote that was meant to end the text, so
        // look for that first and name the real problem.
        let line = rest_of_line(source, span.start);
        if let Some(offset) = unmatched_open_brace(line) {
            let brace = Span::new(span.start + offset, span.start + offset + 1);
            return Diagnostic::error("unclosed-interpolation", "this `{` is never closed")
                .at(brace, "the hole starts here")
                .also_at(span, "inside this text")
                .with_help("add a `}` to close it, or write `\\{` for a literal `{`")
                .with_note("every piece of text in Vaab can hold values, so `{` always starts a hole");
        }

        return Diagnostic::error("unterminated-text", "this text is never closed")
            .at(span, "the text starts here")
            .with_help("add a closing `\"` before the end of the line")
            .with_note("text in Vaab stays on one line");
    }

    let (code, message, help) = match text {
        ";" => (
            "no-semicolons",
            "Vaab does not use semicolons".to_string(),
            Some("statements end at the end of the line, so this `;` can go".to_string()),
        ),
        "!" => (
            "unknown-character",
            "`!` is not an operator in Vaab".to_string(),
            Some("write `not` for negation, or `!=` to compare for inequality".to_string()),
        ),
        "&" | "&&" => (
            "unknown-character",
            format!("`{text}` is not an operator in Vaab"),
            Some("write `and` instead".to_string()),
        ),
        "|" | "||" => (
            "unknown-character",
            format!("`{text}` is not an operator in Vaab"),
            Some("write `or` instead".to_string()),
        ),
        "'" => (
            "unknown-character",
            "`'` does not start text in Vaab".to_string(),
            Some("text always uses double quotes, as in `\"hello\"`".to_string()),
        ),
        "//" => (
            "unknown-character",
            "`//` does not start a comment in Vaab".to_string(),
            Some("comments start with `#`".to_string()),
        ),
        _ => (
            "unknown-character",
            format!("`{text}` is not something Vaab can read"),
            None,
        ),
    };

    let diagnostic = Diagnostic::error(code, message).at(span, "here");
    match help {
        Some(help) => diagnostic.with_help(help),
        None => diagnostic,
    }
}

/// The source from `offset` up to the next line ending.
fn rest_of_line(source: &str, offset: usize) -> &str {
    let tail = source.get(offset..).unwrap_or("");
    match tail.find('\n') {
        Some(end) => &tail[..end],
        None => tail,
    }
}

/// The offset of the first `{` in `line` that never gets a matching `}`, if there
/// is one. Escaped braces do not count.
fn unmatched_open_brace(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut open: Vec<usize> = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'{' => {
                open.push(index);
                index += 1;
            }
            b'}' => {
                open.pop();
                index += 1;
            }
            _ => index += 1,
        }
    }
    open.first().copied()
}

/// Which bracket we are currently inside, for the line-continuation rules.
#[derive(Clone, Copy, PartialEq)]
enum Bracket {
    /// `(` and `[`: line endings inside these are never statement ends, so an
    /// argument list or a list literal may be spread over as many lines as it likes.
    Round,
    /// `{`: braces hold blocks, whose statements *are* separated by line endings,
    /// so they must be kept.
    Curly,
}

/// Deletes the line endings that continue onto the next line.
///
/// A line ending is dropped when any of these hold:
///
/// * it is inside `(` or `[`;
/// * the line ends with an operator, a comma, a colon, or an opening bracket;
/// * the next line starts with `.`, so a method chain can be broken across lines;
/// * nothing meaningful has been seen yet, or the previous token was also a line
///   ending, so blank lines collapse.
fn join_continued_lines(tokens: &[Token]) -> Vec<Token> {
    let mut joined: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut brackets: Vec<Bracket> = Vec::new();

    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::OpenParen | TokenKind::OpenBracket => brackets.push(Bracket::Round),
            TokenKind::OpenBrace => brackets.push(Bracket::Curly),
            TokenKind::CloseParen | TokenKind::CloseBracket | TokenKind::CloseBrace => {
                // An unbalanced closer is a parse error, not a lexing error; the
                // parser will report it with far better context than we could.
                brackets.pop();
            }
            TokenKind::Newline => {
                if brackets.last() == Some(&Bracket::Round) {
                    continue;
                }
                match joined.last() {
                    // Leading blank lines, and runs of blank lines, collapse away.
                    None => continue,
                    Some(previous) if previous.kind == TokenKind::Newline => continue,
                    Some(previous) if previous.kind.continues_line() => continue,
                    _ => {}
                }
                if starts_continuation(tokens, index) {
                    continue;
                }
            }
            _ => {}
        }
        joined.push(*token);
    }

    joined
}

/// Whether the line *after* the newline at `index` continues the current statement.
///
/// The one case is a leading `.`, which is how method chains are broken across
/// lines:
///
/// ```text
/// let shouted = names
///     .map(name -> name.upper())
///     .join(", ")
/// ```
fn starts_continuation(tokens: &[Token], index: usize) -> bool {
    tokens[index + 1..]
        .iter()
        .find(|token| token.kind != TokenKind::Newline)
        .is_some_and(|token| token.kind == TokenKind::Dot)
}

/// Removes the `_` digit separators from a numeric literal.
pub(crate) fn strip_digit_separators(text: &str) -> String {
    text.chars().filter(|character| *character != '_').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compact rendering of the token stream, for readable assertions.
    fn kinds(source: &str) -> Vec<TokenKind> {
        tokenize(source).tokens.iter().map(|token| token.kind).collect()
    }

    #[test]
    fn keywords_and_names_are_told_apart() {
        use TokenKind::*;
        assert_eq!(
            kinds("let changing count = 0"),
            [Let, Changing, Identifier, Equals, Int, EndOfFile]
        );
    }

    #[test]
    fn soft_keywords_are_still_usable_as_names() {
        // They scan as their own token kind; the parser accepts them where a name
        // is expected. What matters here is that they are not `Identifier`.
        assert_eq!(kinds("list")[0], TokenKind::List);
        assert_eq!(kinds("pure")[0], TokenKind::Pure);
    }

    #[test]
    fn comments_run_to_the_end_of_the_line() {
        use TokenKind::*;
        assert_eq!(kinds("let a = 1 # a comment\nlet b = 2"), [
            Let, Identifier, Equals, Int, Newline, Let, Identifier, Equals, Int, EndOfFile
        ]);
    }

    #[test]
    fn a_range_is_not_a_float() {
        use TokenKind::*;
        assert_eq!(kinds("1..10"), [Int, DotDot, Int, EndOfFile]);
        assert_eq!(kinds("1.5"), [Float, EndOfFile]);
    }

    #[test]
    fn the_rest_marker_beats_the_range_operator() {
        use TokenKind::*;
        assert_eq!(kinds("[first, ...]"), [
            OpenBracket, Identifier, Comma, Ellipsis, CloseBracket, EndOfFile
        ]);
    }

    #[test]
    fn arrow_beats_minus() {
        use TokenKind::*;
        assert_eq!(kinds("n -> n - 1"), [Identifier, Arrow, Identifier, Minus, Int, EndOfFile]);
    }

    #[test]
    fn a_trailing_operator_continues_the_line() {
        use TokenKind::*;
        assert_eq!(kinds("let a = 1 +\n2"), [Let, Identifier, Equals, Int, Plus, Int, EndOfFile]);
    }

    #[test]
    fn a_leading_dot_continues_the_line() {
        use TokenKind::*;
        assert_eq!(kinds("names\n.map(f)"), [
            Identifier, Dot, Map, OpenParen, Identifier, CloseParen, EndOfFile
        ]);
    }

    #[test]
    fn line_endings_vanish_inside_round_brackets() {
        use TokenKind::*;
        assert_eq!(kinds("greet(\n  \"Ada\",\n  \"hi\"\n)"), [
            Identifier, OpenParen, Text, Comma, Text, CloseParen, EndOfFile
        ]);
    }

    #[test]
    fn line_endings_survive_inside_braces() {
        use TokenKind::*;
        assert_eq!(kinds("start {\n  work()\n  rest()\n}"), [
            Start, OpenBrace, Identifier, OpenParen, CloseParen, Newline, Identifier, OpenParen,
            CloseParen, Newline, CloseBrace, EndOfFile
        ]);
    }

    #[test]
    fn a_block_inside_brackets_keeps_its_line_endings() {
        // The bracket stack, rather than a single "are we nested?" flag, is what
        // makes this work.
        use TokenKind::*;
        assert_eq!(kinds("run(start {\n  a()\n  b()\n})"), [
            Identifier, OpenParen, Start, OpenBrace, Identifier, OpenParen, CloseParen, Newline,
            Identifier, OpenParen, CloseParen, Newline, CloseBrace, CloseParen, EndOfFile
        ]);
    }

    #[test]
    fn blank_lines_collapse() {
        use TokenKind::*;
        assert_eq!(kinds("\n\n\na\n\n\nb\n\n"), [
            Identifier, Newline, Identifier, Newline, EndOfFile
        ]);
    }

    #[test]
    fn unreadable_characters_are_reported_once_each() {
        let lexed = tokenize("let a = $$$");
        assert_eq!(lexed.diagnostics.len(), 1);
        assert!(lexed.diagnostics[0].message.contains("$$$"));
    }

    #[test]
    fn familiar_mistakes_get_specific_advice() {
        let semicolon = tokenize("let a = 1;");
        assert_eq!(semicolon.diagnostics[0].code, "no-semicolons");

        let bang = tokenize("if !ready { }");
        assert!(bang.diagnostics[0].help.as_deref().is_some_and(|h| h.contains("`not`")));

        let ampersands = tokenize("if a && b { }");
        assert!(ampersands.diagnostics[0].help.as_deref().is_some_and(|h| h.contains("`and`")));
    }

    #[test]
    fn unterminated_text_says_so() {
        let lexed = tokenize("let greeting = \"hello\n");
        assert_eq!(lexed.diagnostics[0].code, "unterminated-text");
    }

    #[test]
    fn an_unclosed_hole_is_named_as_such() {
        // The missing `}` eats the closing quote, so the text also fails to close;
        // the message should point at the cause rather than the symptom.
        let lexed = tokenize("print(\"Ada is {age\")\n");
        assert_eq!(lexed.diagnostics[0].code, "unclosed-interpolation");
    }

    #[test]
    fn text_inside_a_hole_stays_in_the_same_token() {
        use TokenKind::*;
        assert_eq!(kinds(r#"print("Ada is {ages.get("Ada") otherwise 0}")"#), [
            Identifier, OpenParen, Text, CloseParen, EndOfFile
        ]);
    }

    #[test]
    fn an_escaped_brace_does_not_open_a_hole() {
        use TokenKind::*;
        assert_eq!(kinds(r#""a \{ brace""#), [Text, EndOfFile]);
    }

    #[test]
    fn digit_separators_are_stripped() {
        assert_eq!(strip_digit_separators("1_000_000"), "1000000");
    }
}

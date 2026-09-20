//! Friendly, precise error reporting.
//!
//! A [`Diagnostic`] is a plain data description of something that went wrong. It
//! knows nothing about terminals; [`render`] turns it into the underlined,
//! caret-annotated snippet that `ariadne` draws.
//!
//! House style for the wording, applied consistently across the compiler:
//!
//! * `message` states what is wrong, in one short sentence, in the third person.
//!   Never "you forgot"; the code is the subject, not the person.
//! * the primary label says what the compiler saw at that exact spot.
//! * `help` says what to do about it, concretely enough to type.
//! * `note` adds background only when it genuinely helps.

use std::fmt;

use ariadne::{Color, Config, Label as AriadneLabel, Report, ReportKind, Source};

use crate::span::{LineColumn, Span};

/// How serious a diagnostic is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
}

/// A span with a note attached, drawn underneath the source snippet.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
    /// The primary label is the one the caret points at. There is exactly one.
    pub primary: bool,
}

impl Label {
    pub fn primary(span: Span, message: impl Into<String>) -> Self {
        Label { span, message: message.into(), primary: true }
    }

    pub fn secondary(span: Span, message: impl Into<String>) -> Self {
        Label { span, message: message.into(), primary: false }
    }
}

/// One problem found in one source file.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// A short stable slug such as `unexpected-token`, shown next to the message
    /// so it can be searched for.
    pub code: &'static str,
    pub message: String,
    pub labels: Vec<Label>,
    pub help: Option<String>,
    pub note: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code,
            message: message.into(),
            labels: Vec::new(),
            help: None,
            note: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic { severity: Severity::Warning, ..Diagnostic::error(code, message) }
    }

    pub fn with_label(mut self, label: Label) -> Self {
        self.labels.push(label);
        self
    }

    /// Adds the primary label, the one the caret points at.
    pub fn at(self, span: Span, message: impl Into<String>) -> Self {
        self.with_label(Label::primary(span, message))
    }

    /// Adds a supporting label elsewhere in the file, such as "the block opened here".
    pub fn also_at(self, span: Span, message: impl Into<String>) -> Self {
        self.with_label(Label::secondary(span, message))
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Moves every span in this diagnostic `offset` bytes forward.
    ///
    /// Used when a fragment of source is scanned on its own — the inside of a
    /// `{...}` hole in a string — so that its errors still point at the right place
    /// in the original file.
    pub fn shifted(mut self, offset: usize) -> Self {
        for label in &mut self.labels {
            label.span = Span::new(label.span.start + offset, label.span.end + offset);
        }
        self
    }

    /// The span the caret points at, which is where the error "is".
    pub fn primary_span(&self) -> Span {
        self.labels
            .iter()
            .find(|label| label.primary)
            .or_else(|| self.labels.first())
            .map(|label| label.span)
            .unwrap_or_default()
    }
}

/// Where a diagnostic happened, for callers that want the bare facts rather than a
/// drawn snippet (an editor, a test, a `--format=short` mode).
#[derive(Clone, Debug)]
pub struct Location {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

impl Diagnostic {
    pub fn location(&self, file: &str, source: &str) -> Location {
        let LineColumn { line, column } = LineColumn::of(source, self.primary_span().start);
        Location { file: file.to_string(), line, column }
    }
}

/// Whether to emit ANSI colour codes.
///
/// Snapshot tests pin this to `Never` so the expected output stays stable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorChoice {
    Always,
    Never,
}

/// Draws `diagnostics` as annotated source snippets.
///
/// Returns one string containing every report, in the order given.
pub fn render(
    diagnostics: &[Diagnostic],
    file: &str,
    source: &str,
    color: ColorChoice,
) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics {
        out.push_str(&render_one(diagnostic, file, source, color));
    }
    out
}

/// Draws a single diagnostic.
pub fn render_one(
    diagnostic: &Diagnostic,
    file: &str,
    source: &str,
    color: ColorChoice,
) -> String {
    let kind = match diagnostic.severity {
        Severity::Error => ReportKind::Error,
        Severity::Warning => ReportKind::Warning,
    };
    let use_color = color == ColorChoice::Always;

    // `ariadne` addresses source files by a cheap, cloneable id; a `&str` will do.
    let primary = clamp(diagnostic.primary_span(), source);
    let mut report = Report::build(kind, (file, primary.range()))
        .with_code(diagnostic.code)
        .with_message(&diagnostic.message)
        .with_config(Config::default().with_color(use_color));

    for label in &diagnostic.labels {
        let span = clamp(label.span, source);
        let mut drawn = AriadneLabel::new((file, span.range())).with_message(&label.message);
        if use_color {
            drawn = drawn.with_color(if label.primary { Color::Red } else { Color::Blue });
        }
        // Primary labels are drawn first so the caret lands on them.
        drawn = drawn.with_order(if label.primary { 0 } else { 1 });
        report = report.with_label(drawn);
    }

    if let Some(help) = &diagnostic.help {
        report = report.with_help(help);
    }
    if let Some(note) = &diagnostic.note {
        report = report.with_note(note);
    }

    let mut buffer = Vec::new();
    let cache = (file, Source::from(source));
    match report.finish().write(cache, &mut buffer) {
        Ok(()) => String::from_utf8(buffer).unwrap_or_else(|_| fallback(diagnostic, file, source)),
        // Drawing is best-effort: if it fails we still have to tell the user
        // something useful, so fall back to a plain one-line report.
        Err(_) => fallback(diagnostic, file, source),
    }
}

/// Keeps a span inside the file and non-empty, because a zero-width or
/// past-the-end span has nothing to underline.
fn clamp(span: Span, source: &str) -> Span {
    let end_of_file = source.len();
    let mut start = span.start.min(end_of_file);
    let mut end = span.end.min(end_of_file).max(start);

    if start == end {
        // Widen by one character so there is something to draw a caret under,
        // preferring to grow forwards and only growing backwards at end of file.
        if let Some(next) = source[end..].chars().next() {
            end += next.len_utf8();
        } else if let Some(previous) = source[..start].chars().next_back() {
            start -= previous.len_utf8();
        }
    }

    // Snap both ends to character boundaries; slicing mid-character would panic.
    while start > 0 && !source.is_char_boundary(start) {
        start -= 1;
    }
    while end < end_of_file && !source.is_char_boundary(end) {
        end += 1;
    }

    Span::new(start, end)
}

/// A plain, always-available rendering used when drawing the snippet fails.
fn fallback(diagnostic: &Diagnostic, file: &str, source: &str) -> String {
    let location = diagnostic.location(file, source);
    let mut text = format!("{location}: {} [{}]\n", diagnostic.message, diagnostic.code);
    for label in &diagnostic.labels {
        text.push_str(&format!("  {}\n", label.message));
    }
    if let Some(help) = &diagnostic.help {
        text.push_str(&format!("  help: {help}\n"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_reports_line_and_column() {
        let source = "let a = 1\nlet b = ?\n";
        let diagnostic = Diagnostic::error("test", "nope").at(Span::new(18, 19), "here");
        let location = diagnostic.location("main.vaab", source);
        assert_eq!(location.to_string(), "main.vaab:2:9");
    }

    #[test]
    fn empty_spans_still_draw() {
        let source = "let a = \n";
        let diagnostic = Diagnostic::error("test", "nope").at(Span::empty_at(8), "here");
        let drawn = render_one(&diagnostic, "main.vaab", source, ColorChoice::Never);
        assert!(drawn.contains("main.vaab:1:9"), "{drawn}");
        assert!(drawn.contains("nope"), "{drawn}");
    }

    #[test]
    fn a_span_at_end_of_file_does_not_panic() {
        let source = "let a =";
        let diagnostic = Diagnostic::error("test", "nope").at(Span::empty_at(source.len()), "here");
        let drawn = render_one(&diagnostic, "main.vaab", source, ColorChoice::Never);
        assert!(drawn.contains("nope"), "{drawn}");
    }
}

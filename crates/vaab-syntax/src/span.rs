//! Source positions.
//!
//! Every token and every AST node carries a [`Span`]: a half-open range of byte
//! offsets into the source text. Byte offsets (rather than line/column pairs) are
//! what `ariadne` wants, and they are cheap to copy and to merge.

use std::fmt;
use std::ops::Range;

/// A half-open byte range `start..end` within a single source file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// An empty span sitting just before `offset`, used to point at a place where
    /// something is *missing* rather than at a token that is present.
    pub const fn empty_at(offset: usize) -> Self {
        Span { start: offset, end: offset }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }

    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn range(self) -> Range<usize> {
        self.start..self.end
    }

    /// The slice of `source` this span covers, or `""` if the span is out of bounds.
    pub fn slice(self, source: &str) -> &str {
        source.get(self.range()).unwrap_or("")
    }
}

impl From<Range<usize>> for Span {
    fn from(range: Range<usize>) -> Self {
        Span { start: range.start, end: range.end }
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Self {
        span.range()
    }
}

/// Renders as `12..17`, which keeps snapshot tests readable.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A 1-based line and column, computed on demand for human-facing messages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineColumn {
    pub line: usize,
    pub column: usize,
}

impl LineColumn {
    /// Locates `offset` within `source`.
    ///
    /// The column counts characters, not bytes, so a line of emoji does not report
    /// wildly inflated columns.
    pub fn of(source: &str, offset: usize) -> LineColumn {
        let offset = offset.min(source.len());
        let before = &source[..offset];
        let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let column = source[line_start..offset].chars().count() + 1;
        LineColumn { line, column }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_spans_covers_both() {
        assert_eq!(Span::new(2, 4).to(Span::new(9, 11)), Span::new(2, 11));
        assert_eq!(Span::new(9, 11).to(Span::new(2, 4)), Span::new(2, 11));
    }

    #[test]
    fn line_and_column_are_one_based() {
        let source = "let a = 1\nlet b = 2\n";
        assert_eq!(LineColumn::of(source, 0), LineColumn { line: 1, column: 1 });
        assert_eq!(LineColumn::of(source, 10), LineColumn { line: 2, column: 1 });
        assert_eq!(LineColumn::of(source, 14), LineColumn { line: 2, column: 5 });
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let source = "let name = \"héllo\"";
        let offset = source.find("llo").unwrap_or(0);
        // `é` is two bytes but one column.
        assert_eq!(LineColumn::of(source, offset).column, 15);
    }
}

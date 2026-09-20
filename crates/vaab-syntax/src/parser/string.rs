//! Interpolated text.
//!
//! Every string in Vaab interpolates, so `"{greeting}, {name}"` needs no prefix or
//! special form. The lexer treats a string as one token; this module takes that
//! token apart into literal runs and `{...}` holes, and parses each hole as an
//! ordinary expression.
//!
//! Holes are scanned from the original source rather than from a copied substring,
//! so every span inside a hole points at the real file and errors underline exactly
//! what was written.

use crate::ast::{Expr, TextPart};
use crate::diagnostic::Diagnostic;
use crate::lexer;
use crate::span::Span;
use crate::token::Token;

use super::{Failed, Parse, Parser};

impl<'src> Parser<'src> {
    /// Splits a text token into its literal runs and its holes.
    ///
    /// `span` covers the whole token, quotes included.
    pub(crate) fn text_parts(&mut self, span: Span) -> Parse<Vec<TextPart>> {
        // The lexer only produces this token for a properly quoted string, so the
        // quotes are known to be there; `saturating_sub` keeps it safe regardless.
        let contents = Span::new(span.start + 1, span.end.saturating_sub(1));
        let source = self.source();
        let text = contents.slice(source);

        let mut parts: Vec<TextPart> = Vec::new();
        let mut literal = String::new();
        let mut index = 0usize;

        while index < text.len() {
            let Some(character) = text[index..].chars().next() else { break };

            match character {
                '\\' => {
                    let escape_at = contents.start + index;
                    index += 1;
                    let Some(escaped) = text[index..].chars().next() else {
                        self.report(dangling_escape(Span::new(escape_at, escape_at + 1)));
                        return Err(Failed);
                    };
                    match decode_escape(escaped) {
                        Some(decoded) => literal.push(decoded),
                        None => {
                            let span = Span::new(escape_at, escape_at + 1 + escaped.len_utf8());
                            self.report(unknown_escape(escaped, span));
                            return Err(Failed);
                        }
                    }
                    index += escaped.len_utf8();
                }

                '{' => {
                    let open_at = contents.start + index;
                    let Some(close) = find_closing_brace(text, index) else {
                        self.report(unclosed_hole(Span::new(open_at, open_at + 1)));
                        return Err(Failed);
                    };

                    if !literal.is_empty() {
                        parts.push(TextPart::Literal(std::mem::take(&mut literal)));
                    }

                    let inner = Span::new(contents.start + index + 1, contents.start + close);
                    parts.push(TextPart::Interpolation(self.parse_hole(inner, open_at)?));
                    index = close + 1;
                }

                // A lone `}` is just a closing brace. Only `{` needs escaping,
                // because only `{` could start something.
                other => {
                    literal.push(other);
                    index += other.len_utf8();
                }
            }
        }

        if !literal.is_empty() || parts.is_empty() {
            parts.push(TextPart::Literal(literal));
        }
        Ok(parts)
    }

    /// Parses the expression inside a `{...}` hole.
    fn parse_hole(&mut self, inner: Span, open_at: usize) -> Parse<Expr> {
        if inner.slice(self.source()).trim().is_empty() {
            self.report(
                Diagnostic::error("empty-interpolation", "this `{...}` has nothing in it")
                    .at(Span::new(open_at, inner.end + 1), "Vaab expected a value to show here")
                    .with_help("put a value inside, as in `\"hello, {name}\"`, or write `\\{` for a literal `{`"),
            );
            return Err(Failed);
        }

        let fragment = inner.slice(self.source());
        let lexed = lexer::tokenize(fragment);

        // Shift everything the sub-lexer produced back into the coordinates of the
        // real file, so spans and messages line up with what the programmer wrote.
        let offset = inner.start;
        let tokens: Vec<Token> = lexed
            .tokens
            .iter()
            .map(|token| {
                Token::new(
                    token.kind,
                    Span::new(token.span.start + offset, token.span.end + offset),
                )
            })
            .collect();
        for diagnostic in lexed.diagnostics {
            self.report(diagnostic.shifted(offset));
        }

        let mut inner_parser = Parser::new(self.source(), tokens, Vec::new());
        // Carry the nesting budget across, so `"{"{"{...}"}"}"` cannot recurse for
        // ever through the string parser.
        inner_parser.depth = self.depth;

        let result = inner_parser.expression();

        if result.is_ok() {
            inner_parser.skip_newlines();
            if !inner_parser.at_end() {
                let leftover = inner_parser.current().span;
                inner_parser.report(
                    Diagnostic::error("crowded-interpolation", "a `{...}` holds one value")
                        .at(leftover, "Vaab expected the hole to end before this")
                        .with_help("show one value per hole: `\"{a} and {b}\"`"),
                );
            }
        }

        let failed = !inner_parser.diagnostics.is_empty();
        let inner_diagnostics = std::mem::take(&mut inner_parser.diagnostics);
        for diagnostic in inner_diagnostics {
            self.report(diagnostic);
        }

        match result {
            Ok(expr) if !failed => Ok(expr),
            _ => Err(Failed),
        }
    }
}

/// The character an escape stands for, or `None` if Vaab does not know it.
fn decode_escape(character: char) -> Option<char> {
    Some(match character {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        '\\' => '\\',
        '"' => '"',
        '{' => '{',
        '}' => '}',
        _ => return None,
    })
}

/// Finds the `}` that closes the `{` at `open`, as a byte index into `text`.
///
/// Nested braces are counted, and strings inside the hole are stepped over, so
/// `"{ages.get("Ada") otherwise 0}"` finds the right brace even though the inner
/// string contains none.
fn find_closing_brace(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;

    while index < text.len() {
        let character = text[index..].chars().next()?;
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            '"' => {
                index = skip_nested_text(text, index)?;
                continue;
            }
            _ => {}
        }
        index += character.len_utf8();
    }

    None
}

/// Steps over a quoted string starting at `open`, returning the index just after
/// its closing quote.
fn skip_nested_text(text: &str, open: usize) -> Option<usize> {
    let mut index = open + 1;
    while index < text.len() {
        let character = text[index..].chars().next()?;
        match character {
            '\\' => {
                index += 1;
                let escaped = text[index..].chars().next()?;
                index += escaped.len_utf8();
            }
            '"' => return Some(index + 1),
            _ => index += character.len_utf8(),
        }
    }
    None
}

fn dangling_escape(span: Span) -> Diagnostic {
    Diagnostic::error("bad-escape", "this `\\` has nothing after it")
        .at(span, "an escape needs a character to escape")
        .with_help("write `\\\\` for a single backslash")
}

fn unknown_escape(character: char, span: Span) -> Diagnostic {
    Diagnostic::error("bad-escape", format!("`\\{character}` is not an escape Vaab knows"))
        .at(span, "this escape has no meaning")
        .with_help("Vaab knows `\\n`, `\\t`, `\\r`, `\\\\`, `\\\"`, `\\{` and `\\}`")
}

fn unclosed_hole(span: Span) -> Diagnostic {
    Diagnostic::error("unclosed-interpolation", "this `{` is never closed")
        .at(span, "the hole starts here")
        .with_help("add a `}` to close it, or write `\\{` for a literal `{`")
        .with_note("every piece of text in Vaab can hold values, so `{` always starts a hole")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_closing_brace_past_a_nested_string() {
        let text = r#"{ages.get("}") otherwise 0} tail"#;
        assert_eq!(find_closing_brace(text, 0), Some(26));
    }

    #[test]
    fn counts_nested_braces() {
        let text = "{a{b}c}";
        assert_eq!(find_closing_brace(text, 0), Some(6));
    }

    #[test]
    fn reports_an_unclosed_hole() {
        assert_eq!(find_closing_brace("{oops", 0), None);
    }

    #[test]
    fn knows_the_escapes() {
        assert_eq!(decode_escape('n'), Some('\n'));
        assert_eq!(decode_escape('{'), Some('{'));
        assert_eq!(decode_escape('q'), None);
    }
}

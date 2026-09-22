//! Tokens: the words and symbols Vaab is made of.
//!
//! Vaab reserves as few words as it can get away with. A word is *hard* only when
//! it begins a statement or an expression, because that is the only situation where
//! treating it as a name would make the grammar ambiguous. Everything else is a
//! *soft* keyword: the parser recognises it in the one position where it is
//! meaningful, and it stays available as an ordinary name everywhere else. That is
//! why `let list = [1, 2]` and `let each = 3` are both perfectly legal programs.

use crate::span::Span;

/// What kind of thing a token is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TokenKind {
    // ---- Literals and names -------------------------------------------------
    /// A whole number, e.g. `42` or `1_000_000`.
    Int,
    /// A decimal number, e.g. `3.5`.
    Float,
    /// A double-quoted string, including its quotes and any `{...}` holes.
    Text,
    /// A plain name, e.g. `count`, `Account`, `T`.
    Identifier,

    // ---- Hard keywords ------------------------------------------------------
    // These can never be used as a name.
    To,
    Let,
    Changing,
    Return,
    If,
    Else,
    While,
    For,
    Match,
    When,
    Then,
    Otherwise,
    Type,
    Cast,
    Choice,
    Ability,
    SelfValue,
    And,
    Or,
    Not,
    Yes,
    No,
    Found,
    Nothing,
    Success,
    Failure,
    Try,
    Send,
    Receive,
    Close,
    Start,
    Together,
    Select,
    Repeat,
    Need,

    // ---- Soft keywords ------------------------------------------------------
    // Meaningful in one position; usable as a name everywhere else.
    Returns,
    Fails,
    Each,
    In,
    Times,
    Of,
    Can,
    Entertains,
    Pure,
    As,
    From,
    Maybe,
    List,
    Map,
    Channel,
    Shared,
    Task,
    Timeout,
    After,
    Serve,
    Route,
    Reply,
    Port,
    Expecting,
    Explain,
    Status,
    With,
    Before,
    Every,
    Anything,

    // ---- Symbols ------------------------------------------------------------
    /// `->`
    Arrow,
    /// `=`
    Equals,
    /// `==`
    EqualsEquals,
    /// `!=`
    NotEquals,
    /// `<`
    Less,
    /// `>`
    Greater,
    /// `<=`
    LessEquals,
    /// `>=`
    GreaterEquals,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `%`
    Percent,
    /// `.`
    Dot,
    /// `..`
    DotDot,
    /// `...`, the "and the rest" marker in list patterns.
    Ellipsis,
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// `(`
    OpenParen,
    /// `)`
    CloseParen,
    /// `[`
    OpenBracket,
    /// `]`
    CloseBracket,
    /// `{`
    OpenBrace,
    /// `}`
    CloseBrace,

    // ---- Structure ----------------------------------------------------------
    /// A line ending that survived the line-continuation rules, so it really does
    /// end a statement.
    Newline,
    /// The end of the file.
    EndOfFile,
}

/// Every reserved word, paired with the token it produces.
///
/// Kept as one table so the lexer, the "is this a keyword?" check and the
/// documentation can never drift apart.
const KEYWORDS: &[(&str, TokenKind)] = &[
    // Hard keywords.
    ("to", TokenKind::To),
    ("let", TokenKind::Let),
    ("changing", TokenKind::Changing),
    ("return", TokenKind::Return),
    ("if", TokenKind::If),
    ("else", TokenKind::Else),
    ("while", TokenKind::While),
    ("for", TokenKind::For),
    ("match", TokenKind::Match),
    ("when", TokenKind::When),
    ("then", TokenKind::Then),
    ("otherwise", TokenKind::Otherwise),
    ("type", TokenKind::Type),
    ("cast", TokenKind::Cast),
    ("choice", TokenKind::Choice),
    ("ability", TokenKind::Ability),
    ("self", TokenKind::SelfValue),
    ("and", TokenKind::And),
    ("or", TokenKind::Or),
    ("not", TokenKind::Not),
    ("yes", TokenKind::Yes),
    ("no", TokenKind::No),
    ("found", TokenKind::Found),
    ("nothing", TokenKind::Nothing),
    ("success", TokenKind::Success),
    ("failure", TokenKind::Failure),
    ("try", TokenKind::Try),
    ("send", TokenKind::Send),
    ("receive", TokenKind::Receive),
    ("close", TokenKind::Close),
    ("start", TokenKind::Start),
    ("together", TokenKind::Together),
    ("select", TokenKind::Select),
    ("repeat", TokenKind::Repeat),
    ("need", TokenKind::Need),
    // Soft keywords.
    ("returns", TokenKind::Returns),
    ("fails", TokenKind::Fails),
    ("each", TokenKind::Each),
    ("in", TokenKind::In),
    ("times", TokenKind::Times),
    ("of", TokenKind::Of),
    ("can", TokenKind::Can),
    ("entertains", TokenKind::Entertains),
    ("pure", TokenKind::Pure),
    ("as", TokenKind::As),
    ("from", TokenKind::From),
    ("maybe", TokenKind::Maybe),
    ("list", TokenKind::List),
    ("map", TokenKind::Map),
    ("channel", TokenKind::Channel),
    ("shared", TokenKind::Shared),
    ("task", TokenKind::Task),
    ("timeout", TokenKind::Timeout),
    ("after", TokenKind::After),
    ("serve", TokenKind::Serve),
    ("route", TokenKind::Route),
    ("reply", TokenKind::Reply),
    ("port", TokenKind::Port),
    ("expecting", TokenKind::Expecting),
    ("explain", TokenKind::Explain),
    ("status", TokenKind::Status),
    ("with", TokenKind::With),
    ("before", TokenKind::Before),
    ("every", TokenKind::Every),
    ("anything", TokenKind::Anything),
];

impl TokenKind {
    /// Looks `word` up in the keyword table, or `None` if it is an ordinary name.
    pub fn keyword(word: &str) -> Option<TokenKind> {
        KEYWORDS.iter().find(|(text, _)| *text == word).map(|(_, kind)| *kind)
    }

    /// The source text of a reserved word, or `None` for literals and symbols.
    pub fn keyword_text(self) -> Option<&'static str> {
        KEYWORDS.iter().find(|(_, kind)| *kind == self).map(|(text, _)| *text)
    }

    /// Whether this token may stand in for a name.
    ///
    /// Soft keywords may; hard keywords may not. `Identifier` obviously may.
    pub fn is_name_like(self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Identifier
                | Returns
                | Fails
                | Each
                | In
                | Times
                | Of
                | Can
                | Entertains
                | Pure
                | As
                | From
                | Maybe
                | List
                | Map
                | Channel
                | Shared
                | Task
                | Timeout
                | After
                | Serve
                | Route
                | Reply
                | Port
                | Expecting
                | Explain
                | Status
                | With
                | Before
                | Every
                | Anything
        )
    }

    /// Whether this is a reserved word that can never be used as a name.
    pub fn is_reserved_word(self) -> bool {
        self.keyword_text().is_some() && !self.is_name_like()
    }

    /// Whether a line ending directly *after* this token continues onto the next
    /// line instead of finishing the statement.
    ///
    /// "A line continues if it ends with an operator, comma, or opening bracket."
    pub fn continues_line(self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Arrow
                | Equals
                | EqualsEquals
                | NotEquals
                | Less
                | Greater
                | LessEquals
                | GreaterEquals
                | Plus
                | Minus
                | Star
                | Slash
                | Percent
                | Dot
                | DotDot
                | Comma
                | Colon
                | OpenParen
                | OpenBracket
                | OpenBrace
                | And
                | Or
                | Not
                | Otherwise
        )
    }

    /// How to name this token inside an error message, already quoted or worded so
    /// it can be dropped straight into a sentence.
    pub fn describe(self) -> String {
        use TokenKind::*;
        match self {
            Int => "a whole number".to_string(),
            Float => "a decimal number".to_string(),
            Text => "some text".to_string(),
            Identifier => "a name".to_string(),
            Newline => "the end of the line".to_string(),
            EndOfFile => "the end of the file".to_string(),
            other => match other.keyword_text() {
                Some(word) => format!("the word `{word}`"),
                None => format!("`{}`", other.symbol_text().unwrap_or("?")),
            },
        }
    }

    /// The literal spelling of a symbol token.
    pub fn symbol_text(self) -> Option<&'static str> {
        use TokenKind::*;
        Some(match self {
            Arrow => "->",
            Equals => "=",
            EqualsEquals => "==",
            NotEquals => "!=",
            Less => "<",
            Greater => ">",
            LessEquals => "<=",
            GreaterEquals => ">=",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Dot => ".",
            DotDot => "..",
            Ellipsis => "...",
            Comma => ",",
            Colon => ":",
            OpenParen => "(",
            CloseParen => ")",
            OpenBracket => "[",
            CloseBracket => "]",
            OpenBrace => "{",
            CloseBrace => "}",
            _ => return None,
        })
    }
}

/// A single token: what it is, and where it came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }

    /// The exact source text this token covers.
    pub fn text<'src>(&self, source: &'src str) -> &'src str {
        self.span.slice(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_table_round_trips() {
        for (word, kind) in KEYWORDS {
            assert_eq!(TokenKind::keyword(word), Some(*kind), "{word} did not look up");
            assert_eq!(kind.keyword_text(), Some(*word), "{word} did not round-trip");
        }
    }

    #[test]
    fn to_is_reserved_but_list_is_not() {
        // The specification only promises that `to` can never be a name.
        assert!(TokenKind::To.is_reserved_word());
        assert!(!TokenKind::To.is_name_like());
        assert!(TokenKind::List.is_name_like());
        assert!(TokenKind::Each.is_name_like());
    }

    #[test]
    fn ordinary_words_are_not_keywords() {
        for word in ["count", "request", "new", "raw"] {
            assert_eq!(TokenKind::keyword(word), None, "{word} should stay usable as a name");
        }
    }

    #[test]
    fn descriptions_read_like_english() {
        assert_eq!(TokenKind::To.describe(), "the word `to`");
        assert_eq!(TokenKind::OpenBrace.describe(), "`{`");
        assert_eq!(TokenKind::Identifier.describe(), "a name");
        assert_eq!(TokenKind::EndOfFile.describe(), "the end of the file");
    }
}

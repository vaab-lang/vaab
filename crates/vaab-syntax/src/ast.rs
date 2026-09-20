//! The shape of a Vaab program after parsing.
//!
//! This is a *syntax* tree: it records what was written, not what it means. No
//! name resolution, no types, no desugaring. The type checker in `vaab-types` is
//! what gives these nodes meaning.
//!
//! Every node carries a [`Span`] so that later phases can point at the exact
//! source text that caused a problem.

use crate::span::Span;

/// A whole source file.
#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

/// A name written in the source, with the place it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub text: String,
    pub span: Span,
}

impl Name {
    pub fn new(text: impl Into<String>, span: Span) -> Self {
        Name { text: text.into(), span }
    }

    /// Vaab spells types in `UpperCamelCase` and everything else in `snake_case`.
    /// The parser does not enforce this, but it is a useful hint for diagnostics.
    pub fn looks_like_a_type(&self) -> bool {
        self.text.chars().next().is_some_and(char::is_uppercase)
    }
}

// ---------------------------------------------------------------------------
// Statements
// ---------------------------------------------------------------------------

/// One step in a block, or one declaration at the top level of a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// `let name = value` or `let changing count = 0`, with an optional annotation.
    Let(LetStmt),
    /// `count = count + 1`. Only legal for `changing` bindings, which the type
    /// checker enforces.
    Assign(AssignStmt),
    /// `return`, or `return value`.
    Return(Option<Expr>),
    /// `for each item in items { ... }`
    ForEach(ForEachStmt),
    /// `while condition { ... }`
    While(WhileStmt),
    /// `repeat 4 times { ... }`
    Repeat(RepeatStmt),
    /// `send value to channel`
    Send(SendStmt),
    /// `close channel`
    Close(Expr),
    /// `together { ... }`
    Together(Block),
    /// A function declaration, at the top level or nested inside a block.
    Function(Box<FunctionDecl>),
    /// `type Account { ... }`
    Type(Box<TypeDecl>),
    /// `choice AccountError { ... }`
    Choice(Box<ChoiceDecl>),
    /// `ability Describable { ... }`
    Ability(Box<AbilityDecl>),
    /// An expression evaluated for its effect, or, if it is last in a block, for
    /// the block's value.
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LetStmt {
    /// `true` for `let changing`, meaning the binding may be assigned to later.
    pub changing: bool,
    pub name: Name,
    /// An explicit annotation, as in `let total: Int = 0`. Locals are otherwise
    /// inferred.
    pub declared_type: Option<TypeExpr>,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssignStmt {
    /// The thing being assigned to: a name, a field, or an index.
    pub target: Expr,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForEachStmt {
    /// `for each (key, value) in pairs` destructures, so this is a pattern.
    pub pattern: Pattern,
    pub sequence: Expr,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RepeatStmt {
    pub count: Expr,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SendStmt {
    pub value: Expr,
    pub channel: Expr,
}

/// `{ ... }`: a sequence of statements whose value is its last expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// `to name(params) returns T { ... }`.
///
/// Also used for the signatures inside an `ability`, which have no body.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionDecl {
    /// `true` when written as `pure to f(...)`.
    pub pure: bool,
    pub name: Name,
    pub parameters: Vec<Parameter>,
    /// The declared result. `None` means the function returns `Nothing`.
    /// `returns T or fails E` arrives here as [`TypeKind::Fallible`].
    pub returns: Option<TypeExpr>,
    /// `None` for an ability's required signature, which has no implementation.
    pub body: Option<FunctionBody>,
    pub span: Span,
}

/// A function's implementation.
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionBody {
    /// `{ ... }`
    Block(Block),
    /// The one-line form, `= expression`.
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: Name,
    /// Always present: every function signature in Vaab is fully annotated.
    pub declared_type: TypeExpr,
    /// `greeting: Text = "hello"` makes the argument optional at call sites.
    pub default: Option<Expr>,
    pub span: Span,
}

/// `type Account can Describable { ... }`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeDecl {
    pub name: Name,
    /// The abilities listed after `can`.
    pub abilities: Vec<Name>,
    pub fields: Vec<Field>,
    pub functions: Vec<FunctionDecl>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Name,
    pub declared_type: TypeExpr,
    /// `balance: Int = 0` may be left out when calling `Account.new`.
    pub default: Option<Expr>,
    pub span: Span,
}

/// `choice AccountError { InvalidAmount(amount: Int) Frozen }`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceDecl {
    pub name: Name,
    pub variants: Vec<Variant>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub name: Name,
    /// Empty for a variant with no payload, such as `Frozen`.
    pub fields: Vec<VariantField>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VariantField {
    /// Payload fields are named, as in `InvalidAmount(amount: Int)`.
    pub name: Name,
    pub declared_type: TypeExpr,
    pub span: Span,
}

/// `ability Describable { to describe() returns Text }`.
#[derive(Clone, Debug, PartialEq)]
pub struct AbilityDecl {
    pub name: Name,
    /// Required signatures. Their `body` is `None`.
    pub functions: Vec<FunctionDecl>,
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// Text always interpolates, so it is a sequence of literal runs and holes.
    Text(Vec<TextPart>),
    /// The absent half of `maybe T`.
    Nothing,
    /// A bare name.
    Name(Name),
    /// `self`, inside a type's own functions.
    SelfValue,

    /// `[1, 2, 3]`
    List(Vec<Expr>),
    /// `{"Ada": 36}`
    Map(Vec<MapEntry>),
    /// `(1, "a")`
    Tuple(Vec<Expr>),
    /// `1..10`, inclusive at both ends.
    Range { start: Box<Expr>, end: Box<Expr> },

    /// `not ready`, `-count`
    Unary { operator: UnaryOp, operand: Box<Expr> },
    /// `a + b`, `a and b`
    Binary { operator: BinaryOp, left: Box<Expr>, right: Box<Expr> },

    /// `greet("Ada", greeting: "hi")`
    Call { callee: Box<Expr>, arguments: Vec<Argument> },
    /// `account.balance`, `Account.new`
    Member { target: Box<Expr>, name: Name },
    /// `items[0]`
    Index { target: Box<Expr>, index: Box<Expr> },

    /// `n -> n * n`, `(a, b) -> a + b`, `item -> { ... }`
    Closure { parameters: Vec<Name>, body: Box<FunctionBody> },

    /// `if a > b { ... } else { ... }`, which is an expression.
    If(Box<IfExpr>),
    /// `match value { when ... then ... }`
    Match(Box<MatchExpr>),

    /// `found x`
    Found(Box<Expr>),
    /// `success x`
    Success(Box<Expr>),
    /// `failure e`
    Failure(Box<Expr>),
    /// `try expr`: on failure, return the failure from the enclosing function.
    Try(Box<Expr>),
    /// `expr otherwise fallback`
    Otherwise { value: Box<Expr>, fallback: Box<Expr> },

    /// `receive from inbox`, which evaluates to `maybe T`.
    Receive { channel: Box<Expr> },
    /// `start { ... }`, which evaluates to `task of T`.
    Start(Box<Block>),
    /// `select { when ... }`
    Select(Box<SelectExpr>),
}

/// One piece of an interpolated string.
#[derive(Clone, Debug, PartialEq)]
pub enum TextPart {
    /// A run of literal characters, with escapes already decoded.
    Literal(String),
    /// A `{expr}` hole.
    Interpolation(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapEntry {
    pub key: Expr,
    pub value: Expr,
    pub span: Span,
}

/// An argument at a call site. `name` is set for `greet("Ada", greeting: "hi")`.
#[derive(Clone, Debug, PartialEq)]
pub struct Argument {
    pub name: Option<Name>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    /// `not`
    Not,
    /// `-`
    Negate,
}

impl UnaryOp {
    pub fn spelling(self) -> &'static str {
        match self {
            UnaryOp::Not => "not",
            UnaryOp::Negate => "-",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equals,
    NotEquals,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    And,
    Or,
}

impl BinaryOp {
    pub fn spelling(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Remainder => "%",
            BinaryOp::Equals => "==",
            BinaryOp::NotEquals => "!=",
            BinaryOp::Less => "<",
            BinaryOp::LessOrEqual => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterOrEqual => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfExpr {
    pub condition: Expr,
    pub then_block: Block,
    pub else_branch: Option<ElseBranch>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ElseBranch {
    /// `else if ...`
    If(Box<IfExpr>),
    /// `else { ... }`
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchExpr {
    pub subject: Expr,
    pub arms: Vec<MatchArm>,
}

/// `when pattern if guard then body`, or `otherwise then body`.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    pub pattern: ArmPattern,
    /// The optional `if` guard, as in `when n if n < 0`.
    pub guard: Option<Expr>,
    pub body: ArmBody,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArmPattern {
    Pattern(Pattern),
    /// The catch-all `otherwise` arm.
    Otherwise,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArmBody {
    Expr(Expr),
    Block(Block),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectExpr {
    pub arms: Vec<SelectArm>,
    /// The `otherwise` arm, which makes the whole `select` non-blocking.
    pub otherwise: Option<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SelectArm {
    /// `when receive from inbox as message { ... }`
    Receive { channel: Expr, binding: Option<Name>, body: Block, span: Span },
    /// `when timeout after 2 seconds { ... }`
    Timeout { amount: Expr, unit: Name, body: Block, span: Span },
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    /// Patterns match text exactly, so no interpolation is allowed here.
    Text(String),
    /// A new name bound to whatever was matched.
    Binding(Name),
    /// `nothing`
    Nothing,
    /// `found x`
    Found(Box<Pattern>),
    /// `success x`
    Success(Box<Pattern>),
    /// `failure e`
    Failure(Box<Pattern>),
    /// `AccountError.InvalidAmount(amount)`, or a bare `Frozen`.
    Variant { path: Vec<Name>, fields: Vec<Pattern> },
    /// `[first, second]`, or `[first, ...]` where `rest` is true.
    List { elements: Vec<Pattern>, rest: bool },
    /// `(a, b)`
    Tuple(Vec<Pattern>),
}

// ---------------------------------------------------------------------------
// Types as written in the source
// ---------------------------------------------------------------------------

/// A type as it appears in source. The type checker turns this into a real type.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    /// `Int`, `Text`, `Account`, or a type parameter such as `T`.
    Named(Name),
    /// `list of T`
    List(Box<TypeExpr>),
    /// `map of K to V`
    Map { key: Box<TypeExpr>, value: Box<TypeExpr> },
    /// `maybe T`
    Maybe(Box<TypeExpr>),
    /// `channel of T`
    Channel(Box<TypeExpr>),
    /// `task of T`
    Task(Box<TypeExpr>),
    /// `shared T`
    Shared(Box<TypeExpr>),
    /// `(A, B)`
    Tuple(Vec<TypeExpr>),
    /// `to(A, B) returns C`. `returns` is `None` for a function yielding `Nothing`.
    Function { parameters: Vec<TypeExpr>, returns: Option<Box<TypeExpr>> },
    /// `T or fails E`
    Fallible { ok: Box<TypeExpr>, error: Box<TypeExpr> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_carry_their_capitalisation_hint() {
        assert!(Name::new("Account", Span::new(0, 7)).looks_like_a_type());
        assert!(!Name::new("account", Span::new(0, 7)).looks_like_a_type());
        assert!(Name::new("T", Span::new(0, 1)).looks_like_a_type());
    }

    #[test]
    fn operator_spellings_use_words_where_the_language_does() {
        assert_eq!(BinaryOp::And.spelling(), "and");
        assert_eq!(UnaryOp::Not.spelling(), "not");
        assert_eq!(BinaryOp::Add.spelling(), "+");
    }
}

//! The front end of the Vaab programming language.
//!
//! Vaab source text goes in; an abstract syntax tree, or a set of friendly
//! diagnostics, comes out. Nothing in this crate knows what a program *means* —
//! that is the type checker's job.
//!
//! ```
//! let parsed = vaab_syntax::parse("let name = \"world\"\n");
//! assert!(!parsed.has_errors());
//! assert_eq!(parsed.module.statements.len(), 1);
//! ```
//!
//! When something cannot be parsed, render the diagnostics as annotated source:
//!
//! ```
//! use vaab_syntax::{ColorChoice, diagnostic};
//!
//! let source = "let count = \n";
//! let parsed = vaab_syntax::parse(source);
//! let report = diagnostic::render(&parsed.diagnostics, "main.vaab", source, ColorChoice::Never);
//! assert!(report.contains("main.vaab"));
//! ```

pub mod ast;
pub mod diagnostic;
pub mod lexer;
pub mod parser;
pub mod print;
pub mod span;
pub mod token;

pub use ast::Module;
pub use diagnostic::{ColorChoice, Diagnostic, Label, Severity};
pub use parser::{parse, Parsed};
pub use print::print_module;
pub use span::Span;
pub use token::{Token, TokenKind};

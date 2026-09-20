//! `vaab repl`: a session that grows a file one line at a time.
//!
//! The honest way to build this, given a compiler that works on whole files, is
//! to keep the file. Every line typed is added to the source of the session, the
//! whole thing is parsed and checked again, and only the statements the line added
//! are run. That costs a re-check per line — nothing, at the size a person types —
//! and buys exact agreement with `vaab run`: a session cannot accept something a
//! file would reject, because it is a file.
//!
//! The values a session works out live in the [`World`], which outlives each line,
//! so `let x = 1` on one line is still `1` on the next.

use std::io::{BufRead, Write};

use vaab_syntax::{diagnostic, ColorChoice, Module};
use vaab_vm::{error, Output, Ref, World};

/// What the session is called in its diagnostics. Errors quote a file name, and
/// this reads better in front of a line the person just typed than `<stdin>`.
const SESSION: &str = "session";

pub fn start(color: ColorChoice) {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();

    println!("vaab {} — an interactive session", env!("CARGO_PKG_VERSION"));
    println!("Type an expression to see its value. Press Ctrl-D to leave.");

    let mut session = Session::new(color);
    // What has been typed towards a statement that is not finished yet.
    let mut pending = String::new();

    loop {
        prompt(if pending.is_empty() { "vaab> " } else { "   ...> " });

        let Some(Ok(line)) = lines.next() else {
            // Ctrl-D, or a pipe that ran out. Either way the session is over.
            println!();
            return;
        };

        pending.push_str(&line);
        pending.push('\n');

        // A brace left open means the person is halfway through a block, so keep
        // reading rather than reporting a file that ends too soon.
        if unfinished(&pending) {
            continue;
        }

        let typed = std::mem::take(&mut pending);
        if typed.trim().is_empty() {
            continue;
        }
        session.feed(&typed);
    }
}

fn prompt(text: &str) {
    print!("{text}");
    // A prompt with no newline stays in the buffer until something flushes it.
    let _ = std::io::stdout().flush();
}

/// Whether more is expected: a `{`, `(` or `[` that has not been closed.
///
/// This counts brackets outside text and comments, which is enough to tell "still
/// typing" from "finished". Anything subtler is the parser's job, and the parser
/// gets its turn as soon as the brackets balance.
fn unfinished(source: &str) -> bool {
    let mut depth: i32 = 0;
    let mut characters = source.chars();

    while let Some(character) = characters.next() {
        match character {
            '#' => {
                // A comment runs to the end of its line.
                for character in characters.by_ref() {
                    if character == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut escaped = false;
                for character in characters.by_ref() {
                    match character {
                        _ if escaped => escaped = false,
                        '\\' => escaped = true,
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }

    depth > 0
}

/// One running session: the source so far, and the values it has worked out.
struct Session {
    color: ColorChoice,
    /// Every line that has been accepted, which together are a Vaab file.
    source: String,
    /// How many top-level statements that file had last time it ran, which is
    /// where the next line's statements begin.
    statements: usize,
    world: World,
}

impl Session {
    fn new(color: ColorChoice) -> Session {
        let empty = vaab_syntax::parse("");
        let checked = vaab_types::check(&empty.module).unwrap_or_default();
        let world = vaab_vm::prepare(&empty.module, &checked, Output::Terminal);
        Session { color, source: String::new(), statements: 0, world }
    }

    /// Takes one finished piece of input: checks it against everything typed so
    /// far, runs what it added, and shows the value it worked out.
    ///
    /// Anything wrong is reported and thrown away, leaving the session exactly as
    /// it was. A line that does not compile never becomes part of the file.
    fn feed(&mut self, typed: &str) {
        let mut attempt = self.source.clone();
        attempt.push_str(typed);

        let parsed = vaab_syntax::parse(&attempt);
        if parsed.has_errors() {
            self.report(&parsed.diagnostics, &attempt);
            return;
        }

        let checked = match vaab_types::check(&parsed.module) {
            Ok(checked) => checked,
            Err(problems) => {
                self.report(&problems, &attempt);
                return;
            }
        };

        let added = statement_count(&parsed.module);
        self.world.reload(Ref::new(vaab_vm::compile(&parsed.module, &checked)));

        match vaab_vm::run_from(&mut self.world, self.statements) {
            Ok(value) => {
                // A statement that worked something out shows it; one that only
                // did something — a `print`, a declaration — has already spoken.
                if !value.is_nothing() {
                    println!("{}", value.show());
                }
                self.source = attempt;
                self.statements = added;
            }
            Err(problem) => {
                eprint!("{}", error::render(&problem, SESSION, &attempt, self.color));
                // The statements before this one did run, so the session has to
                // keep them; only the line that stopped is dropped.
                self.source = attempt;
                self.statements = added;
            }
        }
    }

    fn report(&self, problems: &[diagnostic::Diagnostic], source: &str) {
        eprint!("{}", diagnostic::render(problems, SESSION, source, self.color));
    }
}

fn statement_count(module: &Module) -> usize {
    module.statements.len()
}

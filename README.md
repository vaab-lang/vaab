# Vaab

A strictly typed, plain-English programming language, written in Rust.

Vaab (said "vibe") prefers words to symbols, annotates every function signature,
and makes data races a compile error.

```vaab
type Account {
    owner: Text
    balance: Int = 0

    to deposit(amount: Int) returns Account or fails AccountError {
        if amount <= 0 { return failure AccountError.InvalidAmount(amount) }
        return success self.with(balance: self.balance + amount)
    }
}

let account = Account.new(owner: "Ada")

match account.deposit(25) {
    when success updated then print("{updated.owner} has {updated.balance}")
    when failure error   then print("that did not work")
}
```

## Status

Phases 1 to 4 of 7 are complete: the front end, the type checker, the machine,
and safe concurrency. Vaab source parses, type-checks and runs, and everything
that goes wrong — a mistake in the program, a calculation with no answer, or a
deadlock — is reported with a snippet, a caret and an explanation. Data races are
compile errors; channels, tasks, `select`, `together` and `shared` run on a
single-threaded scheduler with cooperative preemption.

| Phase | What it brings | State |
| --- | --- | --- |
| 1 | Lexer, AST, parser, diagnostics, `vaab parse` | done |
| 2 | Type checker, `vaab check` | done |
| 3 | Bytecode VM, standard library basics, `vaab run`, `vaab repl` | done |
| 4 | Sendability checking, channels, tasks, `select`, `shared`, scheduler | done |
| 5 | `pure` enforcement, `vaab new` | done |
| 5b | Standard library — file I/O, time, json | done |
| 6 | `serve` and `route`, `vaab-server`, `vaab serve` | done |
| 7 | Multi-threaded work-stealing scheduler | done |

## Try it

```sh
cargo run -p vaab-cli -- new demo && cargo run -p vaab-cli -- run demo/main.vaab
cargo run -p vaab-cli -- run examples/12_running_programs.vaab
cargo run -p vaab-cli -- check examples/05_types.vaab
cargo run -p vaab-cli -- parse examples/01_hello.vaab
```

Or type at it:

```sh
cargo run -p vaab-cli -- repl
```
```
vaab> let numbers = [1, 2, 3]
vaab> numbers.map(n -> n * n)
[1, 4, 9]
```

Feed it something broken to see the diagnostics:

```sh
echo 'let count = 1;' > /tmp/oops.vaab
cargo run -p vaab-cli -- parse /tmp/oops.vaab

printf 'let ages = {"Ada": 36}\nlet age: Int = ages.get("Ada")\n' > /tmp/maybe.vaab
cargo run -p vaab-cli -- check /tmp/maybe.vaab

printf 'let none = 0\nprint(10 / none)\n' > /tmp/nope.vaab
cargo run -p vaab-cli -- run /tmp/nope.vaab
```

## Layout

```
crates/vaab-syntax   lexer, AST, parser, diagnostics
crates/vaab-types    the type checker
crates/vaab-vm       the bytecode compiler and the machine that runs it
crates/vaab-cli      the `vaab` binary
docs/LANGUAGE.md     the living specification
docs/DECISIONS.md    design choices and their reasoning
examples/            runnable programs, one per feature
examples/staffpulse/ aspirational StaffPulse API rewrite (phase 5–6 spec)
```

Crates for the standard library and the server are added in the phase that first
needs them.

## Development

```sh
cargo test                     # everything
cargo insta review             # review changed snapshots
```

Parser behaviour and every error message are covered by `insta` snapshots. Those
snapshots are meant to be read: if a message would not help a newcomer work out
what to type next, it is wrong even when the test passes.

## Licence

MIT or Apache-2.0, at your option.

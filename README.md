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

Phases 1 and 2 of 7 are complete: the front end and the type checker. Vaab source
parses and type-checks, and mistakes are reported with a snippet, a caret and an
explanation. Nothing runs yet.

| Phase | What it brings | State |
| --- | --- | --- |
| 1 | Lexer, AST, parser, diagnostics, `vaab parse` | done |
| 2 | Type checker, `vaab check` | done |
| 3 | Bytecode VM, standard library basics, `vaab run`, `vaab repl` | next |
| 4 | Channels, tasks, `select`, `shared`, sendability checking | |
| 5 | Standard library, `pure` enforcement, `vaab new` | |
| 6 | `serve` and `route` | |
| 7 | Multi-threaded work-stealing scheduler | |

## Try it

```sh
cargo run -p vaab-cli -- check examples/05_types.vaab
cargo run -p vaab-cli -- parse examples/01_hello.vaab
```

Feed it something broken to see the diagnostics:

```sh
echo 'let count = 1;' > /tmp/oops.vaab
cargo run -p vaab-cli -- parse /tmp/oops.vaab

printf 'let ages = {"Ada": 36}\nlet age: Int = ages.get("Ada")\n' > /tmp/maybe.vaab
cargo run -p vaab-cli -- check /tmp/maybe.vaab
```

## Layout

```
crates/vaab-syntax   lexer, AST, parser, diagnostics
crates/vaab-types    the type checker
crates/vaab-cli      the `vaab` binary
docs/LANGUAGE.md     the living specification
docs/DECISIONS.md    design choices and their reasoning
examples/            runnable programs, one per feature
```

Crates for the VM, standard library and server are added in the phase that first
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

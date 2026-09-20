# Decisions

Choices the specification did not make, with the reasoning behind them. When
something was ambiguous the rule was: pick the simplest option that keeps the
language readable, write it down here, and carry on.

Each entry says what was decided, why, and — where it matters — what it costs.

---

## Phase 1: the front end

### D1. Crates are created in the phase that first needs them

The architecture names six crates. Only `vaab-syntax` and `vaab-cli` exist so far.
Empty placeholder crates would slow every build and every `cargo test` while
proving nothing. `vaab-types` arrives in phase 2, and so on.

### D2. Diagnostics live in `vaab-syntax`, not a crate of their own

Every later crate depends on `vaab-syntax`, so `vaab_syntax::diagnostic` is
reachable from all of them without adding a crate the specification did not list.
If the dependency graph ever stops being a chain, this should become `vaab-diag`.

### D3. Only words that start a statement or an expression are reserved

The specification only promises that `to` can never be a name. Reserving every
keyword would cost words people want: `list`, `map`, `each`, `in`, `pure`, `from`,
`as`, `can`, `times` are all plausible variable names.

So keywords come in two kinds:

* **Hard** (33): `to let changing return if else while for match when then
  otherwise type choice ability self and or not yes no found nothing success
  failure try send receive close start together select repeat`.
  These begin a statement or an expression, so a name in the same position would
  be genuinely ambiguous. They can never be used as names.
* **Soft** (18): `returns fails each in times of can pure as from maybe list map
  channel shared task timeout after`. These are only meaningful in one position —
  inside a type, or after a specific keyword — so the parser recognises them there
  and accepts them as ordinary names everywhere else.

Additionally, **any** word may be used after `.` and as a named-argument label, so
`value.with(...)`, `x.type` and `f(to: 1)` all work.

The cost: `type`, `choice` and `start` are reserved, so none of them can be a
field or variable name. That is the price of `type X { ... }` being unambiguous.

### D4. Phase 6's words are not reserved yet

`serve`, `route`, `port`, `request`, `status`, `expecting`, `before`, `every`,
`anything`, `with`, `reply` and `on` are **not** keywords. They are all words a
programmer would reasonably want as a variable name, especially `request` and
`status`. Phase 6 will recognise them contextually — `serve` only at the start of
a statement, `route` only inside a `serve` block — rather than reserving them
globally.

### D5. `...` is a token

The symbol list in the specification does not mention `...`, but the pattern
grammar uses `[first, ...]`. It is lexed as one token rather than as `..` followed
by `.`, so that `[a, ...]` and the range `1..10` cannot be confused.

### D6. `{` opens a block in header positions, and after `->` and `then`

`{` is genuinely ambiguous: it opens a map literal in an expression and a block
after a header. Vaab resolves this by position.

A `{` opens a **block** when it directly follows:

* the condition of `if` or `while`, the count of `repeat`, the sequence of
  `for each`, or the subject of `match`;
* `->` in a closure;
* `then` in a match arm;
* `start`, `together`, `select`, or a function signature.

Everywhere else it opens a **map literal**. To use a map literal in a header,
wrap it in parentheses: `if (counts == {"a": 1}) { ... }`. To return a map from a
closure, likewise: `n -> ({"n": n})`.

The cost: `let x = { ... }` is always a map literal, so there is no bare
block-expression form. Nothing in the specification needs one.

### D7. `{}` is the empty map

Consistent with D6: in expression position a brace opens a map, and an empty one
is an empty map rather than an empty block.

### D8. Operator precedence, loosest to tightest

```
->                                closure body (takes everything to its right)
otherwise                         left-associative
or
and
== != < > <= >=                   does not chain
..                                does not chain
+ -
* / %
not found success failure         prefix; reach down to the comparison level
- try (receive from)              prefix; bind to a single thing
. () []                           calls, fields, indexing
```

Two prefix groups, deliberately given different reach:

* `not`, `found`, `success` and `failure` wrap a **value**, so they take a whole
  comparison-level expression. `found count + 1` wraps the sum; `not a == b`
  negates the comparison. Both read the way the English does.
* `-`, `try` and `receive from` act on a **single thing**, so they bind tightly.
  `try parse(text) + 1` adds one to what `try` produced, which is what anyone
  writing it would mean.

### D9. Comparisons and ranges cannot be chained

`low < value < high` is rejected with a message suggesting
`low < value and value < high`. Every language that quietly allows the first form
gives it a meaning nobody wants.

### D10. A bare word in a pattern is a variant if it is capitalised

`when Frozen` matches the variant; `when frozen` binds everything to a new name
called `frozen`. This reuses the capitalisation rule the language already has for
telling types from values, and it avoids the trap — present in several languages —
where a misspelled variant silently becomes a catch-all binding.

### D11. At most one problem is reported per line

One mistake usually confuses the parser about everything after it on the same
line. A missing `}` inside a string swallows the closing quote, so the text is
unclosed too; reporting both buries the cause. Vaab reports the first problem on
each line and stops there. The line-continuation rules mean a "line" is a logical
statement, not a physical one, which is the right unit.

Diagnostics are then sorted into source order, because scanning and parsing find
their problems at different times but people read a file top to bottom.

### D12. Text is scanned by hand, and holes may contain text

`"Ada is {ages.get("Ada") otherwise 0}"` is one token. A regular expression cannot
express this, so the lexer scans strings with a small recursive scanner that
counts `{}` and steps over nested strings. Without it, the very first example
anyone writes that looks up a key inside a string would fail confusingly.

A hole may not span lines, because text may not span lines.

### D13. Escapes

Vaab knows `\n`, `\t`, `\r`, `\\`, `\"`, `\{` and `\}`. Anything else is an error
rather than a silent literal, so a typo is caught rather than shipped. Only `{`
strictly needs escaping; `}` is accepted as an escape for symmetry, and a bare `}`
in text is simply a closing brace.

### D14. `else` must be on the same line as the `}` before it

`}` followed by a line ending finishes the statement, so `else` on its own line is
a parse error. The specification's examples all use `} else {`, and accepting both
would mean guessing whether a line ending is significant.

### D15. `or fails` inside a function type binds to the result

In `to(Int) returns Text or fails E`, the failure belongs to the function type's
result, not to the type surrounding it. This is the only reading that lets a
function type describe a fallible function at all.

### D16. `(A)` is `A`

A single type in parentheses is a grouping, not a one-element tuple, matching the
way `(expr)` works for values.

### D17. The CLI parses its own arguments

`clap` is not among the crates the specification lists, and the command grammar is
small. Hand-written parsing also lets the CLI's error messages be written in the
same voice as the compiler's.

Colour is on by default, off when `NO_COLOR` is set or `--no-color` is passed.
Exit codes: `0` success, `1` the program has problems in it, `2` the command
itself was wrong.

### D18. Commands that do not exist yet say which phase brings them

`vaab run` reports that it arrives in phase 3 rather than "unknown command". The
tool should never look smaller than the plan.

### D19. Parsing keeps going after an error

A failed statement, match arm or select arm is recovered from at the nearest
boundary, so one mistake does not hide the rest of the file. Recursion is capped
at 128 levels, turning a stack overflow on pathological input into a diagnostic.

### D20. Type arguments to built-in constructors are parsed as expressions

In `Channel.new(of: Text, size: 10)`, `Text` is parsed as a name, because a call
site is an expression. The type checker will interpret it in phase 4. This means
only simple type names work there for now; `Channel.new(of: list of Int)` does not
parse yet. Revisit in phase 4.

---

## Phase 2: the type checker

### D21. Every node carries a `NodeId`, and it is not printed

The compiler in phase 3 needs the type of an arbitrary expression. A `Span` cannot
be the key: `(a)` is the same node as `a` with a span covering the brackets (D16),
so several nodes can share one. So `Expr`, `Stmt` and `Pattern` each carry a
`NodeId`, handed out by a counter on the parser in the order nodes are built.

`print.rs` does not print them, because the printed tree is a test fixture and an
id would make every snapshot churn whenever the parser's order of construction
changed. `NodeId::PLACEHOLDER` exists for a node built by hand in a test.

### D22. `Checked` is written for the compiler, not for a person

Everything the checker worked out is handed on: the type of every expression, what
every name refers to, which local slot in which frame, how many frames out a
captured value lives, where each argument of a call comes from once names and
defaults are sorted out, which `.new` is automatic and which is a validated `to
new`, and the number of each choice variant.

The cost is that `Checked` is wide and will grow. The alternative — a compiler that
re-derives name resolution — is two implementations of one rule, which is how the
two drift apart.

### D23. Generics are erased, and a call site is the only place they are solved

A signature mentioning `T` holds a type *parameter*. A call swaps every parameter
for a fresh *variable* and pins those down by matching the arguments. There is no
generalisation, no constraint set, no occurs check beyond a variable declining to
be bound to itself.

This is much less than Hindley-Milner, and it is enough, because Vaab never infers
a signature. It costs the checker's ability to infer a generic function *from* a
body — which Vaab does not allow anyway.

### D24. Inference is local, and what cannot be inferred is reported at the end

`[]`, `{}` and a bare `nothing` have no type of their own. Each leaves a hole that
something later may fill — the other branch of an `if`, the annotation on the `let`
it lands in — and whatever is still open when the file has been walked is reported
then. Reporting on the spot would reject `if ready { [] } else { [1] }`, which is
sound and readable.

### D25. `+` is arithmetic only; text is joined by interpolating it

The specification gives text one way to be built: `"{greeting}, {name}"`. Making
`+` also mean concatenation would give it two, and the second one reads worse in
every case. So `"a" + "b"` is an error whose help is the interpolation to write
instead.

### D26. A closure with several parameters takes one value apart

`pairs.map((a, b) -> a + b)` is in the specification, and `.map` hands its closure
one value. So a closure whose parameter count does not match, but does match the
arity of a single tuple parameter, names the parts of that tuple instead.
`Closure::unpacks` records this, so the compiler knows to take the value apart.

### D27. A `match` covers a case only if it covers all of it

`when found x` covers the `found` half of a `maybe`; `when found 0` does not,
because it matches one value out of many. The same goes for a variant whose parts
are matched against literals, and for a guarded arm, which may not run at all.
Anything open-ended — `Int`, `Text`, a list of a particular length — has to finish
with `otherwise`.

This is deliberately simpler than a decision-tree exhaustiveness checker: it never
claims a set of literal arms is complete. The cost is an `otherwise` that a cleverer
checker could have proved unnecessary; the gain is that the rule fits in a sentence.

### D28. A built-in prelude of signatures, not a standard library

`print`, `read_file`, `.map`, `.each`, `.is_empty`, `.join`, `.get`, `.upper`,
`.contains`, `.wait`, `.update` and `.value` are a table of signatures in
`prelude.rs` with no implementations behind them. This is the smallest thing that
lets `examples/` type-check for real, and it is the same shape the standard library
will have when phase 5 puts a runtime behind it.

`read_file`'s failure is left as a type parameter, so `try read_file(path)` fits
whatever error the surrounding function declares. Phase 5 pins it to a real
`FileError` once there is a runtime behind it.

### D29. Concurrency is typed, not yet checked

`Channel.new(of: T)`, `Shared.new(v)`, `start`, `send`, `receive`, `select` and
`together` are given their types — `channel of T`, `shared T`, `task of T`, and
`maybe T` for a `receive` — and nothing more. Sendability, whether a `select` arm's
channel is still open, and what a failing task does are phase 4. This keeps
`11_concurrency.vaab` honestly checked as far as phase 2 reaches.

### D30. A type cannot be changed in place, and neither can a list

Assignment is legal only for a `let changing` *name*. `account.balance = 5` and
`numbers[0] = 3` are errors pointing at `.with(field: value)` and `.map(...)`. The
specification makes every `type` immutable; a field assignment would be the one
hole in that, and an indexed assignment would make a list a box rather than a
value.

### D31. `raw` is checked by where it is written, not by who wrote it

`Email.raw(...)` is legal only inside `Email`'s own body — a method, or its `to
new`. The checker tracks which type's body it is in and compares. A stricter rule
(only inside `to new`) was rejected: a method rebuilding a value from its own
fields has the same right to skip the checks.

### D32. `.new` and `.raw` take their fields by name; a method does not

Two fields of the same type could otherwise be swapped silently. Named arguments
are optional everywhere else, because a function's parameters are in an order the
programmer chose and can see. `.with` names fields too, and keeps whatever it does
not mention.

### D33. Declarations are collected before any body is checked

Names first, contents second, so two types may mention each other whichever is
written first, and a function may be called above its declaration. `type`, `choice`
and `ability` must be at the top level: their names are visible everywhere, so
belonging to one block would be a lie. A nested one is its own error.

### D34. `Nothing` is a type, and an `if` without an `else` has it

A branch that sometimes produces no value cannot be a value, so an `if` with no
`else` is `Nothing` and may only be used for its effect. Asked for a real type, it
reports the missing `else` and then carries on *as if* it had the type asked for,
so one missing `else` is one message rather than two.

---

## Still open

Recorded here so they are not forgotten, to be settled in the phase that needs
them.

* **`job.wait()`** — does it return `T` or `T or fails E`? To be decided in phase
  4, when tasks exist. The simplest sound option is likely `T or fails E` with the
  error being the task's failure, but it is not worth guessing before the failure
  model is built.
* **Crate names on crates.io** — `vaab-syntax`, `vaab-types`, `vaab-vm`,
  `vaab-std`, `vaab-server` and `vaab-cli` are local workspace names and have
  **not** been checked for availability. This must be done before any publish; if
  taken, the whole set gets a consistent prefix or suffix, and the outcome is
  recorded here.
* **Error conversion for `try`** — `try` currently requires the error types to
  match exactly, as the specification says. A conversion mechanism is a later
  feature.
* **Diagnostics that span two files** — every message today quotes one source. The
  moment the standard library is loaded alongside a program (phase 5), a message
  will want to point at a declaration in one file and a use in another.
* **Where a generic function's `T` may not go** — a signature may mention `T`
  anywhere, including as the type of a field or in a nested function type. Nothing
  bounds it, so `T + T` inside the body is rejected with "needs two numbers" rather
  than with something about `T` not being a number. Abilities are the obvious way
  to bound a parameter; nothing in the specification asks for it yet.


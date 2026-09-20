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

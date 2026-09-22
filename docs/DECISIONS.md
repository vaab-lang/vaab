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

### D29. Concurrency is typed *and* checked; the runtime is not here yet

`Channel.new(of: T)`, `Shared.new(v)`, `start`, `send`, `receive`, `select` and
`together` have both their types and their rules. A value crossing into a task,
down a channel, or into a `shared` must be **sendable**, and a `let changing` name
may never cross at all — that last one is the error the language exists to give.
The rules are structural and live in `checker/sendable.rs`; `Checked::tasks`
records what each `start` has to carry in, so the runtime does not work it out
again. The rules themselves are D53 onwards.

What is still open is everything that needs a machine underneath it: whether a
`select` arm's channel is still open, what a failing task does to its siblings, and
deadlock detection. Those are properties of a run, not of a program, and no amount
of reading can settle them.

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

## Phase 3: the machine

### D35. The compiler reads the checker's answers and never works one out twice

`vaab-vm` takes the `Checked` the type checker produced and treats it as settled:
which declaration a name refers to, how many frames out a local lives, which slot
a field is in, which type a method belongs to, whether a call is a constructor.
The compiler never asks a question the checker has already answered. The cost is
that the two crates are coupled through `Checked` and a change to it is a change
to both; the gain is that there is exactly one place where a name is resolved, so
the machine cannot disagree with the type errors a person was shown.

### D36. A Vaab call is a frame pushed onto the machine, never a Rust call

`Machine` holds a value stack and a frame stack, and a call to a Vaab function
pushes a frame rather than calling into Rust. How deep a Vaab program goes
therefore has nothing to do with how much Rust stack is left, and `fib(25)` — or
a recursion ten thousand deep — cannot take the process down. It also means the
machine can stop between any two instructions and be picked up later, which is
what phase 4's scheduler needs and what a Rust-recursive interpreter could not
give it without threads. The cost is that everything the interpreter would have
kept in Rust locals has to be a field of the frame or a slot on the stack.

### D37. One alias, `Ref`, stands for every shared value

Every heap value — text, a list, a map, a record, a closure — sits behind
`value::Ref<T>`, which is `std::rc::Rc<T>` today. Phase 7 adds a work-stealing
scheduler across threads, and when it does, `Ref` becomes `Arc` and nothing else
changes shape. `Value::Captured` is part of the same promise: it holds a `Cell`,
which is exactly the amount of interior mutability an `Rc` allows, and becomes a
`Mutex` beside the `Arc`. The cost is a layer of indirection where a plain `Rc` or
`Arc` could have been written; the gain is that phase 7 is a change to one file
rather than to every file that touches a value.

### D38. A local a closure reaches is put in a box where it is declared

The checker already says how many frames out each name lives. Before compiling,
the machine's compiler collects every local reached from further in than its own
frame and boxes exactly those: at the point of declaration they become a
`Value::Captured` cell, and every read and write goes through it. A closure then
captures the cell, so `numbers.each(n -> { sum = sum + n })` changes the very
`sum` the function declared rather than a copy of it.

The rejected alternative was a chain of environments, one per call, which is
simpler to write and slower everywhere: every local would cost an allocation,
whether a closure ever looked at it or not. Boxing only what is shared keeps the
common case — a local nobody captures — a plain slot on the value stack. The cost
is the pass that works out which locals those are, and one indirection on the ones
that are shared.

### D39. `.map` and `.each` are compiled as loops, not as calls into Rust

They would be easy to write as builtins that take a closure and call it. But a
builtin that calls back into Vaab would have to run a machine inside a machine,
and a machine halfway through a `map` could not then be parked — which is exactly
what phase 4 will want to do when a closure blocks on a channel. So the compiler
writes out the loop: a position, a length, an `Index`, a `Call`, a jump back.
`.map` also needs a list to gather into, so it takes one more slot than `.each`.
The cost is a longer listing for a common thing; the gain is that there is one
kind of call in the machine and every one of them can be parked.

### D40. How a value is written when it is printed

`print` shows text bare when the text *is* the value and in quotes when it sits
inside something else, so `print("a")` is `a` while `print(["a"])` is `["a"]` and
not two bare words. Lists are `[1, 2, 3]`, maps `{"Ada": 36}`, tuples
`(1, "one")` — each the way the same value is written in Vaab source, so a printed
value can be pasted back into a program. Booleans are `yes` and `no`, the words
the language uses. A `Float` keeps its point, so `5.0` does not read as a whole
number. A record is `Account(owner: "Ada", balance: 0)`: the constructor's shape
without the `.new`, because `Account.new(...)` would suggest the value was being
made rather than shown. A variant is `Colour.Red`, a `maybe` is `found 3` or
`nothing`, a result is `success 1` or `failure Trouble.Broken` — all exactly as a
pattern would spell them.

### D41. Dividing a decimal by zero is an error, like dividing a whole number

IEEE arithmetic would answer `inf`, and Vaab does not. The specification says a
program stops abnormally only on an unrecoverable runtime error and names
division by zero as one; having it be an error for `Int` and a strange value for
`Float` would make the rule something a person has to remember the exceptions to.
The cost is that a program doing deliberate IEEE arithmetic cannot; nothing in the
language asks for that yet, and a `Float` that reaches `inf` by overflow is still
left alone.

### D42. A range stands for at most ten million numbers

`1..10` is a list, so `1..1_000_000_000_000` is a list too, and building it would
take the machine down before anything could report it. Ten million is the line:
far past any loop a person writes on purpose, far short of anything that hurts.
Crossing it is a `range-too-long` report pointing at the range. The alternative —
a lazy range that a `for each` walks without building — is a better answer and a
larger one, because a range is a `list of Int` everywhere else in the language and
would stop being one. Worth revisiting when the standard library arrives.

### D43. Calls stop at ten thousand deep

Runaway recursion has to end somewhere, and ending it with a report and a trace
is the difference between a language that explains itself and one that dies. Ten
thousand is deep enough for any recursion that was going to terminate and shallow
enough to hit quickly. The trace shows the nearest six calls and the furthest
three with a count of the rest, because nobody reads the middle of ten thousand
identical lines.

### D44. A file's top-level values belong to the world, not to a frame

Locals inside a function live in slots on the value stack and vanish when the
frame does. The file's own `let`s are different: they outlive every call, a
closure at the top level can change one, and a REPL session needs them to still
be there on the next line. So they are globals, held by the `World` rather than by
any `Machine`. Phase 4 gets the same arrangement for free — many machines, one
world, one copy of the file's values, which is what "tasks see the same file"
means.

### D45. A machine is run for a budget of instructions and handed back

`Machine::resume(world, budget)` runs up to a given number of instructions and
answers `Yielded`, `Finished(value)` or `Failed(error)`. A machine that yielded
holds a borrow of nothing, so a scheduler may own as many as it likes and step
them in turn; `Budget::unlimited()` is what `vaab run` uses, since it has nothing
to share with. When a machine parks on a channel in phase 4 it will stop exactly
the same way, with one more `Step` saying what it is waiting for, and none of the
machinery here has to change.

### D46. The REPL keeps the file and checks the whole of it every line

A session is a Vaab file that grows. Each line typed is added to the source, the
whole file is parsed and checked again, and only the statements the line added are
run — the machine is started at the new statements' instruction, with the world's
values still in place. Re-checking costs nothing at the size a person types, and
buys exact agreement with `vaab run`: a session cannot accept something a file
would reject, because it *is* a file. A line that does not compile is thrown away
and the session is untouched. A line that compiles and then stops is kept, because
the statements before the fault really did run.

A line is read on when a `{`, `(` or `[` is still open, counted outside text and
comments. Anything subtler is the parser's job, and the parser gets its turn as
soon as the brackets balance.

### D47. Rounding goes to the nearest whole number, halves away from zero

`2.5.round()` is `3` and `(-2.5).round()` is `-3`. This is the rounding taught in
school, and the one a reader of plain English expects; banker's rounding is a
better default for statistics and a worse one for a language that is trying not to
surprise anybody. A result too large to be an `Int` is an overflow report rather
than a saturated number.

### D48. `.count` for things that are counted, `.length` for text

A list and a map answer `.count`, because what is being asked is how many things
there are. Text answers `.length`, because a length is a measurement, and it is
measured in characters rather than bytes: `"héllo".length` is `5`. The cost is two
names for what a shorter language would call one thing; the gain is that both read
as English where one would not.

### D49. Everything a later phase brings compiles to one instruction that names it

`read_file`, `Channel.new`, `Shared.new`, `start`, `receive`, `send`, `close`,
`select` and `together` all type-check today and none of them runs. Each compiles
to a `NotYet` instruction carrying which feature it was, so reaching one is a
report — "`read_file` arrives in phase 5", with a help line and the call that led
there — rather than a panic or a quiet wrong answer. A program may be written
against phase 4 and phase 5 now; it simply stops at the first thing that is not
built, and says so.

### D50. Every example in `examples/` is run

The test that walks `examples/` runs each `.vaab` file and snapshots what it
prints, so an example that breaks is a failing test. `11_concurrency.vaab` was
held back while the scheduler was missing; it runs now, along with
`13_worker_pool.vaab`.

### D51. The machine reports its own confusion instead of panicking

Nothing in the machine unwraps, and a state the machine cannot explain — a frame
naming a body that is not there, an instruction given a value of the wrong shape —
becomes a `confused` report saying it is a bug in Vaab rather than in the program.
A checked program cannot reach one; the point is that if a bug in the compiler
ever does, the person running the program gets a sentence and a source position
instead of a Rust backtrace. `no-arm-applied` is the same kind of net: the checker
makes every `match` exhaustive, so nothing should ever fall off the end of one.

### D52. A `return` inside a closure returns from the closure

The machine returns from the innermost frame, which is the closure. This is what
every language with closures does and the only reading that works when a closure
outlives the function that made it. The checker currently reads a `return` inside
a closure as belonging to the enclosing *function*, which makes such a `return`
practically unusable rather than wrong: the closure's body is then `Nothing` and
the types do not line up. Nobody writes one today. Settling the checker's side of
it is a phase 5 job, and the machine's answer is recorded here so that it is
settled in the direction the machine already goes.

---

## Phase 4: concurrency, the compile-time half

The rules a program has to obey to be safe. What happens during a run — a failing task's siblings, deadlock, the scheduler — is the other half, and arrives with the runtime.

### D53. Sendability is a property of the type, worked out structurally

The checker asks one question of a type — *could two tasks holding this disagree
about what it is?* — and answers it by looking at what the type holds, not by any
declaration a person writes. `Int`, `Float`, `Bool`, `Text` and `Nothing` are
sendable because nothing can change them. A list, map, tuple, `maybe T` or
`T or fails E` is sendable when everything inside it is, which follows from D30: a
list cannot be changed in place, so it is a value like any other. A declared `type`
is sendable when every field is, and a `choice` when every part of every variant is.

A marker trait in the manner of Rust's `Send` was rejected. It would be a second
thing to teach, it would need a `derive` or an explicit `can Sendable` on almost
every type, and the structural answer is the same answer with none of the writing.
The cost is that the reason a value is refused can be several steps away from the
value, which is why every refusal spells the path out: "``Job``'s field ``work`` is
``to(Int) returns Int``".

### D54. A function value is the only thing that cannot cross

Every refusal bottoms out at a function value. A closure holds on to whatever was
around it when it was written; a value of type `to(Int) returns Int` carries no
record of what that was, so Vaab cannot promise it is safe to move. Everything else
is judged by what it holds, and so every unsendable type is unsendable because a
function value is somewhere inside it.

This keeps the rule sayable in one sentence, which was the point. A *declared*
function — `to fetch(url: Text)` — is not a value and is never carried, so calling
one by name from inside a task is always fine; this is the first thing the help
text offers.

### D55. A `let changing` name may never cross, whatever it holds

Rule 2 of the specification, taken literally. It is checked on the *name*, before
the type is looked at, so even `let changing counter = Shared.new(0)` is refused —
what a `shared` holds is safe to change from any number of tasks, but the name is
not, and assigning to it would leave the tasks looking at different values. That
case gets its own help, which asks for the `changing` to be dropped rather than
offering a `Shared` the code already has.

The message is fixed by the specification, word for word, and has a unit
test guarding its wording, so a later edit cannot drift it by accident.

### D56. A closure bound straight to a fixed name is judged by its captures

"A closure that captures something unsendable cannot itself be sent" is only a real
rule if some closures *can* be sent. So a local bound straight to a closure — `let
double = n -> n * 2` — remembers which closure it holds, and a task using that name
is judged by that closure's own captures rather than refused for being a function.
`double` crosses; `n -> n + total` does not, and the error points at the line inside
the closure where `total` is read.

A function value that arrived any other way — a parameter, the result of a call, a
field, a name bound to another name — cannot be traced back to a body, and is
refused. This is sound in the direction that matters: it never lets an unsafe
closure through, it only sometimes refuses a safe one. Widening it needs a real
capture analysis over the whole module, which is not worth its weight yet.

### D57. The change handed to `.update` may not wait

`shared.update(...)` holds the value while the change runs. If the change waited —
a `receive`, a `send`, a `.wait()`, a nested `.update`, a `select`, a `together`, or
a `start` needing its turn — it would hold the value for as long as it waited, and
what it was waiting for could be waiting for that same value. That is a deadlock
written by hand, and it is visible in the source, so it is refused in the source.

`.value` is a plain read and waits for nothing. Together these are the whole
contract of a `shared`: read it whenever, change it through a change that finishes
on its own.

### D58. An unsendable `T` poisons its handle rather than cascading

`Channel.new(of: Job)` where `Job` cannot cross is reported once, and the channel is
then treated as carrying `Unknown`. Every later `send` to it is silently accepted,
because `Unknown` fits anywhere — which is the existing "one mistake stays one
mistake" rule, applied to a new place. `Shared.new` does the same. Without this, one
bad channel prints a paragraph for every `send` in the program and buries the line
that has to change.

### D59. A `together` that cannot start a task, and a `select` with no arms, are errors

`together` exists to wait for tasks and to cancel the rest when one fails. A body
with no `start` in it and no call that could contain one provably waits for nothing,
so it is dead syntax and is refused. Any call at all is taken as possibly starting a
task, which makes the check conservative: it fires only when there is nothing there.

`select` with no `when` arms has nothing that could ever become ready, with or
without an `otherwise`. Both are cheap to spot and both are almost certainly a
half-finished edit.

### D60. Timeout units are a closed list, in both spellings

`when timeout after 2 seconds` takes `millisecond`, `second`, `minute` or `hour`,
and the plural of each. The parser accepts any word there, because only the checker
knows which words are units; an unknown one is offered the nearest match by the same
`did you mean` the rest of the checker uses, so `secnds` is answered with `seconds`.

Both spellings are accepted so that `after 1 second` and `after 2 seconds` both read
as English. Nothing smaller than a millisecond is offered, because a scheduler that
preempts on an instruction count cannot honestly promise it.

### D61. A value known only by its ability is judged by every type that claims it

A parameter declared `thing: Runnable` could be any type declared `can Runnable`, so
it is sendable only when every one of them is. The note names the type in the way —
"any `Runnable` could be a `Job`, and `Job`'s field `work` is …" — because the
reader never wrote `Job` down and would otherwise have no idea where the refusal
came from.

This is the conservative answer, and it means adding an unsendable provider to an
ability can break a distant `start`. The alternative, checking at each call site
where the concrete type is known, needs flow information the checker does not keep.

### D62. What each task captures is recorded on `Checked`

`Checked::tasks` maps each `start` expression to the locals its body uses from
outside itself, in first-use order, including those reached only by a task nested
inside it. The sendability rules have to work this out anyway, and the VM needs
exactly the same list to build a task's frame, so it is published rather than thrown
away. This follows the existing `Checked` contract: the checker answers questions
once, and later phases read the answers.

A type parameter such as `T` is treated as sendable throughout. Nothing in Vaab
bounds a type parameter yet, so there is nothing to check it against; when bounds
arrive, this is the line to revisit.

---

## Phase 4: the runtime

Channels, tasks, `select`, `shared`, and a single-threaded scheduler. The
compile-time half is above; this is what happens during a run.

### D63. `job.wait()` returns `T`, not `T or fails E`

A `task of T` names the success value. `.wait()` hands that value back when the
task finishes. If the task failed or was cancelled, the wait becomes a fatal
`RuntimeError` with a call trace — the same shape as division by zero or any
other thing a running program cannot recover from. A fallible `.wait()` would need
a failure type on the task itself, which the language does not carry today, and
would tempt people to treat task failure as ordinary control flow when the
specification treats every runtime stop as final.

### D64. One OS thread, cooperative preemption

Every task is a `Machine` on a single scheduler thread. After
`PREEMPT_AFTER` instructions (10_000 by default) the running task yields so
another runnable task may go. There is no work-stealing yet; phase 7 swaps the
`Ref` cells for `Arc` and the scheduler shape stays.

### D65. `Host` and `Scheduler` are separate

`Host` owns channels, task metadata, together groups, timeouts, and the spawn
queue. `Scheduler` owns the `Machine` values and the runnable queue. The split
keeps borrow-checking honest while every task still shares one world of channels
and tasks. Phase 7 can wrap the host in a mutex without changing the op
handlers.

### D66. Deadlock is a `RuntimeError` with every parked task named

When every task is parked and nothing can wake, the run stops with a deadlock
report. The message lists each live task and where it is stuck — receiving from a
channel, sending to one, waiting on a job, inside `together`, or in a `select` —
so a person can see the cycle or the missing sender without attaching a debugger.

### D67. `select` does not treat a closed, drained channel as ready

A standalone `receive from` on a closed empty channel returns `nothing`. In
`select`, that arm is skipped so a closed inbox does not win over a timeout or
an `otherwise` arm. Use `receive` when you want the `nothing` answer from a
closed channel.

### D68. `.update` runs the closure on the scheduler thread

`.update` is synchronous with respect to the caller: it builds a one-off
`Machine` for the closure, runs it to completion on the same thread, and writes
the result back. It may not wait (D57); the runtime enforces that by refusing to
park inside the nested run. Concurrent `.update` from several tasks is serialised
by the single-threaded scheduler visiting one task at a time; phase 7 will need
a real lock around the shared cell.

---

## Phase 5: `pure` and project scaffolding

The first slice of phase 5: the checker holds `pure` functions to their promise, and `vaab new` writes the smallest project that runs today.

### D69. What `pure` means in the checker

A function declared `pure to f(...)` is checked after its body is type-checked. A
subtree walk looks for anything the promise rules out:

| Forbidden | How it is caught |
|-----------|------------------|
| `print`, `read_file` | `Resolution::Builtin` |
| `send`, `close`, `together` | statement walk |
| `receive`, `start`, `select` | expression walk |
| `Channel.new`, `Shared.new` | `Resolution::NewChannel` / `NewShared` |
| `.wait()`, `.update(...)`, `.value` on `shared` | `Resolution::BuiltinMethod` |
| A call to a declared function or method that is not `pure` | `Resolution::Function` / `Method` / `UserNew` |
| A call through a function-typed parameter or other value | callee is a `Local` holding `to(...) returns ...`, or callee type is `to(...)` with no declaration |
| Assignment to a `changing` binding declared outside the function | `locals_begin` on the function's frame |

**Allowed inside `pure`:** arithmetic, data structure work, `.map` / `.each` with
pure closures, reading immutable fields, calling other `pure` functions by name,
changing `let changing` bindings declared inside the function.

### D70. Closures inside `pure`

Closures inherit the enclosing function's rules because the walk goes into closure
bodies. A closure passed to `.map` that calls `print` is reported on the `print`,
with the `pure` signature as a second label.

### D71. `pure` passed as values

Function values carry no purity flag. A `pure` function may call another `pure`
function **by name** only. Calling a parameter or field typed `to(...) returns ...`
is refused with `not-pure` / "call a function held in a value".

### D72. `pure` methods

`pure to` on a method inside a `type` is checked the same way as a free function.

### D73. Diagnostic code and wording

- **Code:** `not-pure`
- **Message shape:** `` `{name}` is declared `pure`, so it cannot {effect} ``
- **Labels:** primary on the offending line; secondary on the `pure` signature
  ("`name` is declared `pure` here")
- **Variants:**
  - effect: `` cannot call `print` ``, `` cannot start a task ``, etc.
  - non-pure callee: `` cannot call `log` `` with "this calls `log`, which is not declared `pure`"
  - outside assignment: `` cannot change a value from outside itself ``

### D74. `vaab new <name>`

Creates a single directory with one file:

```
<name>/
  main.vaab
```

`main.vaab` contents:

```vaab
# A small Vaab program.
# Run it with: vaab run main.vaab

print("hello")
```

No package manifest, no modules, no imports — only what Vaab can run today.

- Fails with exit code **2** if the directory already exists, the name is empty, or
  the name contains a path separator.
- Succeeds with exit code **0** and prints how to run the file.
- Tested with a temp working directory and `vaab run` on the generated file.


---

## Phase 5b

### D75. `read_file` and `FileError`

`read_file(path: Text) returns Text or fails FileError` is pinned in the prelude.
`FileError` is **not** injected by the runtime — each program declares its own
choice, and the compiler looks up `FileError.NotFound` when compiling `read_file`.
A missing path becomes `failure FileError.NotFound(path)`; other I/O errors stop
the program with a confused report rather than a second variant.

### D76. `now()`

`now()` returns an `Int` of whole seconds since the Unix epoch (1970-01-01 UTC).
Sub-second precision is deferred.

### D77. `can Json`

`Json` is a built-in marker ability with no required methods. The checker
registers it before user abilities and validates that every field, variant payload
and nested container of a claiming type may itself be encoded. Serialization is
`to_json(value)` → `Text`; deserialization is deferred.

### D78. JSON encoding

Encoding uses `serde_json`. Records become JSON objects; choice variants use
externally tagged objects (`{"VariantName": {...}}`); `maybe`/`success`/`failure`
unwrap to their held value or `null`.

### D79. `not-json` diagnostic

When a type claims `can Json` but holds something that cannot be encoded, or when
`to_json` is called on a non-Json type, the checker reports code `not-json`.

### D80. First-class app I/O lives in the language

A vibe-coded app always needs the same six things: HTTP server, JSON, a database,
auth, env/secrets, and an outbound HTTP client. Those are builtins — not packages.

* `env.get` / `env.required` — process environment
* `Db.connect` / `db.execute` / `db.query` — SQLite today (Postgres URLs later);
  `$1`-style placeholders are accepted and rewritten for SQLite
* `request.who` — bearer token `id|email|hmac` verified with `AUTH_SECRET`; needs a
  declared `type User { id: Text, email: Text }` and `choice AuthError { Unauthorized }`
* `http.get` / `http.post` — outbound HTTP, returning response text

Programs declare the error choices (`EnvError`, `DbError`, `AuthError`, `HttpError`)
the same way they declare `FileError` for `read_file`. Vendor integrations
(Stripe, email, AI) stay as riffs.

### D81. JIT before AOT — tiered Cranelift, not ahead-of-time compilation

Vaab speeds up hot numeric code with a **tier-1 JIT** (Cranelift), not AOT
compilation to native binaries.

**Why JIT**

* **Fast startup** — `vaab repl`, `vaab serve`, and riff-linked projects must
  start instantly; AOT would add a compile step before the first request.
* **Fits the VM** — channels, tasks, `select`, and I/O stay on the bytecode
  interpreter; only pure, capture-free `Int` bodies get native code.
* **Modern tiered model** — tier 0 (interpreter) always works; tier 1 (Cranelift)
  kicks in after warmup. Tier 2 (recursive bodies with depth checks) is the next
  step for things like `fib`.

**Why not AOT (yet)**

* Whole-program AOT fights Vaab's concurrency model and dynamic loading.
* Steady-state speed is better with AOT, but cold start and incremental dev are
  worse — wrong trade for v0.1.

**Tier-1 scope (v0.1)**

* Pure `Int` locals, arithmetic, compares, jumps, return.
* No captures, no calls to other Vaab functions, no self-recursion.
* No `divide` / `remainder` — the interpreter reports `DividedByZero` with a
  proper stack trace; native `sdiv` would trap instead.

The cost: recursive and multi-function hot paths (e.g. `fib`, call chains) stay
on the interpreter until tier 2.

### D82. Mutable objects are `cast`; inheritance is `entertains`

Ruby-style classes are called **casts**. The word fits the language's voice
(`riff`, `type`, `choice`, `ability`) and the theatre metaphor: a cast is a
role with state and behaviour; each instance is someone playing it.

Inheritance uses **`entertains`**, not `extends` or `from`:

```vaab
cast Animal {
    changing name: Text
    to speak() returns Text = self.name
}

cast Dog entertains Animal {
    changing breed: Text
    to speak() returns Text = "{self.name} ({self.breed})"
}
```

**`type` stays immutable.** Casts are the mutable layer. A `type` holds data
that changes only through `.with(...)`; a `cast` holds `changing` fields that
methods may assign to in place.

**Abilities still use `can`.** A cast may declare `can Describable` the same way
a type does. Inheritance (`entertains`) and interface provision (`can`) are
separate: a `cast Dog entertains Animal can Runnable` both extends and
implements.

**Construction stays `.new`.** Every cast gets `Cast.new(...)` with named
arguments, matching types. Validated construction uses `to new` and `Cast.raw`
inside the body, same as today.

**Class methods use `to self.method`.** Instance methods are plain `to foo(...)`.
Methods on the cast itself — factories, finders — are `to self.open(...)`.

The cost: two ways to define structured objects (`type` and `cast`). The benefit:
immutability stays the default for data; mutation is explicit and theatrically
named.

---

## Still open

Recorded here so they are not forgotten, to be settled in the phase that needs
them.

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


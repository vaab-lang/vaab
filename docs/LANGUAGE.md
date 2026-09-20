# The Vaab language

Vaab (said "vibe") is a statically typed language that reads like English. Words
are preferred to symbols, every function signature is fully annotated, and
concurrency is safe by construction.

This document is the living specification. Where it says **(phase N)**, the
feature is designed but not yet built; everything else works today.

**Status:** phases 1 to 3 are complete. Everything below parses, and `vaab check`
will type-check it: signatures, generics, `maybe`, failures, abilities, and a
`match` that has to cover every case. Sequential programs then run: `vaab run
file.vaab` runs a file, and `vaab repl` opens a session that grows a line at a
time.

Concurrency is here. The compile-time rules — which values may cross into a task
or down a channel, and the `let changing` variable a task may never use — make a
data race a compile error, and channels, tasks, `select`, `together` and `shared`
run on a single-threaded scheduler with cooperative preemption.

Design choices this document does not explain — the ones the original
specification left open — are recorded in [DECISIONS.md](DECISIONS.md).

---

## Contents

1. [Layout](#layout)
2. [Values](#values)
3. [Text](#text)
4. [Functions](#functions)
5. [Types, choices and abilities](#types-choices-and-abilities)
6. [Missing values and errors](#missing-values-and-errors)
7. [Control flow](#control-flow)
8. [Concurrency](#concurrency)
9. [Standard library](#standard-library)
10. [When a program stops](#when-a-program-stops)
11. [Web server](#web-server-phase-6)
12. [Reserved words](#reserved-words)
13. [Symbols](#symbols)

---

## Layout

Files use the `.vaab` extension.

`#` starts a comment, which runs to the end of the line.

Newlines end statements. There are no semicolons. A line continues onto the next
when:

* it ends with an operator, a comma, a colon, or an opening bracket;
* the next line starts with `.`, which is how method chains are broken up;
* it is inside `(` or `[`, so arguments and list items may be spread out freely.

```vaab
let total = 1 +
    2 +
    3

let shouted = names
    .map(name -> name.upper())
    .join(", ")

greet(
    "Ada",
    greeting: "hi"
)
```

Blocks use `{ }`. The last expression in a block is its value.

---

## Values

```vaab
let name = "world"                # fixed, inferred as Text
let changing count = 0            # changeable, inferred as Int
let total: Int = 0                # an explicit type, when it helps a reader
count = count + 1                 # assignment is only allowed for `changing`
```

Local types are inferred. Signatures never are. There are no implicit
conversions, and there is no null.

### Built-in types

| Type | Meaning |
| --- | --- |
| `Int` | a whole number (64-bit) |
| `Float` | a decimal number (64-bit) |
| `Bool` | `yes` or `no` |
| `Text` | text |
| `Nothing` | the unit type, returned by a function with no `returns` |
| `list of T` | a list |
| `map of K to V` | a map |
| `maybe T` | a value that might be absent |
| `T or fails E` | a result that might fail |
| `(A, B)` | a tuple |
| `channel of T` | a channel |
| `task of T` | a running task |
| `shared T` | state shared safely between tasks |
| `to(A, B) returns C` | a function |

Type parameters are single capital letters (`T`, `K`, `V`) and are implicitly
generic.

### Literals

```vaab
let numbers = [1, 2, 3]           # list
let ages = {"Ada": 36}            # map
let pair = (1, "a")               # tuple
let span = 1..10                  # range, inclusive at both ends
let population = 1_000_000        # `_` groups digits
```

---

## Text

Text uses double quotes and always interpolates. `{expr}` is a hole, filled in
when the text is built.

```vaab
print("{greeting}, {name}")
print("Ada is {ages.get("Ada") otherwise 0}")
```

A hole may hold any expression, including text with holes of its own. Text stays
on one line.

Escapes: `\n`, `\t`, `\r`, `\\`, `\"`, `\{`, `\}`. Write `\{` for a literal
brace.

---

## Functions

The keyword is `to`.

```vaab
to greet(name: Text, greeting: Text = "hello") returns Text {
    return "{greeting}, {name}"
}

to double(n: Int) returns Int = n * 2       # one-line form

to announce(message: Text) {                # no `returns` means `Nothing`
    print(message)
}
```

Every parameter is annotated. A parameter with a default may be left out at the
call site, and any argument may be passed by name:

```vaab
greet("Ada", greeting: "hi")
```

`pure to f(...)` asks the compiler to guarantee that `f` has no side effects, no
I/O, no `shared`, and no channel operations, and only calls other pure functions.

The checker enforces this. A `pure` function may change `let changing` locals it
declares itself, but not bindings from outside. It may call another function only
when that function is also declared `pure` and is reached by name; a function held
in a value carries no record of purity and cannot be called from inside a `pure`
body. Closures written inside a `pure` function are held to the same rules.

### Closures

```vaab
numbers.map(n -> n * n)
pairs.map((a, b) -> a + b)
items.each(item -> { print(item) })         # block body
```

### Where `to` means what

`to` at the start of a statement, or at the start of a member inside a `type` or
`ability` body, defines a function. It is a connector inside `send <value> to
<channel>` and inside `map of K to V`. In a type position, `to(A) returns B` is a
function type. `to` is reserved and can never be a name.

---

## Types, choices and abilities

A `type` holds fields and is **immutable**. `value.with(field: new)` returns a
changed copy.

```vaab
type Account {
    owner: Text
    balance: Int = 0                        # default value

    to deposit(amount: Int) returns Account or fails AccountError {
        if amount <= 0 { return failure AccountError.InvalidAmount(amount) }
        return success self.with(balance: self.balance + amount)
    }
}
```

### Construction is always `.new`

Every type gets an automatic `new` built from its fields. There is no bare
`Type(...)` call syntax anywhere in the language.

```vaab
let account = Account.new(owner: "Ada")
let funded = Account.new(owner: "Ada", balance: 50)
```

Arguments are named. Fields with defaults may be omitted; a missing field without
one is a compile error: `Account.new is missing `owner`.`

A type may define its own validated constructor with `to new`, which returns
`Type or fails E` and replaces the automatic one. `Type.raw(...)` is the automatic
field-by-field constructor, and is usable **only inside the type's own body**, so
a validated type can never be built in an invalid state from outside.

```vaab
type Email {
    address: Text

    to new(address: Text) returns Email or fails EmailError {
        if not address.contains("@") { return failure EmailError.Invalid }
        return success Email.raw(address: address)
    }
}
```

Built-in handles use the same form:

```vaab
let inbox = Channel.new(of: Text, size: 10)   # channel of Text
let counter = Shared.new(0)                   # shared Int, inferred
```

Choice variants stay bare: `AccountError.InvalidAmount(5)`. So do `found x`,
`success x`, `failure e` and `nothing`. `.new` is only for types and built-in
handles.

### Choices

A `choice` is a sum type. `match` on one must be exhaustive.

```vaab
choice AccountError {
    InvalidAmount(amount: Int)
    Frozen
}
```

### Abilities

An `ability` is an interface. `type X can A` provides it. A parameter may be typed
as an ability.

```vaab
ability Describable {
    to describe() returns Text
}

type Account can Describable {
    to describe() returns Text = "{self.owner} has {self.balance}"
}

to announce_all(things: list of Describable) { ... }
```

---

## Missing values and errors

No null. No exceptions.

A `maybe T` is `found x` or `nothing`. A `T or fails E` is `success x` or
`failure e`.

```vaab
let age = ages.get("Ada")                 # maybe Int

match age {
    when found value then print("Ada is {value}")
    when nothing     then print("unknown")
}

let safe = ages.get("Ada") otherwise 0    # unwrap with a fallback
```

`try expr` hands a failure straight back to the caller, so the happy path stays
flat. It is only legal inside a function returning `or fails E`, where the error
types match exactly.

```vaab
to read_config(path: Text) returns Config or fails FileError {
    let text = try read_file(path)
    let config = try parse_config(text)
    return success config
}
```

`expr otherwise fallback` works on both `maybe` and `or fails`.

Assigning a `maybe` to a plain type is an error:
`let x: Int = ages.get("Ada")` fails with *expected Int, found maybe Int*.

`otherwise` is contextual: it is the fallback operator in an expression, and the
catch-all arm inside `match` and `select`. The two never meet, because an arm
always begins a line.

---

## Control flow

Conditions are `Bool` only. There is no truthiness. Boolean operators are words:
`and`, `or`, `not`.

```vaab
if a > b { ... } else if a == b { ... } else { ... }     # `if` is an expression

for each item in items { ... }
for each n in 1..10 { ... }                              # inclusive
while condition { ... }
repeat 4 times { ... }

match value {
    when 0            then "zero"
    when n if n < 0   then "negative"
    when [first, ...] then "starts with {first}"
    when found x      then "got {x}"
    otherwise         then "something else"
}
```

In a pattern, a **capitalised** bare word is a choice variant and a **lowercase**
one binds a new name. `when Frozen` matches the variant; `when frozen` matches
anything.

Comparisons do not chain: write `low < value and value < high`.

---

## Concurrency

Everything here is checked and runs. A failing task inside `together` cancels its
siblings. When every task is blocked with no way to wake, the run stops with a
deadlock report that names where each task is stuck.

```vaab
let inbox = Channel.new(of: Text, size: 10)   # bounded; size 0 is a rendezvous
send "ping" to inbox                          # blocks if full; type-checked
let message = receive from inbox              # maybe Text: nothing once drained
close inbox
for each item in inbox { ... }                # loops until closed

let job = start { slow_calculation(42) }      # task of Int
let answer: Int = job.wait()

together {                                    # waits for every `start` inside
    for each url in urls {
        start { fetch(url) }
    }
}                                             # a failure cancels the siblings

select {
    when receive from inbox as message { handle(message) }
    when receive from quit             { return }
    when timeout after 2 seconds       { print("quiet") }
    otherwise                          { print("nothing ready") }   # non-blocking
}

let counter = Shared.new(0)                   # shared Int
counter.update(n -> n + 1)                    # atomic read-modify-write
print(counter.value)
```

### Sendability

A task does not share the stack it was started from, so every value a `start { ... }`
body uses from outside itself has to be **carried in**. A value may be carried when
two tasks holding it can never disagree about what it is. Vaab calls that
*sendable*, works it out from the type, and refuses the rest at compile time.

| | sendable when |
| --- | --- |
| `Int`, `Float`, `Bool`, `Text`, nothing | always |
| a list, a map, a tuple, `maybe T`, `T or fails E` | everything inside is |
| a `type` | every field is |
| a `choice` | every part of every variant is |
| an ability, as in `thing: Runnable` | every type that `can` it is |
| `channel of T`, `shared T`, `task of T` | `T` is |
| a closure | never, unless the name was bound straight to it |

The one thing that is never sendable is a **function value**. A closure holds on to
whatever was around it when it was written, and Vaab cannot look inside that, so
every other refusal is really a function value found somewhere within:

    a task cannot be given `job`
      this task uses `job`, which is a `Job`
      note: `Job`'s field `work` is `to(Int) returns Int`; a function value holds on
            to whatever was around it when it was written, which Vaab has no way to
            look inside

A closure is sendable when the name was bound straight to it, and everything it
reaches for is sendable too — `let double = n -> n * 2` may cross, and
`n -> n + total` may not.

A **`let changing` name may never be carried into a task**, whatever it holds:

> `` `total` can change, so a task cannot use it. Use `Shared` instead. ``

    help: use `Shared` instead: `let total = Shared.new(...)`, and change it
          inside the task with `total.update(value -> ...)`
    note: two tasks could change `total` at the same moment, and then neither
          would see what the other did; if the task only needs the value `total`
          holds right now, copy it into a fixed name first and use that

### Sharing changing state

`shared T` is the sanctioned way, and the `T` inside it must itself be sendable.
`.value` reads what is held, and waits for nothing. `.update(change)` runs `change`
on the held value with every other task kept out, and hands back what it returns.
Because the value is held for as long as the change runs, **the change may not wait
for anything**: no `receive`, `send`, `.wait()`, `select`, `together`, `start`, or
second `.update`. Work the new value out first and hand the finished value in.

### `together`, `select` and `receive`

`together` must be able to start a task — a body with no `start` and no call is
refused, because it would wait for nothing. `select` must have at least one `when`
arm, with or without an `otherwise`. A `timeout after` takes `milliseconds`,
`seconds`, `minutes` or `hours`, or the singular of any of them.

`receive from` needs a channel; reaching for a `shared` there is answered with the
way to read one, since mistaking the two is an easy thing to do.

### Runtime

Tasks are VM structures, not OS threads: each has its own value stack and call
frames, and blocking on a channel parks the task in that channel's wait queue. The
scheduler preempts after a fixed number of instructions so no task can starve the
others. If every task is blocked and none can be woken, the program exits with a
deadlock report listing where each task is stuck.

---

## Standard library

The full standard library — time, file I/O and json among it — is the rest of
**phase 5**. `pure` enforcement is here; what exists today is the handful of things
a small program genuinely needs, and all of it runs.

### Printing

```vaab
print("hello")              # Text is shown as it is
print([1, 2, 3])            # [1, 2, 3]
print({"Ada": 36})          # {"Ada": 36}
print((1, "one"))           # (1, "one")
print(found 3)              # found 3
print(Colour.Red)           # Colour.Red
```

`print` takes a value of any type and writes one line. Text is shown bare when it
*is* the value and in quotes when it sits inside something else, so a list of two
words does not read as two bare words. See D40 in [DECISIONS.md](DECISIONS.md).

### Text

| | |
|---|---|
| `text.upper()` | the same text in capitals |
| `text.lower()` | the same text in small letters |
| `text.contains(part: Text)` | whether `part` appears anywhere in it |
| `text.is_empty` | whether it has no characters |
| `text.length` | how many characters it has |

```vaab
print("ada@example.com".contains("@"))   # yes
print("héllo".length)                    # 5, counted in characters
```

### Lists

| | |
|---|---|
| `items.map(change)` | a new list, each item put through `change` |
| `items.each(do)` | runs `do` on every item and gives back nothing |
| `items.is_empty` | whether it holds nothing |
| `items.count` | how many items it holds |
| `items.first` | `maybe` the first item, since the list may be empty |
| `items.join(separator: Text)` | one piece of text, for a list of Text |

```vaab
print([1, 2, 3].map(n -> n * 2))          # [2, 4, 6]
print(["ada", "alan"].map(n -> n.upper()).join(", "))
```

### Maps

| | |
|---|---|
| `entries.get(key)` | `maybe` the value, since the key may not be there |
| `entries.is_empty` | whether it holds nothing |
| `entries.count` | how many entries it holds |
| `entries.keys` | the keys, in the order they were first put in |

```vaab
let ages = {"Ada": 36, "Alan": 41}
for each name in ages.keys {
    print("{name} is {ages.get(name) otherwise 0}")
}
```

### Numbers

| | |
|---|---|
| `n.abs()` | the size of a whole number, without its sign |
| `n.min(other: Int)` | the smaller of two whole numbers |
| `n.max(other: Int)` | the larger of two whole numbers |
| `n.to_float()` | the same number as a Float |
| `x.abs()` | the size of a decimal, without its sign |
| `x.round()` | the nearest whole number, halves away from zero |

Vaab never turns an `Int` into a `Float` behind your back, so `.to_float()` is
how a calculation mixes them. A method binds tighter than a minus sign, so the
size of a negative literal is written `(-7).abs()`.

```vaab
print(3.to_float() / 2.0)   # 1.5
print(2.5.round())          # 3
```

### File I/O

Programs declare a `FileError` choice; `read_file` fails with it when a path is
missing. Other I/O problems stop the program with a report.

```vaab
choice FileError {
    NotFound(path: Text)
}

match read_file("config.txt") {
    when success text then print(text)
    when failure FileError.NotFound(path) then print("no file at {path}")
}
```

### Time

| | |
|---|---|
| `now()` | whole seconds since 1970-01-01 UTC |

### JSON

A type that `can Json` may be passed to `to_json`, which gives back `Text`.

```vaab
type Person can Json {
    name: Text
    score: Int
}

print(to_json(Person.new(name: "Ada", score: 36)))
```

Built-in scalars, lists, maps with `Text` keys, tuples, records, and choices whose
parts all `can Json` may be encoded. Functions, channels, tasks and other values
that cannot be written as JSON are rejected by the checker.

---

## When a program stops

A running program stops for exactly one reason: something happened that has no
answer. There is nothing to catch and nothing to recover from — a failure a
program is meant to handle is a `failure`, which is part of a function's type.

| | |
|---|---|
| a sum, difference, product, division or negation too large for an `Int` | Vaab checks every calculation instead of wrapping round |
| dividing, or taking a remainder, by zero | for whole numbers and decimals alike |
| reading a list at a position it has not got | positions start at 0 |
| a range standing for more than ten million numbers | a range is a list, so it is built all at once |
| calls ten thousand deep | runaway recursion is a report rather than a crash |
| reaching something a later phase brings | named with the phase that brings it |

Each is reported the way a type error is, with the line it happened on and the
calls that led there:

```
[divide-by-zero] Error: this divides by zero
   ╭─[ rates.vaab:4:40 ]
   │
 4 │     to per(minutes: Int) returns Int = self.per_hour / minutes
   │                                        ───────────┬───────────
   │                                                   ╰───────────── the right-hand side is 0
   │
   │ Help: check the divisor first, as in `if count != 0 { ... }`
───╯
Trace:
  in `Rate.per` at rates.vaab:4:40
  in the top level at rates.vaab:6:7
```

## Web server (phase 6)

```vaab
serve on port 8080 {
    before every request { print("{request.method} {request.path}") }

    route get "/tasks/{id: Int}" {            # typed path parameter
        let task = try find_task(id)
        reply with task
    }

    route post "/tasks" expecting NewTask as new_task {   # invalid body => 400
        reply with created status 201
    }

    when anything fails with ApiError as error {
        reply explain(error)
    }
}
```

Every route body has type `Response or fails ApiError`. Types that `can Json` are
serialised and deserialised automatically. Each request runs as its own task.

---

## Reserved words

Vaab reserves as little as it can. A word is reserved only when it begins a
statement or an expression, because that is the only place where treating it as a
name would be ambiguous.

**Reserved** — these can never be names:

```
to        let       changing  return    if        else      while     for
match     when      then      otherwise type      choice    ability   self
and       or        not       yes       no        found     nothing   success
failure   try       send      receive   close     start     together  select
repeat
```

**Recognised in position** — these are keywords where they are meaningful, and
ordinary names everywhere else, so `let list = [1, 2]` is fine:

```
returns   fails     each      in        times     of        can       pure
as        from      maybe     list      map       channel   shared    task
timeout   after
```

Any word at all may be used after `.` and as a named-argument label, so
`value.with(...)` and `f(type: 1)` both work.

---

## Symbols

```
->  =  ==  !=  <  >  <=  >=  +  -  *  /  %  .  ,  :  (  )  [  ]  {  }
..     ranges
...    "and the rest", in list patterns
```

That is the whole set. Everything else is a word.

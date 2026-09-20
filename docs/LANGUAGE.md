# The Vaab language

Vaab (said "vibe") is a statically typed language that reads like English. Words
are preferred to symbols, every function signature is fully annotated, and
concurrency is safe by construction.

This document is the living specification. Where it says **(phase N)**, the
feature is designed but not yet built; everything else works today.

**Status:** phase 1 is complete. Everything below parses, and `vaab parse`
will show you the tree. Nothing is type-checked or run yet.

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
9. [Web server](#web-server-phase-6)
10. [Reserved words](#reserved-words)
11. [Symbols](#symbols)

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

`pure to f(...)` **(phase 5)** asks the compiler to guarantee that `f` has no side
effects, no I/O, no `shared`, and no channel operations, and only calls other pure
functions.

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

Data races are compile errors. The rules:

1. Fixed (`let`) values of sendable types may be captured by `start` blocks and
   sent over channels.
2. `let changing` variables may **never** be captured by a `start` block.
3. `shared T` and `channel of T` are sendable handles, and the only ways to share
   changing state. The `T` inside must itself be sendable.

Breaking rule 2 reads:

> `` `total` can change, so a task cannot use it. Use `Shared` instead. ``

### Runtime

Tasks are VM structures, not OS threads: each has its own value stack and call
frames, and blocking on a channel parks the task in that channel's wait queue. The
scheduler preempts after a fixed number of instructions so no task can starve the
others. If every task is blocked and none can be woken, the program exits with a
deadlock report listing where each task is stuck.

---

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

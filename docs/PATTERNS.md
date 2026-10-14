# Design patterns in Vaab

Half the classic patterns dissolve in Vaab. The other half fit in a few
lines. This guide walks both kinds, one pattern at a time: a real-world
example, the idea in plain words, the Vaab code, and when to reach for it.

Every program below runs. Each section names its file — try it with
`vaab run examples/<file>` and read the output it prints.

The pattern names and the real-world framing follow the Gang of Four
catalog as retold by
[design-patterns-for-humans](https://github.com/nilbuild/design-patterns-for-humans).
The code is original: the PHP versions there answer "how in an OOP
language", and this guide answers "how in Vaab".

## Patterns that dissolve

Some patterns are workarounds for what a language cannot say. Vaab can
say these things directly, so the pattern never gets written:

| Pattern | What Vaab says instead |
| --- | --- |
| Iterator | `items.each(...)` and `items.map(...)` — traversal is built in |
| Prototype | immutable `type` plus `.with(field: value)` — every value is a safe original |
| Builder | named arguments with defaults, plus `to new` for validated construction |
| Singleton | don't — and when shared state is honestly needed, `Shared.new(...)` says so out loud |
| Factory Method | `factory` methods on a `cast`, or `to new` replacing the automatic constructor |

The rest of this guide shows the ones worth writing, and two dissolves
worked out in full so you can feel the difference.

---

## Strategy

Real world example

> Airport security has lanes: crew, frequent flyer, everyone else. You
> pick a lane at the entrance. The checkpoint works the same way no
> matter which lane you chose.

In plain words

> Hand the decision in from the outside instead of branching on it
> inside. The caller picks the strategy; the function just runs it.

Vaab code (`examples/21_strategy.vaab`)

```vaab
ability Discount {
    to apply(total: Int) returns Int
}

type MemberDiscount can Discount {
    to apply(total: Int) returns Int = total - total / 10
}

type SaleDiscount can Discount {
    cut: Int = 5

    to apply(total: Int) returns Int = total - self.cut
}

checkout(total: Int, discount: Discount) returns Int {
    return discount.apply(total)
}

print(checkout(100, MemberDiscount.new()))        # 90
print(checkout(100, SaleDiscount.new()))          # 95
print(checkout(100, SaleDiscount.new(cut: 20)))   # 80
```

When to use it

> When the caller knows something the function shouldn't — which
> discount, which shipping rule, which scoring formula. Put the varying
> part behind an ability and pass it in. If there are only two
> strategies and they never change, a plain `if` is fine; don't build
> the machinery until the third one shows up.

---

## State

Real world example

> A traffic light cycles red, green, yellow. The light itself decides
> what comes next — drivers just read what's showing.

In plain words

> Let the value say what it is, and let a match say what follows.
> There is no state object holding a flag; the flag would just be a
> worse version of the value.

Vaab code (`examples/22_state.vaab`)

```vaab
choice Light {
    Red
    Yellow
    Green
}

advance(light: Light) returns Light {
    match light {
        when Red then { return Light.Green }
        when Green then { return Light.Yellow }
        when Yellow then { return Light.Red }
    }
}
```

When to use it

> When a value moves through a fixed set of phases — lights, order
> status, connection lifecycle. The payoff is the exhaustiveness check:
> add a fourth light and the compiler refuses to run until `advance`
> (and every other match) handles it. If the transitions carry data or
> guards, give the variants fields and match on those.

---

## Observer

Real world example

> A newsroom wire: reporters file stories once, and every desk reading
> the wire sees them. Nobody phones every desk to announce each story.

In plain words

> One side sends, the other sides read. A channel is the subscriber
> list — except nobody maintains it, and nobody can forget to update it.

Vaab code (`examples/23_observer.vaab`)

```vaab
watch(feed: channel of Text) returns Int {
    let changing count = 0
    for each headline in feed {
        print(headline)
        count = count + 1
    }
    return count
}

let feed = Channel.new(of: Text, size: 4)
let subscriber = start { watch(feed) }

send "launch day" to feed
send "version 0.1.3" to feed
send "patterns guide" to feed
close feed

let seen: Int = subscriber.wait()
print(seen)
```

When to use it

> When producers and consumers should not know about each other —
> event feeds, progress reporting, fan-out work. Size the channel for
> the burst you expect; size 0 is a rendezvous where every send waits
> for a reader. If there is exactly one reader doing one thing, skip
> the channel and call the function.

---

## Command

Real world example

> A TV remote: each button is a lump of intent — power, volume, mute.
> The remote doesn't know how televisions work; it just delivers the
> button that was pressed.

In plain words

> Store the action as data, apply it with a match. Undo is just
> remembering what things looked like before.

Vaab code (`examples/24_command.vaab`)

```vaab
choice LightCommand {
    TurnOn
    TurnOff
}

cast Remote {
    changing lit: Bool = no
    changing previous: Bool = no

    press(command: LightCommand) {
        self.previous = self.lit
        match command {
            when TurnOn then { self.lit = yes }
            when TurnOff then { self.lit = no }
        }
    }

    undo() {
        self.lit = self.previous
    }
}
```

When to use it

> When actions need to travel — queued, logged, retried, undone. A
> choice value crosses a channel or sits in a list far more happily
> than a callback. When the action never travels and never undoes,
> call the function and skip the envelope.

---

## Builder, dissolved

Real world example

> A sandwich counter: bread, cheese, tomato, size. You name what you
> want; everything else stays the house default. Nobody hands you a
> fourteen-argument form.

In plain words

> The pattern exists to fight telescoping constructors. Named
> arguments with defaults end the fight: every flavor is the same
> `.new` call with different options filled in.

Vaab code (`examples/25_builder.vaab`)

```vaab
type Sandwich {
    bread: Text = "rye"
    cheese: Bool = no
    tomato: Bool = no
    size: Int = 12
}

print(describe(Sandwich.new()))
# 12 inch on rye
print(describe(Sandwich.new(size: 6, cheese: yes)))
# 6 inch on rye with cheese
```

When the pattern still earns its keep

> When construction itself is multi-step or fallible — look something
> up, validate, then build. That is what `to new` is for: it replaces
> the automatic constructor with one that returns `Type or fails E`,
> so an invalid value can never be built from outside (see
> `examples/06_validated_construction.vaab`).

---

## Prototype, dissolved

Real world example

> Dolly the sheep: start from an existing sheep, change the name, keep
> everything else. Cloning with edits, not construction from scratch.

In plain words

> Immutable values are already safe originals. Copy one with `.with()`
> and only the named fields change — no clone keyword, no ceremony
> around getting the copy right.

Vaab code (`examples/26_prototype.vaab`)

```vaab
type Sheep {
    name: Text
    category: Text = "Mountain Sheep"
}

let jolly = Sheep.new(name: "Jolly")
let dolly = jolly.with(name: "Dolly")

print(jolly.name)       # Jolly — the original is untouched
print(dolly.name)       # Dolly
print(dolly.category)   # Mountain Sheep
```

When the pattern still earns its keep

> When the starting point is expensive to build — a parsed config, a
> warmed cache — build it once and `.with()` your way to the variants.
> For plain data, you were already doing this; now it has a name.

---

## Notes on the rest

A few patterns come up often enough to deserve a sentence each:

- **Adapter / Bridge / Decorator / Proxy** — all "talk to this through
  that" shapes. In Vaab that is a type holding the other type and
  providing the same ability. Write the wrapper, forward the methods,
  move on.
- **Facade** — a function with a friendly name over a subsystem that
  isn't. Vaab functions are cheap; facades should be too.
- **Factory / Abstract Factory** — `to new` for validated construction,
  `factory` methods on a `cast` for families of related objects (see
  `examples/15_first_class_app.vaab` for a `Database` cast that opens,
  migrates, and queries).
- **Mediator** — a channel with a task behind it. The chat-room example
  is a `Channel.new(of: Text, ...)` plus whoever reads it.
- **Memento** — keep the old immutable value around. Undo is a variable
  holding yesterday's value, which is exactly what `Remote.previous`
  does in `examples/24_command.vaab`.
- **Template Method** — an ability method that calls other ability
  methods; each type fills in the steps. The skeleton is written once,
  in plain words.
- **Composite / Flyweight / Visitor / Iterator / Chain of
  Responsibility** — lists, maps, and `match` cover these ground so
  thoroughly that writing them as "patterns" would only add fog. A
  chain of handlers is a list of abilities tried in order; a visitor is
  a match the compiler checks for you.

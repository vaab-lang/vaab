# Plan: real-time backends in Vaab

Goal: Vaab can run a long-lived, streaming backend like the Python
`pumpfun-quant-bot` — subscribe to a Solana RPC WebSocket, decode binary program
events, compute rolling statistics, persist to SQLite, and push live updates to a
browser dashboard — while it serves HTTP routes in the same process.

The work is split into milestones, ordered so that each one unblocks the next. Every
milestone ends with green `cargo test`, reviewed `insta` snapshots, a runnable
example under `examples/`, and its decisions recorded in `docs/DECISIONS.md`.
Duplicate numbers already exist up to D86, so start at **D87**.

---

## How a builtin is added today (the path every milestone follows)

Use `http.get` as the template:

1. **Checker** — `crates/vaab-types/src/checker/member.rs` resolves `http.get` to a
   `Member::Callable` with a `Signature` and `Resolution::BuiltinMethod("http_get")`.
   Methods on a built-in receiver type (`Float.abs`, `Int.min`) live in
   `crates/vaab-types/src/prelude.rs` instead.
2. **Purity** — `crates/vaab-types/src/checker/pure.rs` forbids the new builtin
   inside `pure` functions if it does I/O or reads the clock.
3. **Sendability** — any new value type gets a rule in
   `crates/vaab-types/src/checker/sendable.rs`.
4. **Compiler** — `crates/vaab-vm/src/compile/expr.rs` maps the resolution to an
   `Op` in `crates/vaab-vm/src/bytecode.rs`.
5. **Machine** — `crates/vaab-vm/src/machine.rs` executes the `Op`. I/O helpers
   live in `crates/vaab-vm/src/app_io.rs`. Failures become the program's declared
   error choice via `app_io::failure_message`.
6. **Messages** — every new diagnostic goes in
   `crates/vaab-types/src/messages.rs` with a snapshot test, and must say what to
   type next.
7. **Docs** — `docs/LANGUAGE.md` (surface), `docs/DECISIONS.md` (why), an example.

New value types also need: a `Type` variant (`crates/vaab-types/src/types.rs`), a
`Value` variant (`crates/vaab-vm/src/value.rs`), printing, equality, and the
`Ref`/`Arc` treatment that `parallel.rs` expects.

---

## M1. Time and maths

**Why:** signal code needs milliseconds and real floating-point maths. Today `now()`
returns whole seconds, and `Float` has only `abs` and `round`.

Surface:

```vaab
let started = Clock.millis()          # Int, ms since epoch
let tick = Clock.monotonic_millis()   # Int, for measuring durations
sleep(milliseconds: 250)              # parks the task, never the thread

let z = (x - mean) / spread
x.sqrt()  x.ln()  x.exp()  x.pow(2.0)  x.floor()  x.ceil()
x.min(y)  x.max(y)  x.is_finite()  x.to_int()
Float.pi  Float.infinity
```

Lists of numbers: `.sum()`, `.min()`, `.max()`, `.mean()` (`Float or nothing` on
empty), `.sorted()`.

Work:
- `Clock.millis` / `Clock.monotonic_millis` as builtins; keep `now()` unchanged.
- `sleep(milliseconds:)`: reuse whatever deadline mechanism already backs
  `when timeout after` in `select` (the `Host` owns timeouts, D65), exposed as a
  standalone `WaitSite::Sleep { until }` in `crates/vaab-vm/src/concurrency.rs` and
  honoured by both `concurrency.rs` and `parallel.rs`. It must not block the OS thread.
- Float and list methods in `prelude.rs` + `Op`s.
- `pure` rules: maths is pure; `Clock.*` and `sleep` are not.
- Float division by zero stays an error (existing decision), but add
  `a.divided_by(b)` returning `Float or nothing` for code that expects it.

Done when: `examples/27_numbers.vaab` computes a rolling mean and z-score and the
timing of a `sleep(100)` loop, with snapshots.

---

## M2. Reading JSON

**Why:** every Helius RPC reply and webhook body is JSON. Vaab can write JSON but
has no user-facing decoder. The decoder already exists internally:
`crates/vaab-vm/src/json.rs` (`json_to_value`) and the route-body path in
`crates/vaab-server/src/run.rs` used by `expecting T`.

Surface — two forms, because RPC payloads are a mix of known and unknown shapes:

```vaab
# typed: decode straight into a declared type
let reply = try from_json(text, as: RpcReply)

# dynamic: walk unknown structure
let doc = try Json.parse(text)
let logs = doc.at("params").at("result").at("value").at("logs").as_list()
```

- `from_json(text, as: T)` returns `T or fails JsonError`. Reuse the layout lookup
  that `expecting T` already does (`body_layout` in `compile/stmt.rs`). Missing
  optional fields decode to `nothing` when the field is a `maybe`.
- `Json` is a built-in `choice`: `Null | Bool | Number | Text | List | Object`, with
  `.at(key)`, `.at(index)`, `.as_text()`, `.as_int()`, `.as_float()`,
  `.as_list()`, each returning `maybe`.
- `JsonError` joins the prelude error choices (see D80 "Prelude error choices").

Done when: the D-record "deserialization is deferred" is superseded, and an example
decodes a captured `logsNotification` payload in both styles.

---

## M3. Bytes, base64 and base58

**Why:** Pump.fun events arrive as base64 `Program data:` lines and must be unpacked
as little-endian integers, and Solana addresses are base58.

Surface:

```vaab
let raw = try Bytes.from_base64(line)
raw.length
raw.slice(from: 8, to: 40)
raw.u8(at: 56)  raw.u32_le(at: 8)  raw.u64_le(at: 40)  raw.i64_le(at: 89)
raw.bool(at: 56)
raw.to_hex()  raw.to_base58()  raw.to_base64()
raw.starts_with(Bytes.from_hex("bddb7fd34ee661ee"))
raw.borsh_text(at: 8)     # (Text, Int): the string and the offset after it
```

Work:
- New `Type::Bytes` and `Value::Bytes(Ref<[u8]>)`; sendable, comparable, printable
  as `bytes(hex…)`.
- Out-of-range reads return `failure BytesError.OutOfRange(at, length)` rather than
  stopping the program, so a malformed event is one skipped event.
- `u64` values above `i64::MAX` fail with `BytesError.TooLarge` instead of wrapping.
  (Lamport amounts fit comfortably in `i64`.)
- Dependencies: `base64` and `bs58` crates in `[workspace.dependencies]`.

Done when: an example decodes a real Pump.fun TradeEvent into a typed record, with
the same layout as `pumpfun-quant-bot/src/paper_trader/stream_parser.py`:

| Field | Bytes |
|-------|-------|
| discriminator `bddb7fd34ee661ee` | 0..8 |
| mint (base58) | 8..40 |
| sol_amount u64, token_amount u64, is_buy bool | 40..57 |
| user (base58) | 57..89 |
| timestamp i64, virtual SOL, virtual token, real SOL, real token reserves (u64) | 89..129 |

The CreateEvent (`1b720dc8084266ae`) exercises `borsh_text` for name, symbol and uri.

---

## M4. Long-lived servers: background tasks, shared state, non-blocking I/O

**Why:** today `vaab serve` only runs route bodies, each as its own machine run
(phase 6 notes D79). A streaming backend needs a loop that runs for the life of the
process, plus state that routes can read. And `http.get` currently blocks the thread
running the machine, which would stall every other task on that worker.

Surface:

```vaab
serve on port 8765 {
    on start {
        start { ingest(book) }
        start { every(milliseconds: 5000, flush_metrics) }
    }

    route GET "/api/state" {
        reply with book.value
    }
}

let book = Shared.new(Book.empty())   # process-wide, visible to routes and tasks
```

Work:
- **`on start` block** inside `serve`: compiled like a route body, run once after
  the listener binds, on the same scheduler that runs requests. Tasks it `start`s
  outlive it.
- **Process-wide `Shared` globals**: a top-level `let x = Shared.new(...)` is
  created once per `World` and given to every request machine. Today each request
  gets fresh state, so find where the server builds its `World` (`ServerState` in
  `crates/vaab-server/src/run.rs`) and hoist globals into it. Only `Shared`,
  `Channel`, `Db`, `Store` and `Logger` values may be globals; anything else is a
  checker error pointing at `Shared.new`.
- **Non-blocking I/O**: add `WaitSite::Io { ticket }`. A blocking builtin
  (`http.*`, `db.*` on long queries, `File.read`) hands its work to a small I/O pool
  (or `tokio::task::spawn_blocking` under `vaab serve`), parks, and is woken with
  the result. Do `http.*` first; it is the one the streaming loop needs.
- **`every(milliseconds:, f)`** helper built on `sleep`.
- **Graceful shutdown**: Ctrl-C cancels background tasks (`TaskMeta.cancelled`) and
  closes sockets before exit.

Done when: an example server keeps a counter that a background task increments
every 100 ms, `GET /count` returns it, and a slow `http.get` in one task does not
delay `GET /count`.

---

## M5. WebSocket client

**Why:** this is the main thing the bot does: `logsSubscribe` against Helius and read
notifications forever.

Surface — incoming messages arrive on an ordinary channel, so `receive`, `select`
and timeouts all work without new syntax:

```vaab
let socket = try Socket.connect(url)
try socket.send(to_json(subscribe_request))

while yes {
    select {
        when receive from socket.messages as message { handle(message) }
        when timeout after 30 seconds {
            log.warn("no message for 30s, reconnecting")
            return
        }
    }
}
```

- `Socket.connect(url, headers: {...})` returns `Socket or fails SocketError`.
- `socket.messages`: `Channel of Text` (binary frames come through a second
  `socket.binary: Channel of Bytes`). The channel closes when the connection drops,
  and the reason is available as `socket.closed_because`.
- `socket.send(text)`, `socket.close()`.
- Pings and pongs are handled for you. Reconnection is left to the program (a
  `while yes` around `connect`), and the example shows the pattern with backoff.
- Implementation: `tokio-tungstenite` (+ `rustls`) on the I/O runtime from M4. A
  reader task pushes frames into a `Host` channel; the channel's capacity
  (default 1024) applies backpressure, and `Socket.connect(url, buffer: n)` sets it.
- No new `select` syntax: `when receive from socket.messages as m` and
  `when timeout after` already exist.

Done when: `examples/apps/solana-logs/main.vaab` subscribes to the Pump.fun program
(`6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`), prints decoded trades, and survives
a forced disconnect.

---

## M6. WebSocket server and broadcast

**Why:** the dashboard receives a push stream; polling adds latency and load.

Surface:

```vaab
let hub = Hub.new(of: Text)

serve on port 8765 {
    socket "/ws" as client {
        hub.join(client)
        while yes {
            let message = try receive from client.messages
            handle_command(message)
        }
    }
}

# anywhere, including background tasks:
hub.broadcast(to_json(snapshot))
```

- A `socket "/path" as name` route in the `serve` block; `name` has the same
  `.messages` / `.send` / `.close` shape as the M5 client, so one mental model covers
  both directions.
- Implemented with hyper's upgrade + `tokio-tungstenite` in
  `crates/vaab-server/src/run.rs`; the router in `router.rs` learns a socket route
  kind.
- `Hub`: a process-wide fan-out. Clients that disconnect are dropped. A slow client
  whose buffer fills is disconnected rather than allowed to slow the broadcaster.
- Static files already work (`reply file`), so the existing dashboard HTML can be
  served as is.

Done when: an example serves a page that shows a live counter pushed over `/ws`.

---

## M7. Throughput and correctness under load

**Why:** Pump.fun on mainnet produces bursts of hundreds of events per second, and
signal maths runs on every one.

- Benchmark harness: replay a captured log file (one JSON notification per line)
  through decode → signal → SQLite insert → broadcast. Target: 2,000 events/s
  sustained on an M-series laptop with p99 under 5 ms per event. Measure before
  optimising.
- Make `vaab serve` use the multi-worker scheduler in
  `crates/vaab-vm/src/parallel.rs`, and make `Shared.update` take a real lock
  (the existing note in DECISIONS says phase 7 needs this).
- SQLite: batch inserts inside a transaction (`db.transaction { ... }`) and turn on
  WAL mode by default for file databases.
- Profile the interpreter's hot path (`machine.rs` dispatch, `Ref` clones) with
  `cargo flamegraph` against the replay harness.

Done when: the replay benchmark is checked in under `crates/vaab-vm/benches/` and
meets the target.

---

## M8. Postgres (optional)

Only needed if the bot moves off SQLite. `sea-query` already emits Postgres SQL for
`postgres:` URLs; add a live connection (`postgres` crate, sync, to match the
`rusqlite` layer) behind the same `Db` handle.

---

## M9. Proof: port the bot

Build `examples/apps/pumpfun-live/` as the acceptance test for everything above:

1. `ingest.vaab` — Helius `logsSubscribe` (M5), extract `Program data:` lines
   (M2), decode Trade/Create events (M3).
2. `signals.vaab` — per-mint rolling buy velocity and z-score, and a Shewhart-style
   dump alarm (M1), held in a `Shared` book (M4).
3. `store.vaab` — append events to SQLite (existing `Db` + M7 transactions).
4. `main.vaab` — `serve` block: `on start` launches ingestion, `GET /api/state`,
   `socket "/ws"` pushes updates via a `Hub` (M6), `reply file` serves the
   dashboard.
5. Config through `env.required("HELIUS_API_KEY")`.

Compare against the Python bot by running both on the same captured replay file:
event counts, per-mint velocity and alarm timestamps should match exactly.
Paper-trading logic (`virtual_wallet.py`, `trader.py`) is ported last, once the
signal parity check passes.

---

## Order and rough size

| Milestone | Depends on | Size |
|-----------|-----------|------|
| M1 Time and maths | — | small |
| M2 Reading JSON | — | small (decoder exists) |
| M3 Bytes | — | medium (new value type) |
| M4 Long-lived servers | M1 (`sleep`) | large (scheduler + server) |
| M5 WebSocket client | M4 | medium |
| M6 WebSocket server | M4, M5 | medium |
| M7 Throughput | M4–M6 | medium |
| M8 Postgres | — | small, optional |
| M9 Port the bot | M1–M7 | medium |

M1, M2 and M3 are independent and can be done in parallel. M4 is the riskiest
because it changes how the scheduler parks tasks; build its non-blocking I/O
mechanism with `http.get` before any socket work.

## Rules for whoever implements this

- One milestone per branch and PR. Don't start M5 until M4 is merged.
- Every user-facing error has a snapshot and tells the person what to write instead.
- No builtin may block an OS thread. If it waits, it parks.
- Record each non-obvious choice as a numbered decision (D87 onwards) in the same PR.
- Update the example snapshot
  (`crates/vaab-vm/tests/snapshots/examples__the_examples_print_what_they_always_printed.snap`)
  deliberately, via `cargo insta review`, never by hand.

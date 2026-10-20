# Changelog

## 0.1.7 — 2026-10-21

SIMD-accelerated JSON encoding and decoding via `sonic-rs`, with full nested JSON support across `to_json`, `from_json`, `Json.parse`, and `expecting` route bodies.

- `to_json` streams directly through `sonic-rs` without building an intermediate JSON tree, and serializes `Json` choice documents as native JSON values.
- `from_json(text, as: T)` and HTTP `expecting T` route bodies decode nested records, lists (`[T]`), maps (`{Text: T}`), tuples, choices, casts, `Bytes`, and recursive types.
- `Json` documents add `.as_bool()` and `.as_map()` alongside `.at()`, `.as_text()`, `.as_int()`, `.as_float()`, and `.as_list()`.

## 0.1.6 — 2026-10-20

Servers can share state across requests, start background work, and speak WebSocket, alongside mounts, Postgres, and file includes.

- `Clock.millis`, `Clock.monotonic_millis`, and `sleep` park a task without blocking the thread. Float and list maths cover `sqrt`, `ln`, `exp`, `pow`, `floor`, `ceil`, `sum`, `min`, `max`, `mean`, and `sorted`.
- `from_json` and `Json.parse` read JSON. `Bytes` unpacks binary payloads.
- `on start` runs once after the listener binds. Process-wide handles stay shared across requests, and HTTP calls park instead of stalling the server.
- `Socket.connect` is a WebSocket client. `socket` routes and `Hub` broadcast to every connected client.
- The live server runs on the worker pool, so `Shared` updates and SQLite transactions stay correct under load.

## 0.1.5 — 2026-10-20

- `need` can include a `.vaab` file. Names land in the same scope, includes resolve relative to the current file, and cycles are an error.
- `Db.connect` opens live Postgres connections. Queries bind native parameters, including `LIMIT` and `OFFSET`, and the client runs on its own thread.
- `serve` can mount named route groups, decode map bodies, and read non-text Postgres cells.

## 0.1.4 — 2026-10-13

- Linked riffs get fresh `NodeId`s, so multi-query database helpers, bare imports, and server routes compile without collisions.
- `riff install` records the install against riff.vaab.dev.

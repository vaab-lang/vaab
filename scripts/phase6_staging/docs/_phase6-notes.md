# Phase 6 notes

Decisions for the web server. Append to `DECISIONS.md` when no other agent is editing it.

## D75. Phase 6 words are soft keywords

Per D4, `serve`, `route`, `port`, `request`, `status`, `expecting`, `before`, `every`,
`anything`, `with`, `reply` and `explain` are recognised contextually as soft keywords.
They remain usable as variable names elsewhere.

## D76. `request` is a triple of text

Inside a route or `before every request` hook, `request` is `(method, path, body)` as
`request.method`, `request.path` and `request.body`, all `Text`.

## D77. `reply with` encodes JSON

A successful reply serialises the value with `serde_json` when the type can JSON.
`reply explain` returns HTTP 400 with the error encoded as JSON.

## D78. One `serve` block per file at the top level

`serve on port ...` must live at file scope, like `type` and `choice`. Routes compile
to separate bodies registered on `Program.routes`.

## D79. `vaab serve` runs the hyper/tokio server

The `vaab-server` crate listens on the declared port and dispatches each HTTP request
to the matching compiled route body as its own machine run.

# Riffs

Vaab packages are called **riffs**. The tool stays `vaab`; only the package noun changed from "module" or "vibe" to something that fits the language.

## Layout

```
my-app/
  riff           # manifest
  needed.lock    # written by vaab gather
  main.vaab      # entry
```

A riff library lives in its own directory:

```
supabase/
  riff
  lib.vaab
```

## Manifest (`riff`)

Plain English, one statement per line:

```
riff blog 0.1.0
description A small blog API
by ada
licence MIT
needs vaab at 0.1
need json from ada at 1
need colours from ../vaab-riffs/colours
```

## Source (`need` in Vaab)

At the top of `main.vaab` (or `lib.vaab`):

```vaab
need json from ada
need supabase from ../vaab-riffs/supabase
need colours from ./vendor/colours of decode, Error
need json as js
```

- **Qualified** (default): `supabase.rest_get(...)` — merged as `supabase__rest_get`
- **`of`**: import only named exports without a prefix
- **`as`**: qualified access under an alias

## Commands

| Command | Purpose |
|---------|---------|
| `vaab new orchard` | Creates `riff` + `main.vaab` |
| `vaab need X from Y` | Adds a dependency line to `riff` and runs gather |
| `vaab gather` | Resolves dependencies → `needed.lock` |
| `vaab run main.vaab` | Links riffs, checks, runs |

## Lockfile (`needed.lock`)

Written by gather. Path deps pin absolute paths; registry deps will pin version + digest when `riffs.vaab.dev` exists.

## What stays builtin

Core I/O remains in the language: `Db`, `Store`, `http`, `env`, `serve`. Vendor SDKs (Supabase, Stripe, …) belong in riffs — see [vaab-riffs](https://github.com/vaab-lang/vaab-riffs).

## Status (v0.1)

- Path dependencies work
- Registry fetch is stubbed (`vaab gather` explains what to do)
- Linker merges `lib.vaab` with prefixed names for qualified imports

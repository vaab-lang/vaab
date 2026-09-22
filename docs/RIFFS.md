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
need supabase
need json from ada at 1
need colours from ../vendor/colours
```

## Install (like `gem install`)

```bash
riff install supabase
```

Riffs land in `~/.vaab/riffs/supabase/` (official catalog: [vaab-riffs](https://github.com/vaab-lang/vaab-riffs)).

## Source (`need` in Vaab)

At the top of `main.vaab` (or `lib.vaab`):

```vaab
need supabase
need json from ada
need colours from ./vendor/colours of decode, Error
need json as js
```

Bare `need supabase` resolves an installed riff. Path and registry forms still work when you need them.

- **Qualified** (default): `supabase.rest_get(...)` — merged as `supabase__rest_get`
- **`of`**: import only named exports without a prefix
- **`as`**: qualified access under an alias

## Commands

| Command | Purpose |
|---------|---------|
| `riff install supabase` | Install a riff into `~/.vaab/riffs` |
| `riff list` | List installed riffs |
| `vaab new orchard` | Creates `riff` + `main.vaab` |
| `vaab need supabase` | Add installed riff to project + gather |
| `vaab need X from Y` | Add path/registry dependency + gather |
| `vaab gather` | Resolve dependencies → `needed.lock` |
| `vaab run main.vaab` | Link riffs, check, run |

## Lockfile (`needed.lock`)

Written by gather. Path deps pin absolute paths; registry deps will pin version + digest when `riffs.vaab.dev` exists.

## What stays builtin

Core I/O remains in the language: `Db`, `Store`, `http`, `env`, `serve`. Vendor SDKs (Supabase, Stripe, …) belong in riffs — see [vaab-riffs](https://github.com/vaab-lang/vaab-riffs).

## Status (v0.1)

- `riff install` fetches from the official GitHub catalog
- Bare `need supabase` resolves installed riffs
- Path dependencies work for local development
- Registry fetch (`need json from ada`) is stubbed for now
- Linker merges `lib.vaab` with prefixed names for qualified imports

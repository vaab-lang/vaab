#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

git checkout HEAD -- crates/vaab-syntax crates/vaab-types crates/vaab-vm crates/vaab-cli Cargo.lock README.md docs/LANGUAGE.md docs/DECISIONS.md
rm -rf crates/vaab-server
rm -f crates/vaab-syntax/src/parser/serve.rs crates/vaab-types/src/checker/serve.rs examples/14_web_server.vaab docs/_phase6-notes.md docs/_phase7-notes.md

cat > Cargo.toml <<'EOF'
[workspace]
resolver = "2"
members = ["crates/vaab-syntax", "crates/vaab-types", "crates/vaab-vm", "crates/vaab-cli"]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
license = "MIT OR Apache-2.0"
repository = "https://github.com/vaab-lang/vaab"

[workspace.dependencies]
ariadne = "0.5"
indexmap = "2"
logos = "0.15"
serde_json = "1"
thiserror = "2"
insta = "1"

vaab-syntax = { path = "crates/vaab-syntax" }
vaab-types = { path = "crates/vaab-types" }
vaab-vm = { path = "crates/vaab-vm" }

[profile.release]
lto = "thin"
EOF

python3 scripts/apply_phase5b_types.py
python3 scripts/apply_phase5b_vm.py

python3 <<'PY'
from pathlib import Path
root = Path('.')

# Fix file_error_not_found Rust syntax
p = root / 'crates/vaab-vm/src/compile/mod.rs'
t = p.read_text()
bad = '''    pub(crate) fn file_error_not_found(&self) -> u32 {
        for (choice, declared) in self.checked.choices.iter().enumerate() {
            if declared.name != "FileError":
                continue
            for (variant, shape) in declared.variants.iter().enumerate():
                if shape.name == "NotFound":
                    return self.variant_of.get(&(choice, variant)).copied().unwrap_or(0)
        }
        0
    }'''
good = '''    pub(crate) fn file_error_not_found(&self) -> u32 {
        for (choice, declared) in self.checked.choices.iter().enumerate() {
            if declared.name != "FileError" {
                continue;
            }
            for (variant, shape) in declared.variants.iter().enumerate() {
                if shape.name == "NotFound" {
                    return self.variant_of.get(&(choice, variant)).copied().unwrap_or(0);
                }
            }
        }
        0
    }'''
if bad in t:
    p.write_text(t.replace(bad, good))

# Shape ability test
p = root / 'crates/vaab-types/tests/check.rs'
t = p.read_text()
old = '    let ability = checked.abilities.first().expect("one ability");'
new = '    let ability = checked.abilities.iter().find(|a| a.name == "Shape").expect("Shape ability");'
if old in t:
    p.write_text(t.replace(old, new))

# not-json error tests
p = root / 'crates/vaab-types/tests/errors.rs'
t = p.read_text()
insert = '''
#[test]
fn a_type_that_cannot_json_is_rejected() {
    let rendered = errors("type Box can Json { inbox: channel of Int }\\n");
    assert!(rendered.contains("cannot be turned into JSON"), "{rendered}");
    assert_snapshot!(rendered);
}

#[test]
fn to_json_requires_a_json_type() {
    let rendered = errors("to echo(n: Int) returns Int = n\\nprint(to_json(echo))\\n");
    assert!(rendered.contains("cannot be turned into JSON"), "{rendered}");
    assert_snapshot!(rendered);
}
'''
marker = '// ---------------------------------------------------------------------------\n// One mistake makes one message'
if 'a_type_that_cannot_json_is_rejected' not in t:
    p.write_text(t.replace(marker, insert + marker))
PY

cat > examples/14_json.vaab <<'EOF'
# File I/O, time, and JSON.
#
# Programs declare `FileError`; `read_file` fails with it when a path is missing.
# Types that `can Json` may be passed to `to_json`, which gives back Text.
# `now()` is whole seconds since 1970-01-01 UTC.

choice FileError {
    NotFound(path: Text)
}

type Person can Json {
    name: Text
    score: Int
}

let person = Person.new(name: "Ada", score: 36)
print(to_json(person))

# The exact second changes; only show that the clock is plausible.
print(now() > 1_000_000_000)

match read_file("this-file-is-not-here.txt") {
    when success text then print("unexpected: {text}")
    when failure _    then print("missing file is expected")
}
EOF

python3 <<'PY'
from pathlib import Path

lang = Path('docs/LANGUAGE.md').read_text()
old = '''### Arriving later

`read_file` has a signature so that a program can be written against it, and
stops with a report naming phase 5 if it is reached.

---'''
new = '''### File I/O

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

---'''
if old in lang:
    Path('docs/LANGUAGE.md').write_text(lang.replace(old, new))

readme = Path('README.md').read_text()
readme = readme.replace('| 5b | Standard library — file I/O, time, json | next |',
                        '| 5b | Standard library — file I/O, time, json | done |')
Path('README.md').write_text(readme)

decisions = Path('docs/DECISIONS.md').read_text()
if '## Phase 5b' not in decisions:
    block = '''
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

'''
    decisions = decisions.replace('---\n\n## Still open', block + '---\n\n## Still open')
    Path('docs/DECISIONS.md').write_text(decisions)
PY

cargo update -w -q
INSTA_UPDATE=always cargo test -p vaab-syntax -p vaab-types -p vaab-vm -p vaab-cli --lib --tests

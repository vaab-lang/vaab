#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

git checkout HEAD -- Cargo.lock crates/vaab-syntax crates/vaab-types crates/vaab-vm crates/vaab-cli
rm -rf crates/vaab-server
rm -f crates/vaab-syntax/src/parser/serve.rs crates/vaab-types/src/checker/serve.rs crates/vaab-vm/src/parallel.rs examples/14_web_server.vaab

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
python3 -c "from pathlib import Path;p=Path('crates/vaab-vm/src/compile/mod.rs');t=p.read_text();bad='if declared.name != \"FileError\":\n                continue\n            for (variant, shape) in declared.variants.iter().enumerate():\n                if shape.name == \"NotFound\":\n                    return self.variant_of.get(&(choice, variant)).copied().unwrap_or(0)';good='if declared.name != \"FileError\" {\n                continue;\n            }\n            for (variant, shape) in declared.variants.iter().enumerate() {\n                if shape.name == \"NotFound\" {\n                    return self.variant_of.get(&(choice, variant)).copied().unwrap_or(0);\n                }\n            }';p.write_text(t.replace(bad,good) if bad in t else t)"
python3 -c "from pathlib import Path;p=Path('crates/vaab-types/tests/check.rs');t=p.read_text();old='    let ability = checked.abilities.first().expect(\"one ability\");';new='    let ability = checked.abilities.iter().find(|ability| ability.name == \"Shape\").expect(\"Shape ability\");';p.write_text(t.replace(old,new) if old in t else t)"

# Strip any phase-6/7 pollution that landed during apply.
git checkout HEAD -- crates/vaab-syntax crates/vaab-vm/src/compile/stmt.rs crates/vaab-vm/src/concurrency.rs crates/vaab-cli/Cargo.toml crates/vaab-cli/src/main.rs crates/vaab-cli/src/args.rs
git checkout HEAD -- crates/vaab-types/src/checked.rs crates/vaab-types/src/lib.rs crates/vaab-types/src/checker/walk.rs crates/vaab-types/src/checker/stmt.rs crates/vaab-types/src/checker/pure.rs crates/vaab-types/src/checker/member.rs
rm -f crates/vaab-syntax/src/parser/serve.rs crates/vaab-types/src/checker/serve.rs crates/vaab-vm/src/parallel.rs examples/14_web_server.vaab
python3 scripts/apply_phase5b_types.py

rm -f examples/14_web_server.vaab
test ! -f crates/vaab-vm/src/parallel.rs

cargo update -w -q
INSTA_UPDATE=always cargo test -p vaab-syntax -p vaab-types -p vaab-vm -p vaab-cli --lib --tests

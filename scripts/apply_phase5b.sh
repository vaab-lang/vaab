#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

git checkout HEAD -- crates/vaab-types crates/vaab-syntax crates/vaab-vm
rm -f crates/vaab-types/src/checker/serve.rs crates/vaab-syntax/src/parser/serve.rs

python3 scripts/apply_phase5b_types.py
python3 scripts/apply_phase5b_vm.py

INSTA_UPDATE=always cargo test -p vaab-syntax -p vaab-types -p vaab-vm -p vaab-cli --lib --tests

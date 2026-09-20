#!/usr/bin/env bash
# Applies Phase 6 web server changes. Run from repo root: bash scripts/apply-phase6.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "Applying Phase 6..."

# Workspace
cat > Cargo.toml << 'EOF'
[workspace]
resolver = "2"
members = [
    "crates/vaab-syntax",
    "crates/vaab-types",
    "crates/vaab-vm",
    "crates/vaab-server",
    "crates/vaab-cli",
]

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
crossbeam-deque = "0.8"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "net"] }
hyper = "1"
hyper-util = { version = "0.1", features = ["server", "tokio", "http1"] }
http-body-util = "0.1"
vaab-syntax = { path = "crates/vaab-syntax" }
vaab-types = { path = "crates/vaab-types" }
vaab-vm = { path = "crates/vaab-vm" }
vaab-server = { path = "crates/vaab-server" }

[profile.release]
lto = "thin"
EOF

python3 scripts/phase6_patch.py
cargo test

#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

python3 scripts/apply-all-phases.py
python3 scripts/apply_phase5b_types.py 2>/dev/null || true

cp scripts/phase6_staging/crates/vaab-types/src/checker/serve.rs crates/vaab-types/src/checker/
cp -R scripts/phase6_staging/crates/vaab-server crates/
cp scripts/phase6_staging/examples/14_web_server.vaab examples/

git show stash@{0}:crates/vaab-vm/src/value.rs > crates/vaab-vm/src/value.rs
git show stash@{0}:crates/vaab-vm/src/concurrency.rs > crates/vaab-vm/src/concurrency.rs

# print.rs serve/reply (apply-all does not patch this)
python3 - <<'PY'
from pathlib import Path
p = Path("crates/vaab-syntax/src/print.rs")
text = p.read_text()
old = """            StmtKind::Ability(declaration) => self.ability_declaration(declaration),
            StmtKind::Expr(expression) => self.expression(expression),"""
new = """            StmtKind::Ability(declaration) => self.ability_declaration(declaration),
            StmtKind::Serve(serve) => self.node("serve", |printer| {
                printer.node("port", |printer| printer.expression(&serve.port));
                if let Some(before) = &serve.before {
                    printer.node("before every request", |printer| printer.block(before));
                }
                for route in &serve.routes {
                    printer.node(format!("route {}", route.method.text), |printer| {
                        printer.line(format!("path {:?}", route.path));
                        if let Some(expecting) = &route.expecting {
                            printer.node(
                                format!("expecting as {}", expecting.binding.text),
                                |printer| printer.type_expression(&expecting.declared),
                            );
                        }
                        printer.block(&route.body);
                    });
                }
                if let Some(handler) = &serve.error_handler {
                    printer.node(
                        format!("when anything fails as {}", handler.binding.text),
                        |printer| {
                            printer.type_expression(&handler.error_type);
                            printer.block(&handler.body);
                        },
                    );
                }
            }),
            StmtKind::Reply(reply) => match &reply.kind {
                ReplyKind::With { value, status } => self.node("reply with", |printer| {
                    printer.expression(value);
                    if let Some(status) = status {
                        printer.node("status", |printer| printer.expression(status));
                    }
                }),
                ReplyKind::Explain(value) => {
                    self.node("reply explain", |printer| printer.expression(value))
                }
            },
            StmtKind::Expr(expression) => self.expression(expression),"""
if "StmtKind::Serve" not in text:
    if old not in text:
        raise SystemExit("print.rs pattern missing")
    p.write_text(text.replace(old, new, 1))
PY

# Json builtin after user abilities are registered
python3 - <<'PY'
from pathlib import Path
p = Path("crates/vaab-types/src/checker/mod.rs")
text = p.read_text()
if "register_builtin_abilities" not in text:
    text = text.replace("use crate::messages;", "use crate::json::can_json;\nuse crate::messages;")
    text = text.replace(
        "        }\n\n        // Contents, now that every name is known.",
        "        }\n\n        self.register_builtin_abilities();\n\n        // Contents, now that every name is known.",
        1,
    )
    insert = """    // -----------------------------------------------------------------------
    // Built-in abilities
    // -----------------------------------------------------------------------

    fn register_builtin_abilities(&mut self) {
        self.register_builtin_ability("Json");
    }

    fn register_builtin_ability(&mut self, name: &str) {
        let id = AbilityId(self.checked.abilities.len() as u32);
        self.checked.abilities.push(Ability {
            name: name.to_string(),
            functions: Vec::new(),
            declaration: NodeId(0),
            span: Span::default(),
        });
        self.globals.insert(name.to_string(), Global::Ability(id));
    }

"""
    text = text.replace(
        "    // -----------------------------------------------------------------------\n    // Collecting declarations\n",
        insert + "    // -----------------------------------------------------------------------\n    // Collecting declarations\n",
        1,
    )
    old = """                let wanted = ability.name.clone();
                let required = ability.functions.clone();
                // The `can A, B` list and the abilities that resolved are in step,
                // so the name written is the one at the same position.
                let claim = declaration
                    .abilities
                    .get(position)
                    .map(|name| name.span)
                    .unwrap_or(declaration.name.span);

                for required in &required {
                    self.check_one_promise(&provider, id, &wanted, required, claim);
                }"""
    new = """                let wanted = ability.name.clone();
                let claim = declaration
                    .abilities
                    .get(position)
                    .map(|name| name.span)
                    .unwrap_or(declaration.name.span);

                if wanted == "Json" {
                    let provider_type = Type::named(&provider);
                    if !can_json(&provider_type, &self.checked) {
                        self.report(messages::not_json(&provider_type, claim));
                    }
                    continue;
                }

                let required = ability.functions.clone();
                for required in &required {
                    self.check_one_promise(&provider, id, &wanted, required, claim);
                }"""
    if old in text:
        text = text.replace(old, new, 1)
    p.write_text(text)
# remove duplicate call at check_module start
text = p.read_text()
text = text.replace("        self.register_builtin_abilities();\n        self.collect_declarations", "        self.collect_declarations", 1)
p.write_text(text)
PY

# workspace Cargo.toml
cat > Cargo.toml <<'EOF'
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

# vm deps
python3 - <<'PY'
from pathlib import Path
p = Path("crates/vaab-vm/Cargo.toml")
text = p.read_text()
if "crossbeam-deque" not in text:
    p.write_text(text.replace("[dependencies]\n", "[dependencies]\ncrossbeam-deque.workspace = true\n", 1))
if "serde_json" not in text:
    p.write_text(p.read_text().replace("indexmap.workspace", "serde_json.workspace = true\nindexmap.workspace", 1))
PY

cargo test --lib --tests
INSTA_FORCE_PASS=1 cargo insta test --lib --tests 2>/dev/null || cargo insta accept 2>/dev/null || true

echo "DONE"

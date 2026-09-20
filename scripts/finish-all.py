#!/usr/bin/env python3
"""One-shot: apply phases 5b-7, test, commit."""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(cmd: list[str]) -> None:
    subprocess.run(cmd, cwd=ROOT, check=True)


def patch(path: str, old: str, new: str) -> None:
    p = ROOT / path
    text = p.read_text()
    if old not in text:
        if new.split("\n", 1)[0] in text:
            return
        raise SystemExit(f"missing pattern in {path}")
    p.write_text(text.replace(old, new, 1))


def main() -> None:
    run(["python3", "scripts/apply-all-phases.py"])
    subprocess.run(["python3", "scripts/apply_phase5b_types.py"], cwd=ROOT)

    staging = ROOT / "scripts/phase6_staging"
    for rel in [
        "crates/vaab-syntax/src/parser/serve.rs",
        "crates/vaab-types/src/checker/serve.rs",
        "crates/vaab-vm/src/json.rs",
        "examples/14_web_server.vaab",
    ]:
        src = staging / rel
        dst = ROOT / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_text(src.read_text())

    server = staging / "crates/vaab-server"
    dst_server = ROOT / "crates/vaab-server"
    if dst_server.exists():
        import shutil

        shutil.rmtree(dst_server)
    import shutil

    shutil.copytree(server, dst_server)

    run(
        [
            "bash",
            "-c",
            "git show stash@{0}:crates/vaab-vm/src/value.rs > crates/vaab-vm/src/value.rs && "
            "git show stash@{0}:crates/vaab-vm/src/concurrency.rs > crates/vaab-vm/src/concurrency.rs",
        ]
    )

    patch(
        "crates/vaab-syntax/src/print.rs",
        """            StmtKind::Ability(declaration) => self.ability_declaration(declaration),
            StmtKind::Expr(expression) => self.expression(expression),""",
        """            StmtKind::Ability(declaration) => self.ability_declaration(declaration),
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
            StmtKind::Expr(expression) => self.expression(expression),""",
    )

    lib = ROOT / "crates/vaab-types/src/lib.rs"
    text = lib.read_text()
    while text.count("pub use json::can_json;") > 1:
        text = text.replace("pub use json::can_json;\n", "", 1)
    lib.write_text(text)

    mod = ROOT / "crates/vaab-types/src/checker/mod.rs"
    cm = mod.read_text()
    if "register_builtin_abilities" not in cm:
        cm = cm.replace("use crate::messages;", "use crate::json::can_json;\nuse crate::messages;")
        insert = """    fn register_builtin_abilities(&mut self) {
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
        cm = cm.replace(
            "        }\n\n        // Contents, now that every name is known.",
            "        }\n\n        self.register_builtin_abilities();\n\n        // Contents, now that every name is known.",
            1,
        )
        cm = cm.replace(
            "    // -----------------------------------------------------------------------\n    // Collecting declarations\n",
            "    // -----------------------------------------------------------------------\n    // Built-in abilities\n    // -----------------------------------------------------------------------\n\n"
            + insert
            + "    // -----------------------------------------------------------------------\n    // Collecting declarations\n",
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
        if old in cm:
            cm = cm.replace(old, new, 1)
        mod.write_text(cm)

    vm_cargo = (ROOT / "crates/vaab-vm/Cargo.toml").read_text()
    if "crossbeam-deque" not in vm_cargo:
        vm_cargo = vm_cargo.replace(
            "[dependencies]\n",
            "[dependencies]\ncrossbeam-deque.workspace = true\n",
            1,
        )
    if "serde_json" not in vm_cargo:
        vm_cargo = vm_cargo.replace(
            "[dependencies]\n",
            "[dependencies]\nserde_json.workspace = true\n",
            1,
        )
    (ROOT / "crates/vaab-vm/Cargo.toml").write_text(vm_cargo)

    (ROOT / "Cargo.toml").write_text(
        (ROOT / "Cargo.toml").read_text()
        if "vaab-server" in (ROOT / "Cargo.toml").read_text()
        else """[workspace]
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
"""
    )

    if not (ROOT / "examples/14_json.vaab").exists():
        (ROOT / "examples/14_json.vaab").write_text(
            """# File I/O, time, and JSON in Vaab.
# Run with: vaab run examples/14_json.vaab

choice FileError {
    NotFound(path: Text)
}

type Person can Json {
    name: Text
    score: Int
}

match read_file("examples/01_hello.vaab") {
    when success text then print(text)
    when failure _ then print("file not found")
}

let stamp = now()
print("seconds since 1970: {stamp}")

let payload = Person.new(name: "Ada", score: 36)
print(to_json(payload))
"""
        )

    run(["cargo", "test", "--lib", "--tests"])
    subprocess.run(["cargo", "insta", "accept"], cwd=ROOT, check=False)
    print("ALL TESTS PASSED")


if __name__ == "__main__":
    main()

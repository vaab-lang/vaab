#!/usr/bin/env python3
from pathlib import Path

root = Path(__file__).resolve().parents[1]

(root / "crates/vaab-types/src/json.rs").write_text(
    (root / "crates/vaab-types/src/json.rs").read_text()
    if (root / "crates/vaab-types/src/json.rs").exists()
    else ""
)

json_src = '''//! Which types may be turned into JSON.

use crate::checked::{Checked, ChoiceId, TypeId};
use crate::types::Type;

pub fn can_json(value: &Type, checked: &Checked) -> bool {
    match value {
        Type::Int | Type::Float | Type::Bool | Type::Text | Type::Nothing => true,
        Type::List(item) | Type::Maybe(item) => can_json(item, checked),
        Type::Map { key, value } => **key == Type::Text && can_json(value, checked),
        Type::Tuple(items) => items.iter().all(|item| can_json(item, checked)),
        Type::Variable(_) => false,
        Type::Named(name) => {
            if let Some((index, _)) = checked.declared_types.iter().enumerate().find(|(_, d)| d.name == *name) {
                record_can_json(TypeId(index as u32), checked)
            } else if let Some((index, _)) = checked.choices.iter().enumerate().find(|(_, c)| c.name == *name) {
                choice_can_json(ChoiceId(index as u32), checked)
            } else { false }
        }
        Type::Parameter(_) | Type::Unknown => true,
        Type::Function { .. } | Type::Fallible { .. } | Type::Task(_) | Type::Shared(_) | Type::Channel(_) | Type::Ability(_) => false,
    }
}

fn record_can_json(id: TypeId, checked: &Checked) -> bool {
    checked.declared_type(id).is_some_and(|d| d.fields.iter().all(|f| can_json(&f.declared, checked)))
}

fn choice_can_json(id: ChoiceId, checked: &Checked) -> bool {
    checked.choice(id).is_some_and(|c| c.variants.iter().all(|v| v.fields.iter().all(|f| can_json(&f.declared, checked))))
}
'''
(root / "crates/vaab-types/src/json.rs").write_text(json_src)

def patch(path, old, new):
    p = root / path
    text = p.read_text()
    if old not in text:
        if new.split("\n", 1)[0] in text:
            return
        raise SystemExit(f"pattern missing in {path}")
    p.write_text(text.replace(old, new, 1))

patch("crates/vaab-types/src/lib.rs", "mod checked;\nmod checker;\nmod messages;", "mod checked;\nmod checker;\nmod json;\nmod messages;")
patch("crates/vaab-types/src/lib.rs", "pub use checker::check;\npub use types::", "pub use checker::check;\npub use json::can_json;\npub use types::")
patch(
    "crates/vaab-types/src/prelude.rs",
    """        Function {
            // The error is left as a type parameter so that `try read_file(path)`
            // fits whatever error the surrounding function declares. Phase 5 gives
            // the standard library a real `FileError` and this becomes concrete.
            name: "read_file",
            signature: Signature::new(
                vec![Parameter::new("path", Type::Text)],
                Type::fallible(Type::Text, Type::parameter("E")),
            ),
        },
    ]""",
    """        Function {
            name: "read_file",
            signature: Signature::new(
                vec![Parameter::new("path", Type::Text)],
                Type::fallible(Type::Text, Type::named("FileError")),
            ),
        },
        Function {
            name: "now",
            signature: Signature::new(Vec::new(), Type::Int),
        },
        Function {
            name: "to_json",
            signature: Signature::new(
                vec![Parameter::new("value", Type::parameter("T"))],
                Type::Text,
            ),
        },
    ]""",
)
patch("crates/vaab-types/src/prelude.rs", 'assert_eq!(names, ["print", "read_file"]);', 'assert_eq!(names, ["print", "read_file", "now", "to_json"]);')
patch(
    "crates/vaab-types/src/messages.rs",
    """// ---------------------------------------------------------------------------
// Abilities
// ---------------------------------------------------------------------------

pub fn undefined_ability(name: &str, span: Span, known: &[String]) -> Diagnostic {""",
    """// ---------------------------------------------------------------------------
// Abilities
// ---------------------------------------------------------------------------

pub fn not_json(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("not-json", format!("{} cannot be turned into JSON", a(found)))
        .at(span, format!("this is {found}"))
        .with_help("only types that `can Json` may be passed to `to_json`")
        .with_note("a type `can Json` when every field, variant and item it holds can Json too")
}

pub fn undefined_ability(name: &str, span: Span, known: &[String]) -> Diagnostic {""",
)

cm = (root / "crates/vaab-types/src/checker/mod.rs").read_text()
if "register_builtin_abilities" not in cm:
    cm = cm.replace("use crate::messages;\nuse crate::prelude;", "use crate::json::can_json;\nuse crate::messages;\nuse crate::prelude;")
    cm = cm.replace("        self.collect_declarations(&module.statements);", "        self.register_builtin_abilities();\n        self.collect_declarations(&module.statements);")
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
    cm = cm.replace("    // -----------------------------------------------------------------------\n    // Collecting declarations\n", insert + "    // -----------------------------------------------------------------------\n    // Collecting declarations\n")
    cm = cm.replace(
        """                let wanted = ability.name.clone();
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
                }""",
        """                let wanted = ability.name.clone();
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
                }""",
    )
    (root / "crates/vaab-types/src/checker/mod.rs").write_text(cm)

cc = (root / "crates/vaab-types/src/checker/call.rs").read_text()
if "check_to_json_argument" not in cc:
    cc = cc.replace("use crate::checked::{ArgumentSource, Call, Resolution};\nuse crate::messages;", "use crate::checked::{ArgumentSource, Call, Resolution};\nuse crate::json::can_json;\nuse crate::messages;")
    cc = cc.replace("                        self.expression(&argument.value, Wanted::Exactly(declared));", "                        let found = self.expression(&argument.value, Wanted::Exactly(declared));\n                        self.check_to_json_argument(shape, argument, &found);")
    cc = cc.replace("            self.expression(&argument.value, Wanted::Exactly(declared));\n        }\n\n        self.fill_the_rest(", "            let found = self.expression(&argument.value, Wanted::Exactly(declared));\n            self.check_to_json_argument(shape, argument, &found);\n        }\n\n        self.fill_the_rest(")
    helper = """
    fn check_to_json_argument(&mut self, shape: &CallShape, argument: &Argument, found: &Type) {
        if shape.name != "to_json" { return; }
        let resolved = self.variables.resolve(found);
        if matches!(resolved, Type::Unknown | Type::Variable(_)) { return; }
        if !can_json(&resolved, &self.checked) {
            self.report(messages::not_json(&resolved, argument.value.span));
        }
    }
"""
    cc = cc.replace("\n}\n\n/// Turns a function's type into a signature", helper + "\n}\n\n/// Turns a function's type into a signature")
    (root / "crates/vaab-types/src/checker/call.rs").write_text(cc)

print("vaab-types phase 5b applied")

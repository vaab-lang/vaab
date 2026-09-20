#!/usr/bin/env python3
from pathlib import Path

root = Path(__file__).resolve().parents[1]

vm_json = '''//! Turning runtime values into JSON text.

use serde_json::{Map, Number, Value as Json};

use crate::error::Fault;
use crate::value::Value;

pub fn encode(value: &Value) -> Result<String, Fault> {
    let json = value_to_json(value)?;
    serde_json::to_string(&json).map_err(|_| Fault::Confused("could not turn the value into JSON text"))
}

fn value_to_json(value: &Value) -> Result<Json, Fault> {
    match value {
        Value::Nothing => Ok(Json::Null),
        Value::Int(number) => Ok(Json::Number(Number::from(*number))),
        Value::Float(number) => Number::from_f64(*number).map(Json::Number).ok_or(Fault::Confused("a decimal that cannot be written as JSON")),
        Value::Bool(yes) => Ok(Json::Bool(*yes)),
        Value::Text(text) => Ok(Json::String(text.to_string())),
        Value::List(items) => Ok(Json::Array(items.iter().map(value_to_json).collect::<Result<Vec<_>, _>>()?)),
        Value::Map(entries) => {
            let mut object = Map::new();
            for (key, value) in entries.iter() {
                let Value::Text(text) = &key.0 else { return Err(Fault::Confused("a map key was not text")); };
                object.insert(text.to_string(), value_to_json(value)?);
            }
            Ok(Json::Object(object))
        }
        Value::Tuple(parts) => Ok(Json::Array(parts.iter().map(value_to_json).collect::<Result<Vec<_>, _>>()?)),
        Value::Maybe(None) => Ok(Json::Null),
        Value::Maybe(Some(held)) => value_to_json(held),
        Value::Record(record) => {
            let mut object = Map::new();
            for (name, value) in record.layout.fields.iter().zip(&record.fields) {
                object.insert(name.clone(), value_to_json(value)?);
            }
            Ok(Json::Object(object))
        }
        Value::Variant(variant) => {
            let mut fields = Map::new();
            for (name, value) in variant.layout.fields.iter().zip(&variant.fields) {
                fields.insert(name.clone(), value_to_json(value)?);
            }
            let mut object = Map::new();
            object.insert(variant.layout.name.clone(), Json::Object(fields));
            Ok(Json::Object(object))
        }
        Value::Success(held) => value_to_json(held),
        _ => Err(Fault::Confused("this value cannot be turned into JSON")),
    }
}
'''
(root / "crates/vaab-vm/src/json.rs").write_text(vm_json)

def patch(path, old, new):
    p = root / path
    text = p.read_text()
    if old not in text:
        if new.split("\n", 1)[0] in text:
            return
        raise SystemExit(f"pattern missing in {path}")
    p.write_text(text.replace(old, new, 1))

patch("crates/vaab-vm/Cargo.toml", "indexmap.workspace = true\nvaab-syntax.workspace = true", "indexmap.workspace = true\nserde_json.workspace = true\nvaab-syntax.workspace = true")
patch("crates/vaab-vm/src/lib.rs", "mod compile;\npub mod concurrency;", "mod compile;\nmod json;\npub mod concurrency;")
patch("crates/vaab-vm/src/builtin.rs", "pub enum Builtin {\n    Print,\n\n    Upper,", "pub enum Builtin {\n    Print,\n    ToJson,\n\n    Upper,")
patch("crates/vaab-vm/src/builtin.rs", "            Builtin::Print => \"print\",\n            Builtin::Upper => \"upper\",", "            Builtin::Print => \"print\",\n            Builtin::ToJson => \"to_json\",\n            Builtin::Upper => \"upper\",")
patch("crates/vaab-vm/src/builtin.rs", "            Builtin::Print\n            | Builtin::Upper", "            Builtin::Print\n            | Builtin::ToJson\n            | Builtin::Upper")
patch("crates/vaab-vm/src/builtin.rs", "        match (self, arguments) {\n            (Builtin::Upper, [Value::Text(text)]) => Ok(Value::text(text.to_uppercase())),", "        match (self, arguments) {\n            (Builtin::ToJson, [value]) => crate::json::encode(value).map(Value::text),\n            (Builtin::Upper, [Value::Text(text)]) => Ok(Value::text(text.to_uppercase())),")

if "ReadFile" not in (root / "crates/vaab-vm/src/bytecode.rs").read_text():
    patch("crates/vaab-vm/src/bytecode.rs", "    Select(u32),\n\n    // -- Stopping ----------------------------------------------------------", "    Select(u32),\n\n    ReadFile(u32),\n    Now,\n\n    // -- Stopping ----------------------------------------------------------")
    patch("crates/vaab-vm/src/bytecode.rs", "        Op::Builtin(builtin) => format!(\"Builtin {}\", builtin.name()),\n        Op::NotYet(feature) => format!(\"NotYet ({feature})\"),", "        Op::Builtin(builtin) => format!(\"Builtin {}\", builtin.name()),\n        Op::ReadFile(layout) => format!(\"ReadFile {layout}\"),\n        Op::Now => \"Now\".to_string(),\n        Op::NotYet(feature) => format!(\"NotYet ({feature})\"),")

if "file_error_not_found" not in (root / "crates/vaab-vm/src/compile/mod.rs").read_text():
    patch("crates/vaab-vm/src/compile/mod.rs", "            _ => None,\n        }\n    }\n}\n\nimpl Builder {", """            _ => None,\n        }\n    }\n\n    pub(crate) fn file_error_not_found(&self) -> u32 {\n        for (choice, declared) in self.checked.choices.iter().enumerate() {\n            if declared.name != \"FileError\":\n                continue\n            for (variant, shape) in declared.variants.iter().enumerate():\n                if shape.name == \"NotFound\":\n                    return self.variant_of.get(&(choice, variant)).copied().unwrap_or(0)\n        }\n        0\n    }\n}\n\nimpl Builder {""")

ex = (root / "crates/vaab-vm/src/compile/expr.rs").read_text().replace("use crate::error::Feature;\n", "")
if "Op::ReadFile" not in ex:
    ex = ex.replace(
        '            Some(Resolution::Builtin("read_file")) => {\n                self.emit(Op::NotYet(Feature::ReadFile), span)\n            }\n            Some(Resolution::Builtin("print")) => {\n                self.push_constant(Value::Builtin(Builtin::Print), span)\n            }',
        '            Some(Resolution::Builtin("read_file")) => {\n                self.emit(Op::ReadFile(self.file_error_not_found()), span)\n            }\n            Some(Resolution::Builtin("now")) => self.emit(Op::Now, span),\n            Some(Resolution::Builtin("print")) => {\n                self.push_constant(Value::Builtin(Builtin::Print), span)\n            }\n            Some(Resolution::Builtin("to_json")) => {\n                self.push_constant(Value::Builtin(Builtin::ToJson), span)\n            }',
    )
    ex = ex.replace(
        '            Some(Resolution::Builtin("read_file")) => {\n                self.emit(Op::NotYet(Feature::ReadFile), span)\n            }\n\n            Some(Resolution::Function(id))',
        '            Some(Resolution::Builtin("read_file")) => {\n                let count = self.push_arguments(call, arguments, Defaults::None);\n                let _ = count;\n                self.emit(Op::ReadFile(self.file_error_not_found()), span);\n            }\n            Some(Resolution::Builtin("now")) => {\n                self.emit(Op::Now, span);\n            }\n            Some(Resolution::Builtin("to_json")) => {\n                let count = self.push_arguments(call, arguments, Defaults::None);\n                let _ = count;\n                self.emit(Op::Builtin(Builtin::ToJson), span);\n            }\n\n            Some(Resolution::Function(id))',
    )
(root / "crates/vaab-vm/src/compile/expr.rs").write_text(ex)

if "Op::ReadFile" not in (root / "crates/vaab-vm/src/machine.rs").read_text():
    patch(
        "crates/vaab-vm/src/machine.rs",
        "                return self.run_select(meta, descriptor, channels, pc, span, host);\n            }\n\n            // -- Stopping --------------------------------------------------",
        """                return self.run_select(meta, descriptor, channels, pc, span, host);\n            }\n            Op::ReadFile(layout) => {\n                let path = self.pop()?;\n                let text = match &path {\n                    Value::Text(text) => text.to_string(),\n                    _ => return Err(Fault::Confused(\"read_file expected a path\")),\n                };\n                match std::fs::read_to_string(&text) {\n                    Ok(contents) => self.stack.push(Value::success(Value::text(contents))),\n                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {\n                        let layout = program.variants.get(layout as usize).cloned().ok_or(\n                            Fault::Confused(\"read_file named a variant that is not there\"),\n                        )?;\n                        let variant = Value::Variant(Ref::new(Variant { layout, fields: vec![path] }));\n                        self.stack.push(Value::failure(variant));\n                    }\n                    Err(_) => return Err(Fault::Confused(\"could not read the file\")),\n                }\n            }\n            Op::Now => {\n                let seconds = std::time::SystemTime::now()\n                    .duration_since(std::time::UNIX_EPOCH)\n                    .map(|duration| duration.as_secs() as i64)\n                    .map_err(|_| Fault::Confused(\"the clock could not be read\"))?;\n                self.stack.push(Value::Int(seconds));\n            }\n\n            // -- Stopping --------------------------------------------------""",
    )

# tests
patch("crates/vaab-vm/tests/bytecode.rs", 'fn a_feature_a_later_phase_brings_compiles_to_one_instruction() {\n    assert_snapshot!(listing("print(read_file(\\"notes.txt\\"))\\n"));\n}', 'fn read_file_compiles_to_a_read_instruction() {\n    assert_snapshot!(listing(\n        "choice FileError { NotFound(path: Text) }\\nprint(read_file(\\"notes.txt\\"))\\n"\n    ));\n}')
patch(
    "crates/vaab-vm/tests/errors.rs",
    """fn reading_a_file_says_which_phase_brings_it() {
    let rendered = stops("print(read_file(\\"notes.txt\\"))\\n");
    assert!(rendered.contains("phase 5"), "{rendered}");
    assert_snapshot!(rendered);
}""",
    """fn reading_a_missing_file_fails_with_file_error() {
    let source = "choice FileError { NotFound(path: Text) }\\n\\
        match read_file(\\"missing.vaab\\") {\\n\\
        when success _ then print(\\"found\\")\\n\\
        when failure _ then print(\\"missing\\")\\n\\
        }\\n";
    assert_snapshot!(support::printed(source));
}""",
)

lib = (root / "crates/vaab-vm/tests/library.rs").read_text()
if "now_returns_seconds_since_1970" not in lib:
    lib += """
// ---------------------------------------------------------------------------
// File I/O, time and JSON
// ---------------------------------------------------------------------------

#[test]
fn now_returns_seconds_since_1970() {
    let seconds = printed("print(now())\\n").parse::<i64>().expect("a whole number");
    assert!(seconds > 1_600_000_000);
}

#[test]
fn to_json_turns_a_map_into_text() {
    assert_eq!(printed("print(to_json({\\"a\\": 1}))\\n"), "{\\"a\\":1}");
}

#[test]
fn read_file_returns_the_contents_of_a_file() {
    let path = std::env::temp_dir().join("vaab-read-file-test.txt");
    std::fs::write(&path, "hello from disk").expect("write temp file");
    let path = path.display();
    let source = format!(
        "choice FileError {{ NotFound(path: Text) }}\\n\\
         match read_file(\\"{path}\\") {{\\n\\
         when success text then print(text)\\n\\
         when failure _ then print(\\"missing\\")\\n\\
         }}\\n"
    );
    assert_eq!(printed(&source), "hello from disk");
}
"""
    (root / "crates/vaab-vm/tests/library.rs").write_text(lib)

print("vaab-vm phase 5b applied")

//! Browser playground endpoint: parse, check, and run Vaab source on demand.

use vaab_syntax::{diagnostic, parse, ColorChoice};
use vaab_types::check;
use vaab_vm::{error, machine::HttpResponse, Output, Value};

const MAX_SOURCE_BYTES: usize = 32 * 1024;

pub fn run(source: &str) -> HttpResponse {
    if source.trim().is_empty() {
        return HttpResponse::json(
            200,
            r#"{"status":"error","phase":"input","message":"source is empty"}"#.into(),
        );
    }

    if source.len() > MAX_SOURCE_BYTES {
        return HttpResponse::json(
            200,
            r#"{"status":"error","phase":"input","message":"source is too large"}"#.into(),
        );
    }

    let parsed = parse(source);
    if parsed.has_errors() {
        return HttpResponse::json(
            200,
            serde_json::json!({
                "status": "error",
                "phase": "parse",
                "message": diagnostic::render(
                    &parsed.diagnostics,
                    "playground.vaab",
                    source,
                    ColorChoice::Never,
                ),
            })
            .to_string(),
        );
    }

    let checked = match check(&parsed.module) {
        Ok(checked) => checked,
        Err(problems) => {
            return HttpResponse::json(
                200,
                serde_json::json!({
                    "status": "error",
                    "phase": "check",
                    "message": diagnostic::render(
                        &problems,
                        "playground.vaab",
                        source,
                        ColorChoice::Never,
                    ),
                })
                .to_string(),
            );
        }
    };

    let mut world = vaab_vm::prepare(&parsed.module, &checked, Output::collected());
    match vaab_vm::run(&mut world) {
        Ok(value) => HttpResponse::json(
            200,
            serde_json::json!({
                "status": "ok",
                "stdout": world.output.lines(),
                "result": show_value(&value),
            })
            .to_string(),
        ),
        Err(problem) => HttpResponse::json(
            200,
            serde_json::json!({
                "status": "error",
                "phase": "run",
                "message": error::render(&problem, "playground.vaab", source, ColorChoice::Never),
            })
            .to_string(),
        ),
    }
}

fn show_value(value: &Value) -> String {
    match value {
        Value::Nothing => "nothing".into(),
        other => other.show(),
    }
}

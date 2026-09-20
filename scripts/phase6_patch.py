#!/usr/bin/env python3
"""Apply Phase 6 web server changes. Run from repo root: python3 scripts/phase6_patch.py"""
from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STAGE = Path(__file__).resolve().parent / "phase6_staging"

RESET_PATHS = [
    "Cargo.toml",
    "crates/vaab-syntax",
    "crates/vaab-types",
    "crates/vaab-vm",
    "crates/vaab-cli",
]

RESET_TEST_PATHS = [
    "crates/vaab-vm/tests",
]


def reset_baseline() -> None:
    """Return patched crates to HEAD so the script is repeatable."""
    print("Resetting to HEAD baseline...")
    subprocess.run(
        ["git", "checkout", "HEAD", "--", *RESET_PATHS, *RESET_TEST_PATHS],
        cwd=ROOT,
        check=True,
    )
    parallel = ROOT / "crates/vaab-vm/src/parallel.rs"
    if parallel.exists():
        parallel.unlink()
        print("  removed crates/vaab-vm/src/parallel.rs")
    # Phase 5b WIP example breaks the examples/ type-check sweep.
    json_example = ROOT / "examples/14_json.vaab"
    if json_example.exists():
        json_example.unlink()
        print("  removed examples/14_json.vaab")


def write(rel: str, content: str) -> None:
    path = ROOT / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    print(f"  wrote {rel}")


def patch(rel: str, old: str, new: str, *, skip_if: str | None = None) -> None:
    path = ROOT / rel
    text = path.read_text()
    if skip_if and skip_if in text:
        print(f"  skip {rel} ({skip_if!r} already present)")
        return
    if old not in text:
        raise SystemExit(f"patch anchor missing in {rel}")
    path.write_text(text.replace(old, new, 1))
    print(f"  patched {rel}")


SERVE_MESSAGES = """
// ---------------------------------------------------------------------------
// Web server (phase 6)
// ---------------------------------------------------------------------------

pub fn serve_port_must_be_a_number(span: Span) -> Diagnostic {
    Diagnostic::error(
        "serve-port-must-be-a-number",
        "the port in `serve on port ...` must be a whole number",
    )
    .at(span, "this is not a plain whole number")
    .with_help("write something like `serve on port 8080 { ... }`")
}

pub fn unknown_http_method(method: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "unknown-http-method",
        format!("`{method}` is not an HTTP method Vaab knows"),
    )
    .at(span, format!("`{method}` was written here"))
    .with_help("use `get`, `post`, `put`, `patch` or `delete`")
}

pub fn route_param_type(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error(
        "route-param-type",
        format!("a route parameter may be `Int` or `Text`, not {found}"),
    )
    .at(span, format!("this parameter is declared as {found}"))
}

pub fn reply_outside_route(span: Span) -> Diagnostic {
    Diagnostic::error(
        "reply-outside-route",
        "`reply` may only appear inside a route body or error handler",
    )
    .at(span, "`reply` was written here")
}

pub fn cannot_json(span: Span) -> Diagnostic {
    Diagnostic::error(
        "cannot-json",
        "this value cannot be turned into JSON for the response body",
    )
    .at(span, "Vaab can only reply with JSON-serialisable values")
    .with_help("use text, numbers, lists, maps, records or choices whose fields can all be JSON")
}

"""


def cleanup() -> None:
    """Ensure serve message helpers exist and remove bad duplicates."""
    messages = ROOT / "crates/vaab-types/src/messages.rs"
    if not messages.exists():
        return
    text = messages.read_text()
    bad = (
        '        .help("write the port directly, such as `serve on port 8080`")\n'
    )
    if bad in text:
        text = text.replace(
            'pub fn serve_port_must_be_a_number(span: Span) -> Diagnostic {\n'
            '    Diagnostic::error("serve-port-must-be-a-number", "the port must be a whole number written as a literal")\n'
            '        .at(span, "this is not a literal port number")\n'
            '        .help("write the port directly, such as `serve on port 8080`")\n'
            "}\n\n"
            'pub fn route_param_type(found: &Type, span: Span) -> Diagnostic {\n'
            '    Diagnostic::error("route-param-type", format!("a path parameter may be Text or Int, not {}", show_type(found)))\n'
            '        .at(span, format!("this is {}", show_type(found)))\n'
            "}\n\n"
            'pub fn reply_outside_route(span: Span) -> Diagnostic {\n'
            '    Diagnostic::error("reply-outside-route", "`reply` only belongs inside a route body").at(span, "`reply` was written here")\n'
            "}\n\n"
            'pub fn unknown_http_method(method: &str, span: Span) -> Diagnostic {\n'
            '    Diagnostic::error("unknown-http-method", format!("`{method}` is not a supported HTTP method"))\n'
            '        .at(span, format!("`{method}` was written here"))\n'
            '        .help("use `get`, `post`, `put`, `patch` or `delete`")\n'
            "}\n\n"
            'pub fn cannot_json(span: Span) -> Diagnostic {\n'
            '    Diagnostic::error("cannot-json", "this value cannot be turned into JSON").at(span, "this value cannot be serialised")\n'
            "}\n\n",
            "",
        )
        print("  cleaned duplicate serve messages")
    if "pub fn serve_port_must_be_a_number" not in text:
        anchor = "\n#[cfg(test)]\nmod tests {"
        if anchor in text:
            text = text.replace(anchor, f"\n{SERVE_MESSAGES}{anchor}", 1)
            print("  inserted serve messages")
    messages.write_text(text)
    token_tests = ROOT / "crates/vaab-syntax/src/token.rs"
    if token_tests.exists():
        token_text = token_tests.read_text()
        fixed = 'for word in ["count", "request", "new", "raw"] {'
        if 'for word in ["count", "request", "route"' in token_text:
            token_text = token_text.replace(
                'for word in ["count", "request", "route", "status", "serve", "port", "new", "raw", "with"] {',
                fixed,
            )
            token_tests.write_text(token_text)
            print("  fixed token.rs keyword test expectations")


def main() -> None:
    print("Applying Phase 6...")
    stage = STAGE
    if not stage.exists():
        raise SystemExit(f"staging directory missing: {stage}")

    reset_baseline()

    # New / replacement files from staging
    for rel in [
        "crates/vaab-syntax/src/parser/serve.rs",
        "crates/vaab-types/src/checker/serve.rs",
        "crates/vaab-types/src/json.rs",
        "crates/vaab-vm/src/json.rs",
        "crates/vaab-server/Cargo.toml",
        "crates/vaab-server/src/lib.rs",
        "crates/vaab-server/src/router.rs",
        "crates/vaab-server/src/run.rs",
        "crates/vaab-server/tests/task_api.rs",
        "examples/14_web_server.vaab",
        "docs/_phase6-notes.md",
        "crates/vaab-syntax/tests/snapshots/parse__serve_block.snap",
        "crates/vaab-types/tests/snapshots/errors__reply_outside_route.snap",
        "crates/vaab-vm/tests/snapshots/examples__the_examples_print_what_they_always_printed.snap",
    ]:
        src = stage / rel
        if src.exists():
            write(rel, src.read_text())

    # Patches bundled in staging/patches/
    patches_dir = stage / "patches"
    if patches_dir.exists():
        for patch_file in sorted(patches_dir.glob("*.patch.py")):
            ns: dict = {"patch": patch, "write": write, "ROOT": ROOT}
            exec(patch_file.read_text(), ns)

    cleanup()
    # Phase 7 WIP must not remain mixed into Phase 6.
    parallel = ROOT / "crates/vaab-vm/src/parallel.rs"
    if parallel.exists():
        parallel.unlink()
        subprocess.run(
            ["git", "checkout", "HEAD", "--", "crates/vaab-vm/src/concurrency.rs"],
            cwd=ROOT,
            check=True,
        )
        print("  removed Phase 7 parallel.rs and reset concurrency.rs")
    print("Phase 6 applied.")


if __name__ == "__main__":
    main()

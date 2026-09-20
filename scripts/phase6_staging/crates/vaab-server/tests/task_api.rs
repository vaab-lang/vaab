#[test]
fn web_server_example_compiles_routes() {
    let source = r#"
serve on port 8080 {
    route get "/hello" {
        reply with "hello"
    }
}
"#;
    let parsed = vaab_syntax::parse(source);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    let checked = vaab_types::check(&parsed.module).expect("check");
    assert_eq!(checked.serves.len(), 1);
    let world = vaab_vm::prepare(&parsed.module, &checked, vaab_vm::Output::collected());
    assert_eq!(world.program.routes.len(), 1);
}

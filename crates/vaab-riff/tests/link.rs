use std::path::PathBuf;

use vaab_riff::{exports_of, gather, install_from_path, link};

#[test]
fn installed_riff_links_and_prefixes_exports() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../vaab-riffs/supabase");
    if !source.join("lib.vaab").is_file() {
        return;
    }

    install_from_path("supabase", &source).expect("install should succeed");

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/riff_demo");
    gather(&root).expect("gather should succeed");

    let entry = root.join("main.vaab");
    let linked = link(&entry).expect("link should succeed");
    assert!(linked.riffs.iter().any(|riff| riff.alias == "supabase"));

    let merged_names: Vec<String> = exports_of(&linked.module).into_iter().collect();
    assert!(merged_names.iter().any(|name| name == "supabase__fetch"));
    assert!(merged_names.iter().any(|name| name == "supabase__Client"));
}

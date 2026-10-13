use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use vaab_riff::{exports_of, gather, install_from_path, link};
use vaab_syntax::ast::{Module, NodeId};

fn collect_ids(module: &Module) -> Vec<NodeId> {
    let debug = format!("{module:?}");
    let mut ids = Vec::new();
    let mut rest = debug.as_str();
    while let Some(at) = rest.find("NodeId(") {
        rest = &rest[at + "NodeId(".len()..];
        if let Some(end) = rest.find(')') {
            if let Ok(value) = rest[..end].parse::<u32>() {
                ids.push(NodeId(value));
            }
        }
    }
    ids
}

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

    let ids = collect_ids(&linked.module);
    let unique: HashSet<NodeId> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "merged module has duplicate NodeIds");
}

#[test]
fn merged_riffs_have_unique_node_ids_in_tasks_app() {
    let entry = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/apps/tasks/main.vaab");
    let linked = link(&entry).expect("tasks app should link");
    let ids = collect_ids(&linked.module);
    let unique: HashSet<NodeId> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "merged tasks app should have globally unique NodeIds"
    );

    let dir = std::env::temp_dir().join(format!("vaab_riff_bare_test_{}", std::process::id()));
    let riff_dir = dir.join("vendor").join("rig");
    fs::create_dir_all(&riff_dir).expect("create vendor/rig");
    fs::write(
        riff_dir.join("riff"),
        "riff rig 0.1.0\ndescription test\nby vaab\nlicence MIT\nneeds vaab at 0.1\n",
    )
    .expect("write riff manifest");
    fs::write(
        riff_dir.join("lib.vaab"),
        "to internal_ok(name: Text) returns Bool {\n\
         \x20   return name == \"rig\"\n\
         }\n\
         to check_name(name: Text) returns Bool {\n\
         \x20   return internal_ok(name)\n\
         }\n",
    )
    .expect("write lib.vaab");
    let main_path = dir.join("main.vaab");
    fs::write(
        &main_path,
        "need rig from ./vendor/rig of check_name\n\
         print(check_name(\"rig\"))\n",
    )
    .expect("write main.vaab");

    let linked_bare = link(&main_path).expect("link bare riff with internal helper");
    let bare_ids = collect_ids(&linked_bare.module);
    let bare_unique: HashSet<NodeId> = bare_ids.iter().copied().collect();
    assert_eq!(bare_ids.len(), bare_unique.len());
    let _ = fs::remove_dir_all(&dir);
}


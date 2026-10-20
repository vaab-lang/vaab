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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vaab_include_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &std::path::Path, name: &str, source: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, source).expect("write source");
}

fn run_linked(path: &std::path::Path) -> Vec<String> {
    let linked = link(path).unwrap_or_else(|error| panic!("{error}"));
    let checked = vaab_types::check(&linked.module).unwrap_or_else(|problems| {
        panic!("type check failed: {problems:?}");
    });
    let mut world = vaab_vm::prepare(&linked.module, &checked, vaab_vm::Output::collected());
    match vaab_vm::run(&mut world) {
        Ok(_) => world.output.lines().to_vec(),
        Err(problem) => panic!("runtime error: {problem:?}"),
    }
}

#[test]
fn a_vaab_file_is_included_into_the_same_scope() {
    let dir = scratch("flat");
    write(
        &dir,
        "helpers.vaab",
        "type Greeting {\n\
         \x20   text: Text\n\
         }\n\
         \n\
         to greet(name: Text) returns Greeting {\n\
         \x20   return Greeting.new(text: \"hello, {name}\")\n\
         }\n\
         \n\
         cast Speaker {\n\
         \x20   factory say(name: Text) returns Text = \"hello, {name}\"\n\
         }\n",
    );
    write(
        &dir,
        "main.vaab",
        "need ./helpers.vaab\n\
         let greeting = greet(\"Ada\")\n\
         print(greeting.text)\n\
         print(Speaker.say(\"Ada\"))\n\
         print(helpers.greet(\"Bea\").text)\n",
    );

    assert_eq!(
        run_linked(&dir.join("main.vaab")),
        ["hello, Ada", "hello, Ada", "hello, Bea"]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_named_need_can_point_at_a_vaab_file() {
    let dir = scratch("named");
    write(&dir, "helpers.vaab", "to ping() returns Text = \"pong\"\n");
    write(
        &dir,
        "main.vaab",
        "need helpers from ./helpers\n\
         print(ping())\n\
         print(helpers.ping())\n",
    );

    assert_eq!(run_linked(&dir.join("main.vaab")), ["pong", "pong"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_bare_need_finds_a_sibling_vaab_file() {
    let dir = scratch("sibling");
    write(&dir, "ping_helper.vaab", "to ping() returns Text = \"pong\"\n");
    write(&dir, "main.vaab", "need ping_helper\nprint(ping())\n");

    assert_eq!(run_linked(&dir.join("main.vaab")), ["pong"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn included_files_can_include_files_beside_themselves() {
    let dir = scratch("transitive");
    write(&dir, "sub/b.vaab", "to origin() returns Text = \"from b\"\n");
    write(
        &dir,
        "sub/a.vaab",
        "need ./b.vaab\n\
         to via_a() returns Text = b.origin()\n",
    );
    write(
        &dir,
        "main.vaab",
        "need ./sub/a.vaab\n\
         print(via_a())\n\
         print(origin())\n",
    );

    assert_eq!(run_linked(&dir.join("main.vaab")), ["from b", "from b"]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_file_needed_twice_is_included_once() {
    let dir = scratch("diamond");
    write(&dir, "common.vaab", "to shared() returns Int = 1\n");
    write(
        &dir,
        "a.vaab",
        "need ./common.vaab\n\
         to from_a() returns Int = shared()\n",
    );
    write(
        &dir,
        "b.vaab",
        "need ./common.vaab\n\
         to from_b() returns Int = shared()\n",
    );
    write(
        &dir,
        "main.vaab",
        "need ./a.vaab\n\
         need ./b.vaab\n\
         print(from_a() + from_b())\n",
    );

    assert_eq!(run_linked(&dir.join("main.vaab")), ["2"]);
    let names = exports_of(&link(&dir.join("main.vaab")).expect("link").module);
    assert_eq!(names.iter().filter(|name| name.as_str() == "shared").count(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_cycle_of_included_files_is_an_error() {
    let dir = scratch("cycle");
    write(&dir, "a.vaab", "need ./b.vaab\nto from_a() returns Int = 1\n");
    write(&dir, "b.vaab", "need ./a.vaab\nto from_b() returns Int = 2\n");
    write(&dir, "main.vaab", "need ./a.vaab\nprint(from_a())\n");

    let error = link(&dir.join("main.vaab")).expect_err("cycle should fail");
    assert!(error.contains("cycle detected"), "{error}");
    assert!(error.contains("`a.vaab`"), "{error}");
    assert!(error.contains("`b.vaab`"), "{error}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn of_keeps_the_named_imports_and_prefixes_the_rest() {
    let dir = scratch("of");
    write(
        &dir,
        "helpers.vaab",
        "to greet() returns Text = \"hi\"\n\
         to hidden() returns Text = \"no\"\n",
    );
    write(
        &dir,
        "main.vaab",
        "need ./helpers.vaab of greet\n\
         print(greet())\n",
    );

    assert_eq!(run_linked(&dir.join("main.vaab")), ["hi"]);
    let names = exports_of(&link(&dir.join("main.vaab")).expect("link").module);
    assert!(names.contains("greet"));
    assert!(names.contains("helpers__hidden"));
    assert!(!names.contains("hidden"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn gather_accepts_a_vaab_file_dependency() {
    let dir = scratch("gather");
    write(&dir, "helpers.vaab", "to ping() returns Text = \"pong\"\n");
    write(
        &dir,
        "riff",
        "riff app 0.1.0\nneed helpers from ./helpers.vaab\n",
    );

    let lock = gather(&dir).expect("gather should accept a .vaab file");
    let pinned = lock.path_for("helpers").expect("helpers is pinned");
    assert!(pinned.ends_with("helpers.vaab"), "{}", pinned.display());
    let _ = fs::remove_dir_all(&dir);
}


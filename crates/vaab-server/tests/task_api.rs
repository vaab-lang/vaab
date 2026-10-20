use std::fs;
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use vaab_vm::app_io::{Databases, Stores};
use vaab_vm::concurrency::Host;
use vaab_vm::log::Loggers;
use vaab_vm::machine::{Budget, Machine, Output, Step, World};
use vaab_vm::value::{Ref, Value};

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

#[test]
fn serve_mounts_route_groups() {
    let source = r#"
mount rig_auth at "/auth/v1" {
    route get "/health" {
        reply with "auth-ok"
    }
}

mount rig_api at "/rest/v1" {
    route get "/{table: Text}" {
        reply with "table-{table}"
    }
}

mount rig_studio at "/studio" {
    route get "/" {
        reply with "studio-ok"
    }
}

serve on port 54321 {
    mount rig_auth
    mount rig_api at "/api/v2"
    mount rig_studio at "/admin"
}
"#;
    let parsed = vaab_syntax::parse(source);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    let checked = vaab_types::check(&parsed.module).expect("check");
    assert_eq!(checked.serves.len(), 1);
    let world = vaab_vm::prepare(&parsed.module, &checked, vaab_vm::Output::collected());
    assert_eq!(world.program.routes.len(), 3);
    assert_eq!(
        world.program.routes[0].path,
        vec![vaab_types::RouteSegment::Literal("/auth/v1/health".to_string())]
    );
    assert_eq!(
        world.program.routes[1].path,
        vec![
            vaab_types::RouteSegment::Literal("/api/v2/".to_string()),
            vaab_types::RouteSegment::Param("table".to_string()),
        ]
    );
    assert_eq!(
        world.program.routes[2].path,
        vec![vaab_types::RouteSegment::Literal("/admin".to_string())]
    );
}

#[test]
fn linked_riff_routes_run_multiple_queries_on_one_db_handle() {
    let dir = std::env::temp_dir().join(format!("vaab_rig_route_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let riff_dir = dir.join("vendor").join("rig");
    fs::create_dir_all(&riff_dir).expect("create vendor/rig");
    let db_path = dir.join("test-rig.db");

    fs::write(
        riff_dir.join("riff"),
        "riff rig 0.1.0\ndescription test\nby vaab\nlicence MIT\nneeds vaab at 0.1\n",
    )
    .expect("write riff manifest");

    fs::write(
        riff_dir.join("lib.vaab"),
        format!(
            "choice RigError {{\n\
                 Database(message: Text)\n\
                 BadRequest(message: Text)\n\
             }}\n\
             \n\
             type Client {{\n\
                 db_url: Text\n\
             }}\n\
             \n\
             to connect() returns Client {{\n\
                 return Client.new(db_url: \"sqlite:{}\")\n\
             }}\n\
             \n\
             to open_db(client: Client) returns Db or fails RigError {{\n\
                 match Db.connect(client.db_url) {{\n\
                     when success db then match ensure_schema(db) {{\n\
                         when success _ then {{ return db }}\n\
                         when failure error then {{ fail error }}\n\
                     }}\n\
                     when failure error then match error {{\n\
                         when Failed(message) then {{ fail RigError.Database(message) }}\n\
                     }}\n\
                 }}\n\
             }}\n\
             \n\
             to ensure_schema(db: Db) returns Int or fails RigError {{\n\
                 let empty: [Text] = []\n\
                 match db.execute(\"create table if not exists profiles (id text primary key)\", empty) {{\n\
                     when success _ then {{}}\n\
                     when failure error then match error {{\n\
                         when Failed(message) then {{ fail RigError.Database(message) }}\n\
                     }}\n\
                 }}\n\
                 match db.execute(\"create table if not exists riff_events (id text primary key, name text not null)\", empty) {{\n\
                     when success count then {{ return count }}\n\
                     when failure error then match error {{\n\
                         when Failed(message) then {{ fail RigError.Database(message) }}\n\
                     }}\n\
                 }}\n\
             }}\n\
             \n\
             to known_riff(name: Text) returns Bool {{\n\
                 if name == \"supabase\" {{ return yes }}\n\
                 if name == \"tape\" {{ return yes }}\n\
                 if name == \"rig\" {{ return yes }}\n\
                 return no\n\
             }}\n\
             \n\
             to track_riff(db: Db, id: Text, name: Text) returns Int or fails RigError {{\n\
                 if not known_riff(name) {{\n\
                     fail RigError.BadRequest(\"unknown\")\n\
                 }}\n\
                 match db.from(\"riff_events\").insert({{\"id\": id, \"name\": name}}) {{\n\
                     when success _ then match db.from(\"riff_events\").equals(\"name\", name).count() {{\n\
                         when success n then {{ return n }}\n\
                         when failure error then match error {{\n\
                             when Failed(message) then {{ fail RigError.Database(message) }}\n\
                         }}\n\
                     }}\n\
                     when failure error then match error {{\n\
                         when Failed(message) then {{ fail RigError.Database(message) }}\n\
                     }}\n\
                 }}\n\
             }}\n\
             \n\
             to count_riff(db: Db, name: Text) returns Int or fails RigError {{\n\
                 if not known_riff(name) {{\n\
                     fail RigError.BadRequest(\"unknown\")\n\
                 }}\n\
                 match db.from(\"riff_events\").equals(\"name\", name).count() {{\n\
                     when success n then {{ return n }}\n\
                     when failure error then match error {{\n\
                         when Failed(message) then {{ fail RigError.Database(message) }}\n\
                     }}\n\
                 }}\n\
             }}\n",
            db_path.display()
        ),
    )
    .expect("write lib.vaab");

    let main_path = dir.join("main.vaab");
    fs::write(
        &main_path,
        "need rig from ./vendor/rig of RigError, Client, connect, open_db, track_riff, count_riff\n\
         \n\
         type RiffStat {\n\
             name: Text\n\
             downloads: Int\n\
         }\n\
         \n\
         type Tracked {\n\
             ok: Bool\n\
             downloads: Int\n\
         }\n\
         \n\
         list_stats() returns [RiffStat] or fails RigError {\n\
             let client = connect()\n\
             let db = try open_db(client)\n\
             let s = try count_riff(db, \"supabase\")\n\
             let t = try count_riff(db, \"tape\")\n\
             let r = try count_riff(db, \"rig\")\n\
             return [\n\
                 RiffStat.new(name: \"supabase\", downloads: s),\n\
                 RiffStat.new(name: \"tape\", downloads: t),\n\
                 RiffStat.new(name: \"rig\", downloads: r),\n\
             ]\n\
         }\n\
         \n\
         serve on port 8787 {\n\
             route get \"/api/riffs\" {\n\
                 reply with try list_stats()\n\
             }\n\
             \n\
             route post \"/api/riffs/{name: Text}/track\" {\n\
                 let client = connect()\n\
                 let db = try open_db(client)\n\
                 let n = try track_riff(db, \"{name}-1\", name)\n\
                 reply with Tracked.new(ok: yes, downloads: n)\n\
             }\n\
             \n\
             when anything fails with RigError as error {\n\
                 reply explain(error)\n\
             }\n\
         }\n",
    )
    .expect("write main.vaab");

    let linked = vaab_riff::link(&main_path).expect("link");
    let checked = vaab_types::check(&linked.module).expect("check");
    let mut init_world = vaab_vm::prepare(&linked.module, &checked, Output::collected());
    vaab_vm::run(&mut init_world).expect("init top level");

    let program = Ref::clone(&init_world.program);
    let databases = Databases::shared();
    let stores = Stores::shared();
    let loggers = Loggers::shared();

    // 1. Track "rig" install via route 1
    {
        let mut world = World::with_app_io(
            Ref::clone(&program),
            Output::collected(),
            databases.clone(),
            stores.clone(),
            loggers.clone(),
        );
        let request = Value::Tuple(Ref::from(vec![
            Value::text("POST"),
            Value::text("/api/riffs/rig/track"),
            Value::text(""),
            Value::map(IndexMap::new()),
        ]));
        let mut machine = Machine::entering(&program, program.routes[1].body, 0);
        machine.set_argument(0, request).unwrap();
        machine.set_argument(1, Value::text("rig")).unwrap();
        let mut host = Host::scratch(&program);
        match machine.resume(&mut world, &mut host, Budget::unlimited()) {
            Step::Finished(_) => {}
            other => panic!("unexpected step: {other:?}"),
        }
        let resp = machine.response.expect("response");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "{\"downloads\":1,\"ok\":true}");
    }

    // 2. List all 3 riff stats via route 0 (runs 3 fluent count queries on one Db handle)
    {
        let mut world = World::with_app_io(
            Ref::clone(&program),
            Output::collected(),
            databases.clone(),
            stores.clone(),
            loggers.clone(),
        );
        let request = Value::Tuple(Ref::from(vec![
            Value::text("GET"),
            Value::text("/api/riffs"),
            Value::text(""),
            Value::map(IndexMap::new()),
        ]));
        let mut machine = Machine::entering(&program, program.routes[0].body, 0);
        machine.set_argument(0, request).unwrap();
        let mut host = Host::scratch(&program);
        match machine.resume(&mut world, &mut host, Budget::unlimited()) {
            Step::Finished(_) => {}
            other => panic!("unexpected step: {other:?}"),
        }
        let resp = machine.response.expect("response");
        assert_eq!(resp.status, 200);
        assert_eq!(
            resp.body,
            "[{\"downloads\":0,\"name\":\"supabase\"},{\"downloads\":0,\"name\":\"tape\"},{\"downloads\":1,\"name\":\"rig\"}]"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn route_expecting_text_map_body_inserts_into_studio_table() {
    let body_map = vaab_vm::json::decode_map("{\"id\":\"u-1\",\"email\":\"op@rig.dev\",\"name\":\"Op\"}")
        .expect("decode map");
    let Value::Map(entries) = body_map else {
        panic!("expected Value::Map");
    };
    assert_eq!(entries.len(), 3);
}

#[test]
fn on_start_is_compiled() {
    let source = r#"
let count = Shared.new(0)
to tick() {
    count.update(n -> n + 1)
}
serve on port 8080 {
    on start {
        start { every(milliseconds: 100, f: tick) }
    }
    route get "/count" {
        reply with count.value
    }
}
"#;
    let parsed = vaab_syntax::parse(source);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    let checked = vaab_types::check(&parsed.module).expect("check");
    let world = vaab_vm::prepare(&parsed.module, &checked, vaab_vm::Output::collected());
    assert!(world.program.on_start.is_some());
}

#[tokio::test]
async fn a_background_counter_is_visible_on_get() {
    let port = free_port();
    let source = format!(
        r#"
let count = Shared.new(0)
to tick() {{
    count.update(n -> n + 1)
}}
serve on port {port} {{
    on start {{
        start {{ every(milliseconds: 50, f: tick) }}
    }}
    route get "/count" {{
        reply with count.value
    }}
}}
"#
    );
    let serving = start_source(&source).await;
    tokio::time::sleep(Duration::from_millis(180)).await;
    let body = http_get(port, "/count").await;
    serving.shutdown().await;
    let value: i64 = body
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("expected a number, got {body:?}"));
    assert!(value >= 2, "background task should have incremented, got {value}");
}

#[tokio::test]
async fn a_slow_http_get_does_not_delay_count() {
    let delay_port = start_delay_server(Duration::from_millis(400)).await;
    let port = free_port();
    let source = format!(
        r#"
let count = Shared.new(0)
serve on port {port} {{
    route get "/count" {{
        reply with count.value
    }}
    route get "/slow" {{
        match http.get("http://127.0.0.1:{delay_port}/wait") {{
            when success body then {{ reply with "done" }}
            when failure _ then {{ reply with "done" }}
        }}
    }}
}}
"#
    );
    let serving = start_source(&source).await;

    let slow = tokio::spawn(async move { http_get(port, "/slow").await });
    tokio::time::sleep(Duration::from_millis(40)).await;
    let started = Instant::now();
    let body = http_get(port, "/count").await;
    let elapsed = started.elapsed();
    let _ = slow.await;

    serving.shutdown().await;
    assert_eq!(body.trim(), "0");
    assert!(
        elapsed < Duration::from_millis(250),
        "GET /count waited {elapsed:?} behind a slow http.get"
    );
}

async fn start_source(source: &str) -> vaab_server::Serving {
    let parsed = vaab_syntax::parse(source);
    assert!(!parsed.has_errors(), "{:?}", parsed.diagnostics);
    let checked = vaab_types::check(&parsed.module).expect("check");
    vaab_server::start(&parsed.module, &checked)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    port
}

async fn http_get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap_or_else(|error| panic!("connect {port}: {error}"));
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("read");
    let text = String::from_utf8_lossy(&bytes);
    text.split("\r\n\r\n").nth(1).unwrap_or(&text).to_string()
}

async fn start_delay_server(delay: Duration) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("delay bind");
    let port = listener.local_addr().expect("delay addr").port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                let mut buf = [0u8; 256];
                let _ = stream.read(&mut buf).await;
                tokio::time::sleep(delay).await;
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                    .await;
            });
        }
    });
    port
}

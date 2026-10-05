use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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

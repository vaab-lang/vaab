use std::time::Duration;

use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn a_joined_client_receives_a_broadcast() {
    let port = free_port();
    let source = format!(
        r#"
let hub = Hub.new(of: Text)
to listen(client: Socket) {{
    while yes {{
        match receive from client.messages {{
            when found _ then {{}}
            when nothing then {{ return }}
        }}
    }}
}}
serve on port {port} {{
    socket "/ws" as client {{
        hub.join(client)
        listen(client)
    }}
    route get "/ping" {{
        hub.broadcast("hello")
        reply with "ok"
    }}
}}
"#
    );
    let serving = start_source(&source).await;
    let (mut socket, _) = connect_ws(port, "/ws").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let body = http_get(port, "/ping").await;
    assert_eq!(body.trim(), "\"ok\"");
    let frame = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("broadcast arrived")
        .expect("a frame")
        .expect("a readable frame");
    serving.shutdown().await;
    assert_eq!(frame, Message::Text("hello".into()));
}

#[tokio::test]
async fn a_slow_client_is_dropped_when_its_buffer_fills() {
    let port = free_port();
    let chunk = "x".repeat(4096);
    let source = format!(
        r#"
let hub = Hub.new(of: Text)
to listen(client: Socket) {{
    while yes {{
        match receive from client.messages {{
            when found _ then {{}}
            when nothing then {{ return }}
        }}
    }}
}}
serve on port {port} {{
    socket "/ws" as client {{
        hub.join(client)
        listen(client)
    }}
    route get "/burst" {{
        repeat 64 times {{
            hub.broadcast("{chunk}")
        }}
        reply with "ok"
    }}
    route get "/after" {{
        hub.broadcast("still-here")
        reply with "ok"
    }}
}}
"#
    );
    let serving = start_source(&source).await;

    let (mut slow, _) = connect_ws(port, "/ws").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let body = http_get(port, "/burst").await;
    assert_eq!(body.trim(), "\"ok\"");

    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match slow.next().await {
                Some(Ok(Message::Close(_))) | None => return true,
                Some(Err(_)) => return true,
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(closed, "the slow client should have been disconnected");

    let (mut fresh, _) = connect_ws(port, "/ws").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let after = http_get(port, "/after").await;
    assert_eq!(after.trim(), "\"ok\"");
    let frame = tokio::time::timeout(Duration::from_secs(2), fresh.next())
        .await
        .expect("a later client still receives")
        .expect("a frame")
        .expect("a readable frame");
    serving.shutdown().await;
    assert_eq!(frame, Message::Text("still-here".into()));
}

#[tokio::test]
async fn an_unknown_path_is_still_not_found() {
    let port = free_port();
    let source = format!(
        r#"
let hub = Hub.new(of: Text)
to listen(client: Socket) {{
    while yes {{
        match receive from client.messages {{
            when found _ then {{}}
            when nothing then {{ return }}
        }}
    }}
}}
serve on port {port} {{
    socket "/ws" as client {{
        hub.join(client)
        listen(client)
    }}
}}
"#
    );
    let serving = start_source(&source).await;
    let body = http_get(port, "/missing").await;
    serving.shutdown().await;
    assert!(body.contains("not found"), "{body}");
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

async fn connect_ws(
    port: u16,
    path: &str,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
) {
    let url = format!("ws://127.0.0.1:{port}{path}");
    tokio::time::timeout(Duration::from_secs(2), tokio_tungstenite::connect_async(url))
        .await
        .expect("websocket connect")
        .unwrap_or_else(|error| panic!("websocket handshake: {error}"))
}

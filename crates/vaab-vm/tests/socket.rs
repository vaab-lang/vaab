//! Live WebSocket client: a local echo server, no public network.

mod support;

use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use support::output;
use tokio_tungstenite::tungstenite::{accept, Message};

fn single_worker() {
    std::env::set_var("VAAB_WORKERS", "1");
}

fn serve_once(
    handle: impl FnOnce(tokio_tungstenite::tungstenite::WebSocket<std::net::TcpStream>) + Send + 'static,
) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local websocket");
    let port = listener.local_addr().expect("local address").port();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept a websocket client");
        let socket = accept(stream).expect("websocket handshake");
        handle(socket);
    });
    thread::sleep(Duration::from_millis(50));
    port
}

fn run_fallible(body: &str) -> String {
    format!(
        "to run() returns Text or fails SocketError {{\n\
         {body}\n\
         }}\n\
         match run() {{\n\
         \x20   when success text then print(text)\n\
         \x20   when failure error then print(error)\n\
         }}\n"
    )
}

#[test]
fn a_socket_echoes_text_and_binary_then_closes() {
    single_worker();
    let port = serve_once(|mut socket| {
        if let Ok(Message::Text(text)) = socket.read() {
            let _ = socket.send(Message::Text(text));
        }
        let _ = socket.send(Message::Binary(vec![0xab, 0xcd].into()));
        let _ = socket.close(None);
    });
    let source = run_fallible(&format!(
        "    let socket = try Socket.connect(\"ws://127.0.0.1:{port}\")\n\
         \x20   try socket.send(\"hello\")\n\
         \x20   print(receive from socket.messages otherwise \"no-text\")\n\
         \x20   match receive from socket.binary {{\n\
         \x20       when found bytes then print(bytes.to_hex())\n\
         \x20       when nothing then print(\"no-bin\")\n\
         \x20   }}\n\
         \x20   match receive from socket.messages {{\n\
         \x20       when found message then print(message)\n\
         \x20       when nothing then {{\n\
         \x20           match socket.closed_because {{\n\
         \x20               when found _ then print(\"closed\")\n\
         \x20               when nothing then print(\"closed\")\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   return success \"done\"\n"
    ));
    assert_eq!(output(&source), ["hello", "abcd", "closed", "done"]);
}

#[test]
fn a_forced_disconnect_closes_the_channel_without_crashing() {
    single_worker();
    let port = serve_once(|mut socket| {
        let _ = socket.send(Message::Text("ping".into()));
        drop(socket);
    });
    let source = run_fallible(&format!(
        "    let socket = try Socket.connect(\"ws://127.0.0.1:{port}\")\n\
         \x20   print(receive from socket.messages otherwise \"no-text\")\n\
         \x20   match receive from socket.messages {{\n\
         \x20       when found message then print(message)\n\
         \x20       when nothing then print(\"dropped\")\n\
         \x20   }}\n\
         \x20   match socket.closed_because {{\n\
         \x20       when found _ then print(\"reason\")\n\
         \x20       when nothing then print(\"no-reason\")\n\
         \x20   }}\n\
         \x20   return success \"ok\"\n"
    ));
    let lines = output(&source);
    assert_eq!(lines.first().map(String::as_str), Some("ping"));
    assert_eq!(lines.get(1).map(String::as_str), Some("dropped"));
    assert_eq!(lines.get(2).map(String::as_str), Some("reason"));
    assert_eq!(lines.get(3).map(String::as_str), Some("ok"));
}

#[test]
fn a_refused_connect_is_a_socket_error() {
    single_worker();
    let source = run_fallible(
        "    let socket = try Socket.connect(\"ws://127.0.0.1:1\")\n\
         \x20   return success \"connected\"\n",
    );
    let lines = output(&source);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_ne!(lines[0], "connected");
}

#[test]
fn connect_headers_and_buffer_are_accepted() {
    single_worker();
    let port = serve_once(|mut socket| {
        if let Ok(Message::Text(text)) = socket.read() {
            let _ = socket.send(Message::Text(text));
        }
        let _ = socket.close(None);
    });
    let source = run_fallible(&format!(
        "    let socket = try Socket.connect(\"ws://127.0.0.1:{port}\", headers: {{\"X-Test\": \"1\"}}, buffer: 4)\n\
         \x20   try socket.send(\"ok\")\n\
         \x20   print(receive from socket.messages otherwise \"missing\")\n\
         \x20   socket.close()\n\
         \x20   return success \"ok\"\n"
    ));
    assert_eq!(output(&source), ["ok", "ok"]);
}

//! WebSocket client I/O, run on a dedicated Tokio runtime.
//!
//! Connect, send and close complete through [`IoHub`](crate::io::IoHub) tickets
//! so the scheduler thread never blocks. Incoming frames are posted to a mailbox
//! the host drains onto ordinary channels.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc as tokio_mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::io::{IoCompleter, IoOutcome};

pub const DEFAULT_BUFFER: i64 = 1024;

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("vaab-ws")
            .build()
            .expect("the websocket runtime could start")
    })
}

pub enum SocketCmd {
    Send {
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Close {
        reply: oneshot::Sender<Result<(), String>>,
    },
}

pub enum SocketEvent {
    Text {
        socket: u32,
        text: String,
        ack: oneshot::Sender<()>,
    },
    Binary {
        socket: u32,
        bytes: Vec<u8>,
        ack: oneshot::Sender<()>,
    },
    Closed {
        socket: u32,
        reason: Option<String>,
    },
}

pub struct LiveSocket {
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

#[derive(Clone)]
pub struct SocketMailbox {
    events: Sender<SocketEvent>,
    io: IoCompleter,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl SocketMailbox {
    pub fn new(
        events: Sender<SocketEvent>,
        io: IoCompleter,
        notify: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> SocketMailbox {
        SocketMailbox { events, io, notify }
    }

    fn post(&self, event: SocketEvent) {
        let _ = self.events.send(event);
        self.io.pulse();
        if let Some(notify) = &self.notify {
            notify();
        }
    }
}

pub async fn connect(
    url: String,
    headers: HashMap<String, String>,
) -> Result<LiveSocket, String> {
    let mut request = url.into_client_request().map_err(|error| error.to_string())?;
    for (name, value) in headers {
        let name = HeaderName::try_from(name).map_err(|error| error.to_string())?;
        let value = HeaderValue::try_from(value).map_err(|error| error.to_string())?;
        request.headers_mut().append(name, value);
    }
    let (stream, _) = connect_async(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(LiveSocket { stream })
}

impl LiveSocket {
    pub fn start(self, socket: u32, mailbox: SocketMailbox) -> tokio_mpsc::Sender<SocketCmd> {
        let (commands, command_rx) = tokio_mpsc::channel(8);
        runtime().spawn(connection_loop(self.stream, socket, command_rx, mailbox));
        commands
    }
}

pub fn submit_connect(
    url: String,
    headers: HashMap<String, String>,
    ticket: u64,
    completer: IoCompleter,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    runtime().spawn(async move {
        let outcome = match connect(url, headers).await {
            Ok(live) => IoOutcome::SocketConnect(Ok(live)),
            Err(message) => IoOutcome::SocketConnect(Err(message)),
        };
        completer.complete(ticket, outcome);
        if let Some(notify) = notify {
            notify();
        }
    });
}

pub fn submit_command(
    writer: tokio_mpsc::Sender<SocketCmd>,
    command: impl FnOnce(oneshot::Sender<Result<(), String>>) -> SocketCmd + Send + 'static,
    ticket: u64,
    completer: IoCompleter,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
) {
    runtime().spawn(async move {
        let (done, done_rx) = oneshot::channel();
        let outcome = if writer.send(command(done)).await.is_err() {
            Err("the socket is closed".to_string())
        } else {
            done_rx
                .await
                .unwrap_or_else(|_| Err("the socket is closed".to_string()))
        };
        completer.complete(ticket, IoOutcome::Unit(outcome));
        if let Some(notify) = notify {
            notify();
        }
    });
}

async fn connection_loop(
    stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    socket: u32,
    mut commands: tokio_mpsc::Receiver<SocketCmd>,
    mailbox: SocketMailbox,
) {
    let (mut sink, mut incoming) = stream.split();
    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(SocketCmd::Send { text, reply }) => {
                        let result = sink
                            .send(Message::Text(text.into()))
                            .await
                            .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                    }
                    Some(SocketCmd::Close { reply }) => {
                        let result = sink
                            .send(Message::Close(None))
                            .await
                            .map_err(|error| error.to_string());
                        let _ = reply.send(result);
                        mailbox.post(SocketEvent::Closed {
                            socket,
                            reason: Some("closed".to_string()),
                        });
                        break;
                    }
                    None => break,
                }
            }
            frame = incoming.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        let (ack, wait) = oneshot::channel();
                        mailbox.post(SocketEvent::Text {
                            socket,
                            text: text.to_string(),
                            ack,
                        });
                        let _ = wait.await;
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let (ack, wait) = oneshot::channel();
                        mailbox.post(SocketEvent::Binary {
                            socket,
                            bytes: bytes.to_vec(),
                            ack,
                        });
                        let _ = wait.await;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        let reason = frame
                            .and_then(|frame| {
                                let reason = frame.reason.to_string();
                                if reason.is_empty() {
                                    None
                                } else {
                                    Some(reason)
                                }
                            })
                            .or_else(|| Some("connection closed".to_string()));
                        mailbox.post(SocketEvent::Closed { socket, reason });
                        break;
                    }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        mailbox.post(SocketEvent::Closed {
                            socket,
                            reason: Some(error.to_string()),
                        });
                        break;
                    }
                    None => {
                        mailbox.post(SocketEvent::Closed {
                            socket,
                            reason: Some("connection closed".to_string()),
                        });
                        break;
                    }
                }
            }
        }
    }
}

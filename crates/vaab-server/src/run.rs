use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{HeaderValue, CONNECTION, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_tungstenite::WebSocketStream;
use indexmap::IndexMap;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use vaab_syntax::Module;
use vaab_types::Checked;
use vaab_vm::bytecode::Program;
use vaab_vm::log::server_logger;
use vaab_vm::machine::{HttpResponse, Machine, Output};
use vaab_vm::value::{Key, Ref, Value};
use vaab_vm::LiveRuntime;

use crate::{playground, router};

pub struct Serving {
    pub port: u16,
    state: Arc<ServerState>,
    stop: Option<oneshot::Sender<()>>,
    accept: Option<tokio::task::JoinHandle<()>>,
}

impl Serving {
    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(accept) = self.accept.take() {
            let _ = accept.await;
        }
        self.state.runtime.shutdown();
    }
}

pub async fn serve_file(_source: &str, module: &Module, checked: &Checked) -> Result<(), String> {
    let serving = start(module, checked).await?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = std::future::pending::<()>() => {}
    }
    serving.state.log.info("shutting down");
    serving.shutdown().await;
    Ok(())
}

pub async fn start(module: &Module, checked: &Checked) -> Result<Serving, String> {
    let serve = checked
        .serves
        .values()
        .next()
        .ok_or_else(|| "this file has no `serve on port ...` block".to_string())?;

    let mut world = vaab_vm::prepare(module, checked, Output::collected());
    if let Err(problem) = vaab_vm::run(&mut world) {
        return Err(format!("server startup failed: {:?}", problem.fault));
    }
    let program = Ref::clone(&world.program);
    if program.routes.is_empty() {
        return Err("this server has no compiled routes".to_string());
    }

    let address = SocketAddr::from(([0, 0, 0, 0], serve.port));
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| format!("could not listen on port {}: {}", serve.port, error))?;

    let runtime = Arc::new(LiveRuntime::start(world));
    let top = runtime.run_to_completion(Machine::start(&program));
    if let Err(error) = top.value {
        runtime.shutdown();
        return Err(format!("the file's top-level statements failed: {:?}", error.fault));
    }
    if let Some(body) = program.on_start {
        let started = runtime.run_to_completion(Machine::entering(&program, body, 0));
        if let Err(error) = started.value {
            runtime.shutdown();
            return Err(format!("`on start` failed: {:?}", error.fault));
        }
    }

    let log = server_logger();
    log.info(&format!("listening on http://127.0.0.1:{}", serve.port));
    let state = Arc::new(ServerState {
        program,
        runtime,
        log,
    });

    let (stop, stop_rx) = oneshot::channel();
    let accept_state = Arc::clone(&state);
    let accept = tokio::spawn(async move {
        accept_loop(listener, accept_state, stop_rx).await;
    });

    Ok(Serving {
        port: serve.port,
        state,
        stop: Some(stop),
        accept: Some(accept),
    })
}

async fn accept_loop(
    listener: TcpListener,
    state: Arc<ServerState>,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut stop => break,
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else { break };
                let io = TokioIo::new(stream);
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let log = state.log.clone();
                    let service = service_fn(move |request| handle(state.clone(), request));
                    if let Err(error) = http1::Builder::new()
                        .serve_connection(io, service)
                        .with_upgrades()
                        .await
                    {
                        log.write(
                            vaab_vm::LogLevel::Warn,
                            "connection error",
                            &[("peer", &peer.to_string()), ("error", &error.to_string())],
                        );
                    }
                });
            }
        }
    }
}

struct ServerState {
    program: Ref<Program>,
    runtime: Arc<LiveRuntime>,
    log: vaab_vm::Logger,
}

async fn handle(
    state: Arc<ServerState>,
    request: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let started = std::time::Instant::now();
    let method = request.method().as_str().to_string();
    if method.eq_ignore_ascii_case("OPTIONS") {
        return Ok(cors_response(StatusCode::NO_CONTENT, "text/plain", Bytes::new()));
    }

    let path = request.uri().path().to_string();

    if let Some(matched) = router::find_socket_route(&state.program.routes, &path) {
        if is_websocket_upgrade(&request) {
            return upgrade_socket(state, request, matched.handler).await;
        }
        return Ok(cors_response(
            StatusCode::BAD_REQUEST,
            "application/json",
            Bytes::from("{\"error\":\"websocket upgrade required\"}"),
        ));
    }

    if method.eq_ignore_ascii_case("POST") && path == "/api/run" {
        let body_bytes = http_body_util::BodyExt::collect(request.into_body())
            .await
            .map(|collected| collected.to_bytes())
            .unwrap_or_default();
        let source = serde_json::from_slice::<serde_json::Value>(&body_bytes)
            .ok()
            .and_then(|value| value.get("source").and_then(|source| source.as_str()).map(str::to_string))
            .unwrap_or_default();
        let response = playground::run(&source);
        let elapsed_ms = started.elapsed().as_millis().to_string();
        state.log.write(
            vaab_vm::LogLevel::Info,
            "request",
            &[
                ("method", &method),
                ("path", &path),
                ("status", &response.status.to_string()),
                ("ms", &elapsed_ms),
            ],
        );
        return Ok(cors_response(
            StatusCode::from_u16(response.status).unwrap_or(StatusCode::OK),
            &response.content_type,
            Bytes::from(response.body),
        ));
    }

    let mut headers = IndexMap::new();
    for (name, value) in request.headers() {
        if let Ok(text) = value.to_str() {
            headers.insert(Key(Value::text(name.as_str())), Value::text(text));
        }
    }
    let body_bytes = http_body_util::BodyExt::collect(request.into_body())
        .await
        .map(|collected| collected.to_bytes())
        .unwrap_or_default();

    let matched = router::find_route(&state.program.routes, &method, &path);
    let response = if let Some(matched) = matched {
        let route = state.program.routes[matched.handler].clone();
        let body_text = String::from_utf8_lossy(&body_bytes).into_owned();
        let state = Arc::clone(&state);
        let method_for_route = method.clone();
        let path_for_route = path.clone();
        tokio::task::spawn_blocking(move || {
            run_route(
                &state,
                &route,
                &method_for_route,
                &path_for_route,
                body_text,
                headers,
                matched.params,
            )
        })
        .await
        .unwrap_or_else(|_| HttpResponse::json(500, "{\"error\":\"route failed\"}".to_string()))
    } else {
        HttpResponse::json(404, "{\"error\":\"not found\"}".to_string())
    };

    let elapsed_ms = started.elapsed().as_millis().to_string();
    state.log.write(
        vaab_vm::LogLevel::Info,
        "request",
        &[
            ("method", &method),
            ("path", &path),
            ("status", &response.status.to_string()),
            ("ms", &elapsed_ms),
        ],
    );

    Ok(cors_response(
        StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        &response.content_type,
        Bytes::from(response.body),
    ))
}

fn cors_response(status: StatusCode, content_type: &str, body: Bytes) -> Response<Full<Bytes>> {
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-headers", "authorization, content-type")
        .header("access-control-allow-methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS");
    // Browsers only open the Basic-auth prompt when a 401 carries this challenge.
    if status == StatusCode::UNAUTHORIZED {
        builder = builder.header("www-authenticate", "Basic realm=\"vaab\"");
    }
    builder
        .body(Full::new(body))
        .expect("response body is valid")
}

fn run_route(
    state: &ServerState,
    route: &vaab_vm::bytecode::RouteHandler,
    method: &str,
    path: &str,
    body: String,
    headers: IndexMap<Key, Value>,
    params: Vec<String>,
) -> HttpResponse {
    let request = Value::Tuple(Ref::from(vec![
        Value::Text(Ref::from(method)),
        Value::Text(Ref::from(path)),
        Value::Text(Ref::from(body.as_str())),
        Value::map(headers),
    ]));

    if let Some(before_body) = state.program.before_route {
        let mut before_machine = Machine::entering(&state.program, before_body, 0);
        let _ = before_machine.set_argument(0, request.clone());
        let before = state.runtime.run_to_completion(before_machine);
        if let Some(response) = before.response {
            return response;
        }
    }

    let mut machine = Machine::entering(&state.program, route.body, 0);
    let _ = machine.set_argument(0, request);
    for (slot, value) in params.into_iter().enumerate() {
        let expects_int = matches!(
            route.path_param_types.get(slot),
            Some(vaab_types::Type::Int)
        );
        let argument = if expects_int {
            match value.parse::<i64>() {
                Ok(number) => Value::Int(number),
                Err(_) => {
                    return HttpResponse::json(
                        400,
                        "{\"error\":\"invalid path parameter\"}".to_string(),
                    );
                }
            }
        } else {
            Value::Text(Ref::from(value.as_str()))
        };
        let _ = machine.set_argument((slot + 1) as u32, argument);
    }
    if route.expects_body {
        let slot = (route.path_param_count + 1) as u32;
        let argument = if let Some(shape) = route.body_shape {
            match vaab_vm::json::decode_typed(
                &body,
                shape,
                &state.program.json_shapes,
                &state.program.layouts,
                &state.program.variants,
            ) {
                Ok(decoded) => decoded,
                Err(_) => {
                    return HttpResponse::json(400, "{\"error\":\"invalid body\"}".to_string());
                }
            }
        } else {
            match route
                .body_layout
                .and_then(|index| state.program.layouts.get(index as usize).cloned())
            {
                Some(layout) => match vaab_vm::json::decode_record(&body, layout) {
                    Ok(record) => record,
                    Err(_) => {
                        return HttpResponse::json(400, "{\"error\":\"invalid body\"}".to_string());
                    }
                },
                None if route.body_is_map => match vaab_vm::json::decode_map(&body) {
                    Ok(map) => map,
                    Err(_) => {
                        return HttpResponse::json(400, "{\"error\":\"invalid body\"}".to_string());
                    }
                },
                None => Value::Text(Ref::from(body.as_str())),
            }
        };
        let _ = machine.set_argument(slot, argument);
    }

    let done = state.runtime.run_to_completion(machine);
    if let Some(response) = done.response {
        return response;
    }
    if let Ok(Value::Failure(held)) = &done.value {
        if let Some(error_body) = state.program.error_handler {
            let mut error_machine = Machine::entering(&state.program, error_body, 0);
            let _ = error_machine.set_argument(0, held.as_ref().clone());
            let handled = state.runtime.run_to_completion(error_machine);
            if let Some(response) = handled.response {
                return response;
            }
        }
        if let Ok(body) = vaab_vm::json::encode(held) {
            return HttpResponse::json(400, body);
        }
    }
    if let Err(error) = &done.value {
        state.log.write(
            vaab_vm::LogLevel::Error,
            "route fault",
            &[
                ("method", method),
                ("path", path),
                ("fault", &format!("{:?}", error.fault)),
            ],
        );
    }

    HttpResponse::json(500, "{\"error\":\"route failed\"}".to_string())
}

fn is_websocket_upgrade(request: &Request<hyper::body::Incoming>) -> bool {
    let upgrade = request
        .headers()
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .map_or(false, |value| value.eq_ignore_ascii_case("websocket"));
    let connection = request
        .headers()
        .get(CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map_or(false, |value| value.to_ascii_lowercase().contains("upgrade"));
    request.method() == hyper::Method::GET && upgrade && connection
}

async fn upgrade_socket(
    state: Arc<ServerState>,
    request: Request<hyper::body::Incoming>,
    handler: usize,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let key = request
        .headers()
        .get("sec-websocket-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if key.is_empty() {
        return Ok(cors_response(
            StatusCode::BAD_REQUEST,
            "application/json",
            Bytes::from("{\"error\":\"missing websocket key\"}"),
        ));
    }

    let accept = derive_accept_key(key.as_bytes());
    let on_upgrade = hyper::upgrade::on(request);
    let program = Ref::clone(&state.program);
    let runtime = Arc::clone(&state.runtime);
    let route = state.program.routes[handler].clone();
    tokio::spawn(async move {
        let upgraded = match on_upgrade.await {
            Ok(upgraded) => upgraded,
            Err(error) => {
                state.log.write(
                    vaab_vm::LogLevel::Warn,
                    "websocket upgrade failed",
                    &[("error", &error.to_string())],
                );
                return;
            }
        };
        let mut config = WebSocketConfig::default();
        config.max_write_buffer_size = 16 * 1024;
        config.write_buffer_size = 1024;
        let stream = WebSocketStream::from_raw_socket(
            TokioIo::new(upgraded),
            Role::Server,
            Some(config),
        )
        .await;
        let live = vaab_vm::socket::LiveSocket::from_stream(stream);
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        runtime.attach_socket_and_spawn(
            live,
            move |socket| {
                let mut machine = Machine::entering(&program, route.body, 0);
                let _ = machine.set_argument(0, socket);
                machine
            },
            Some(Box::new(move |done| {
                let _ = done_tx.send(done);
            })),
        );
        let _ = done_rx.await;
    });

    Ok(Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, HeaderValue::from_static("websocket"))
        .header(CONNECTION, HeaderValue::from_static("Upgrade"))
        .header("sec-websocket-accept", accept)
        .body(Full::new(Bytes::new()))
        .expect("upgrade response is valid"))
}

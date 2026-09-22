use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use indexmap::IndexMap;
use tokio::net::TcpListener;
use vaab_syntax::Module;
use vaab_types::Checked;
use vaab_vm::app_io::{Databases, Stores};
use vaab_vm::bytecode::Program;
use vaab_vm::concurrency::Host;
use vaab_vm::log::{server_logger, Loggers};
use vaab_vm::machine::{Budget, HttpResponse, Machine, Output, Step, World};
use vaab_vm::value::{Key, Ref, Value};

use crate::{playground, router};

pub async fn serve_file(_source: &str, module: &Module, checked: &Checked) -> Result<(), String> {
    let serve = checked
        .serves
        .values()
        .next()
        .ok_or_else(|| "this file has no `serve on port ...` block".to_string())?;

    let world = vaab_vm::prepare(module, checked, Output::collected());
    let program = Ref::clone(&world.program);
    if program.routes.is_empty() {
        return Err("this server has no compiled routes".to_string());
    }

    let address = SocketAddr::from(([0, 0, 0, 0], serve.port));
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| format!("could not listen on port {}: {}", serve.port, error))?;

    let log = server_logger();
    log.info(&format!("listening on http://127.0.0.1:{}", serve.port));
    let state = Arc::new(ServerState {
        program,
        databases: Databases::shared(),
        stores: Stores::shared(),
        loggers: Loggers::shared(),
        log,
    });

    loop {
        let (stream, peer) = listener.accept().await.map_err(|error| error.to_string())?;
        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let log = state.log.clone();
            let service = service_fn(move |request| handle(state.clone(), request));
            if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                log.write(
                    vaab_vm::LogLevel::Warn,
                    "connection error",
                    &[("peer", &peer.to_string()), ("error", &error.to_string())],
                );
            }
        });
    }
}

struct ServerState {
    program: Ref<Program>,
    databases: Arc<Databases>,
    stores: Arc<Stores>,
    loggers: Arc<Loggers>,
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

    let response = match router::find_route(&state.program.routes, &method, &path) {
        Some(matched) => {
            let route = &state.program.routes[matched.handler];
            let body_text = String::from_utf8_lossy(&body_bytes).into_owned();
            run_route(&state, route, &method, &path, body_text, headers, matched.params)
        }
        None => HttpResponse::json(404, "{\"error\":\"not found\"}".to_string()),
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
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("access-control-allow-origin", "*")
        .header("access-control-allow-headers", "authorization, content-type")
        .header("access-control-allow-methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS")
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
    let mut world = World::with_app_io(
        Ref::clone(&state.program),
        Output::collected(),
        Arc::clone(&state.databases),
        Arc::clone(&state.stores),
        Arc::clone(&state.loggers),
    );
    world.response = None;

    let request = Value::Tuple(Ref::from(vec![
        Value::Text(Ref::from(method)),
        Value::Text(Ref::from(path)),
        Value::Text(Ref::from(body.as_str())),
        Value::map(headers),
    ]));

    let mut machine = Machine::entering(&state.program, route.body, 0);
    let _ = machine.set_argument(0, request);
    for (slot, value) in params.into_iter().enumerate() {
        let argument = if let Ok(number) = value.parse::<i64>() {
            Value::Int(number)
        } else {
            Value::Text(Ref::from(value.as_str()))
        };
        let _ = machine.set_argument((slot + 1) as u32, argument);
    }
    if route.expects_body {
        let slot = (route.path_param_count + 1) as u32;
        let argument = match route.body_layout.and_then(|index| state.program.layouts.get(index as usize).cloned()) {
            Some(layout) => match vaab_vm::json::decode_record(&body, layout) {
                Ok(record) => record,
                Err(_) => {
                    return HttpResponse::json(400, "{\"error\":\"invalid body\"}".to_string());
                }
            },
            None => Value::Text(Ref::from(body.as_str())),
        };
        let _ = machine.set_argument(slot, argument);
    }

    let mut host = Host::scratch(&state.program);
    match machine.resume(&mut world, &mut host, Budget::unlimited()) {
        Step::Finished(value) => {
            if world.response.is_none() {
                if let Value::Failure(held) = &value {
                    if let Ok(body) = vaab_vm::json::encode(held) {
                        world.response = Some(HttpResponse::json(400, body));
                    }
                }
            }
        }
        Step::Yielded | Step::Parked(_) => {}
        Step::Failed(_) => {}
    }

    world.response.unwrap_or(HttpResponse::json(
        500,
        "{\"error\":\"route failed\"}".to_string(),
    ))
}

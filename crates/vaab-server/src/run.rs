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
use vaab_vm::machine::{Budget, HttpResponse, Machine, Output, Step, World};
use vaab_vm::value::{Key, Ref, Value};

use crate::router;

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

    eprintln!("listening on http://127.0.0.1:{}", serve.port);
    let state = Arc::new(ServerState {
        program,
        databases: Databases::shared(),
        stores: Stores::shared(),
    });

    loop {
        let (stream, _) = listener.accept().await.map_err(|error| error.to_string())?;
        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let service = service_fn(move |request| handle(state.clone(), request));
            if let Err(error) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("connection error: {error}");
            }
        });
    }
}

struct ServerState {
    program: Ref<Program>,
    databases: Arc<Databases>,
    stores: Arc<Stores>,
}

async fn handle(
    state: Arc<ServerState>,
    request: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let method = request.method().as_str().to_string();
    if method.eq_ignore_ascii_case("OPTIONS") {
        return Ok(cors_response(StatusCode::NO_CONTENT, Bytes::new()));
    }

    let path = request.uri().path().to_string();
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
        None => HttpResponse {
            status: 404,
            body: "{\"error\":\"not found\"}".to_string(),
        },
    };

    Ok(cors_response(
        StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Bytes::from(response.body),
    ))
}

fn cors_response(status: StatusCode, body: Bytes) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
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
                    return HttpResponse {
                        status: 400,
                        body: "{\"error\":\"invalid body\"}".to_string(),
                    };
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
                        world.response = Some(HttpResponse { status: 400, body });
                    }
                }
            }
        }
        Step::Yielded | Step::Parked(_) => {}
        Step::Failed(_) => {}
    }

    world.response.unwrap_or(HttpResponse {
        status: 500,
        body: "{\"error\":\"route failed\"}".to_string(),
    })
}

use vaab_types::RouteSegment;
use vaab_vm::bytecode::RouteHandler;

pub(crate) struct Match {
    pub handler: usize,
    pub params: Vec<String>,
}

pub(crate) fn find_route(routes: &[RouteHandler], method: &str, path: &str) -> Option<Match> {
    let method = method.to_ascii_lowercase();
    for (index, route) in routes.iter().enumerate() {
        if route.method != method {
            continue;
        }
        if let Some(params) = match_path(&route.path, path) {
            return Some(Match { handler: index, params });
        }
    }
    None
}

fn match_path(pattern: &[RouteSegment], path: &str) -> Option<Vec<String>> {
    let mut params = Vec::new();
    let mut rest = path;
    for segment in pattern {
        match segment {
            RouteSegment::Literal(text) => {
                if !rest.starts_with(text) {
                    return None;
                }
                rest = &rest[text.len()..];
            }
            RouteSegment::Param(_) => {
                let end = rest.find('/').map_or(rest.len(), |index| index);
                if end == 0 {
                    return None;
                }
                params.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            RouteSegment::CatchAll(_) => {
                params.push(rest.trim_start_matches('/').to_string());
                rest = "";
            }
        }
    }
    if rest.is_empty() || rest == "/" {
        Some(params)
    } else {
        None
    }
}

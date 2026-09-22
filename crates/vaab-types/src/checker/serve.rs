//! `serve on port ...` blocks, routes, and `reply` statements.

use vaab_syntax::ast::{
    Block, ExprKind, Name, ReplyKind, ReplyStmt, RouteDecl, RouteSegment as AstRouteSegment,
    ServeDecl, ServeErrorHandler, Stmt,
};
use vaab_syntax::span::Span;

use super::{Checker, Wanted};
use crate::checked::{FrameKind, LocalId, Route, RouteSegment, Serve};
use crate::json::can_json;
use crate::messages;
use crate::types::Type;

const REQUEST: &str = "request";
const METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];

impl Checker {
    pub(super) fn serve(&mut self, statement: &Stmt, serve: &ServeDecl) {
        if self.scopes.len() > 1 {
            self.report(messages::nested_declaration("serve", "serve", serve.span));
            return;
        }

        self.expression(&serve.port, Wanted::Exactly(Type::Int));
        let port = match &serve.port.kind {
            ExprKind::Int(number) if *number > 0 && *number <= u16::MAX as i64 => Some(*number as u16),
            _ => {
                self.report(messages::serve_port_must_be_a_number(serve.port.span));
                None
            }
        };

        let previous_error = self.route_error.clone();
        self.route_error = serve.error_handler.as_ref().map(|handler| self.resolve_type(&handler.error_type));

        let before = serve.before.as_ref().map(|body| self.before_hook(body, statement.id));
        let routes = serve.routes.iter().map(|route| self.check_route(route, statement.id)).collect();
        let error_handler =
            serve.error_handler.as_ref().map(|handler| self.check_error_handler(handler, statement.id));

        self.route_error = previous_error;

        if let Some(port) = port {
            self.checked.serves.insert(
                statement.id,
                Serve { port, before, routes, error_handler },
            );
        }
    }

    pub(super) fn reply(&mut self, reply: &ReplyStmt) {
        if self.route_depth == 0 {
            self.report(messages::reply_outside_route(reply.span));
            return;
        }

        match &reply.kind {
            ReplyKind::With { value, status } => {
                let found = self.expression(value, Wanted::Anything);
                if !can_json(&found, &self.checked) {
                    self.report(messages::cannot_json(value.span));
                }
                if let Some(status) = status {
                    self.expression(status, Wanted::Exactly(Type::Int));
                }
            }
            ReplyKind::File { path, status } => {
                self.expression(path, Wanted::Exactly(Type::Text));
                if let Some(status) = status {
                    self.expression(status, Wanted::Exactly(Type::Int));
                }
            }
            ReplyKind::Text {
                body,
                content_type,
                status,
            } => {
                self.expression(body, Wanted::Exactly(Type::Text));
                self.expression(content_type, Wanted::Exactly(Type::Text));
                if let Some(status) = status {
                    self.expression(status, Wanted::Exactly(Type::Int));
                }
            }
            ReplyKind::Explain(value) => {
                self.expression(value, Wanted::Anything);
            }
        }
    }

    fn before_hook(&mut self, block: &Block, route: vaab_syntax::ast::NodeId) -> crate::checked::FrameId {
        self.enter_route(route);
        self.bind_request();
        let frame = self.frames.last().copied().unwrap_or(crate::checked::Checked::TOP_LEVEL);
        self.block(block, Wanted::Discarded);
        self.leave_route();
        frame
    }

    fn check_route(&mut self, route: &RouteDecl, route_id: vaab_syntax::ast::NodeId) -> Route {
        let method = route.method.text.to_ascii_lowercase();
        if !METHODS.contains(&method.as_str()) {
            self.report(messages::unknown_http_method(&route.method.text, route.method.span));
        }

        let mut path = Vec::new();
        let mut path_params = Vec::new();
        for segment in &route.path {
            match segment {
                AstRouteSegment::Literal(text) => path.push(RouteSegment::Literal(text.clone())),
                AstRouteSegment::Param { name, declared } => {
                    path.push(RouteSegment::Param(name.text.clone()));
                    let param_type = match declared {
                        Some(type_expr) => {
                            let resolved = self.resolve_type(type_expr);
                            if !matches!(resolved, Type::Int | Type::Text) {
                                self.report(messages::route_param_type(&resolved, type_expr.span));
                            }
                            resolved
                        }
                        None => Type::Text,
                    };
                    path_params.push((name.text.clone(), param_type));
                }
                AstRouteSegment::CatchAll { name } => {
                    path.push(RouteSegment::CatchAll(name.text.clone()));
                    path_params.push((name.text.clone(), Type::Text));
                }
            }
        }

        self.enter_route(route_id);
        self.bind_request();
        let frame = self.frames.last().copied().unwrap_or(crate::checked::Checked::TOP_LEVEL);

        for (name, param_type) in &path_params {
            let binding = Name::new(name.clone(), route.span);
            self.declare_local(&binding, param_type.clone(), false);
        }

        let expecting = route.expecting.as_ref().map(|expecting| {
            let declared = self.resolve_type(&expecting.declared);
            let local = self.declare_local(&expecting.binding, declared.clone(), false);
            (declared, local)
        });

        self.block(&route.body, Wanted::Discarded);
        self.leave_route();

        Route { method, path, frame, path_params, expecting }
    }

    fn check_error_handler(
        &mut self,
        handler: &ServeErrorHandler,
        route_id: vaab_syntax::ast::NodeId,
    ) -> (Type, LocalId, crate::checked::FrameId) {
        let error_type = self.resolve_type(&handler.error_type);
        self.enter_route(route_id);
        let local = self.declare_local(&handler.binding, error_type.clone(), false);
        let frame = self.frames.last().copied().unwrap_or(crate::checked::Checked::TOP_LEVEL);
        self.block(&handler.body, Wanted::Discarded);
        self.leave_route();
        (error_type, local, frame)
    }

    fn enter_route(&mut self, route_id: vaab_syntax::ast::NodeId) {
        self.route_depth += 1;
        let frame = self.new_frame(FrameKind::Route(route_id));
        self.frames.push(frame);
        self.push_scope();
    }

    fn leave_route(&mut self) {
        self.pop_scope();
        self.frames.pop();
        self.route_depth -= 1;
    }

    fn bind_request(&mut self) {
        let request_type = Type::Tuple(vec![
            Type::Text,
            Type::Text,
            Type::Text,
            Type::map(Type::Text, Type::Text),
        ]);
        let name = Name::new(REQUEST, Span::default());
        self.declare_local(&name, request_type, false);
    }
}

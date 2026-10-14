//! Statements, blocks and the bodies of declarations.

use vaab_syntax::ast::{
    join_mount_prefix, order_serve_route_refs, prefix_route_segments, Block, Expr, ExprKind,
    MountUse, ReplyKind, RouteDecl, RouteSegment, ServeDecl, Stmt, StmtKind,
};

use crate::bytecode::RouteHandler;
use crate::value::Ref;

use super::Builder;
use vaab_syntax::span::Span;
use vaab_types::{FrameKind, Type};

use super::Compiler;
use crate::bytecode::Op;

impl<'a> Compiler<'a> {
    /// In a fallible function, bare `return x` is success when `x` is the ok type.
    pub(super) fn should_wrap_return_success(&self, value: &Expr) -> bool {
        let Some(frame) = self.current_frame() else {
            return false;
        };
        let Some(info) = self.checked.frame(frame) else {
            return false;
        };
        let FrameKind::Function(id) = info.kind else {
            return false;
        };
        let Some(function) = self.checked.function(id) else {
            return false;
        };
        let Type::Fallible { .. } = &function.signature.returns else {
            return false;
        };
        let Some(found) = self.checked.type_of(value.id) else {
            return false;
        };
        !matches!(found, Type::Fallible { .. })
    }
}

impl<'a> Compiler<'a> {
    pub(super) fn statement(&mut self, statement: &'a Stmt) {
        let span = statement.span;
        match &statement.kind {
            StmtKind::Let(declaration) => {
                self.expression(&declaration.value);
                match self.binding_at(statement.id) {
                    Some(local) => self.declare(local, span),
                    // A `let` the checker did not record a slot for cannot happen
                    // in a program that checked, but the value still has to go.
                    None => self.emit(Op::Pop, span),
                }
            }

            StmtKind::Assign(assignment) => {
                self.expression(&assignment.value);
                match self.assignable(&assignment.target) {
                    Some(super::Assignment::Local(place)) => self.store(place, span),
                    Some(super::Assignment::CastField(field)) => {
                        if let ExprKind::Member { target, .. } = &assignment.target.kind {
                            self.expression(target);
                        }
                        self.emit(Op::SetField(field), span);
                    }
                    None => self.emit(Op::Pop, span),
                }
            }

            StmtKind::Return(value) => {
                match value {
                    Some(value) => {
                        self.expression(value);
                        if self.should_wrap_return_success(value) {
                            self.emit(Op::Success, span);
                        }
                    }
                    None => self.emit(Op::Nothing, span),
                }
                self.emit(Op::Return, span);
            }

            StmtKind::Fail(value) => {
                self.expression(value);
                self.emit(Op::Failure, span);
                self.emit(Op::Return, span);
            }

            StmtKind::ForEach(loop_) => self.for_each(loop_, span),
            StmtKind::While(loop_) => self.while_loop(loop_, span),
            StmtKind::Repeat(loop_) => self.repeat(loop_, span),

            StmtKind::Send(send) => {
                self.expression(&send.value);
                self.expression(&send.channel);
                self.emit(Op::Send, span);
            }
            StmtKind::Close(channel) => {
                self.expression(channel);
                self.emit(Op::Close, span);
            }
            StmtKind::Together(block) => {
                self.emit(Op::BeginTogether, span);
                self.block_discard(block);
                self.emit(Op::EndTogether, span);
            }

            StmtKind::Function(declaration) => {
                // The declaration itself produces nothing: a function becomes a
                // value where it is used, not where it is written.
                if let Some(id) = self.functions_at.get(&statement.id).copied() {
                    self.function(id, declaration);
                }
            }

            StmtKind::Type(declaration) => {
                let Some(id) = self.types_at.get(&statement.id).copied() else { return };
                let Some(declared) = self.checked.declared_type(id) else { return };
                let methods = declared.methods.clone();
                for (method, written) in methods.iter().zip(&declaration.functions) {
                    self.function(*method, written);
                }
            }

            StmtKind::Cast(declaration) => {
                let Some(id) = self.casts_at.get(&statement.id).copied() else { return };
                for written in &declaration.functions {
                    let method = if written.class_method {
                        self.checked.cast_class_method(id, &written.name.text)
                    } else {
                        self.checked.cast_method(id, &written.name.text)
                    };
                    if let Some(method) = method {
                        self.function(method, written);
                    }
                }
            }

            StmtKind::Choice(_) | StmtKind::Ability(_) | StmtKind::Mount(_) => {}

            StmtKind::Serve(serve) => self.serve(statement, serve),
            StmtKind::Reply(reply) => self.reply(reply, span),

            StmtKind::Expr(expression) => {
                self.expression(expression);
                self.emit(Op::Pop, span);
            }

            StmtKind::Need(_) => {}
        }
    }

    /// The place an assignment writes to.
    ///
    /// Only a `changing` name can be assigned to; a field or an item of a list is
    /// a type error, so there is nothing here for either.
    fn assignable(&mut self, target: &Expr) -> Option<super::Assignment> {
        match &target.kind {
            ExprKind::Name(_) => {
                let Some(vaab_types::Resolution::Local(reference)) =
                    self.checked.resolution(target.id)
                else {
                    return None;
                };
                Some(super::Assignment::Local(self.place_of(reference.local)))
            }
            ExprKind::Member { .. } => {
                let Some(vaab_types::Resolution::CastField { field, .. }) =
                    self.checked.resolution(target.id)
                else {
                    return None;
                };
                Some(super::Assignment::CastField(*field as u32))
            }
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Blocks
    // -----------------------------------------------------------------------

    /// A block run for what it does, leaving nothing behind.
    pub(super) fn block_discard(&mut self, block: &'a Block) {
        self.register_declarations(&block.statements);
        for statement in &block.statements {
            self.statement(statement);
        }
    }

    /// A block run for its value, which is its last expression.
    pub(super) fn block_value(&mut self, block: &'a Block) {
        self.register_declarations(&block.statements);

        let last = block.statements.len().saturating_sub(1);
        for (position, statement) in block.statements.iter().enumerate() {
            if position == last {
                if let StmtKind::Expr(expression) = &statement.kind {
                    self.expression(expression);
                    return;
                }
            }
            self.statement(statement);
        }
        // A block that does not end in an expression has no value of its own.
        self.emit(Op::Nothing, block.span);
    }

    // -----------------------------------------------------------------------
    // Loops
    // -----------------------------------------------------------------------

    fn for_each(&mut self, loop_: &'a vaab_syntax::ast::ForEachStmt, span: Span) {
        let walking = self.checked.type_of(loop_.sequence.id).cloned();
        if matches!(walking, Some(Type::Channel(_))) {
            self.for_each_channel(loop_, span);
            return;
        }

        let items = self.temporary();
        let position = self.temporary();
        let item = self.temporary();

        self.expression(&loop_.sequence);
        // A map hands over its keys and values together, so walking one is walking
        // the list of pairs it stands for.
        if matches!(walking, Some(Type::Map { .. })) {
            self.emit(Op::Entries, span);
        }
        self.emit(Op::StoreLocal(items), span);

        self.emit(Op::Int(0), span);
        self.emit(Op::StoreLocal(position), span);

        let start = self.here();
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::LoadLocal(items), span);
        self.emit(Op::Length, span);
        self.emit(Op::Less, span);
        let done = self.jump(Op::JumpIfFalse(0), span);

        self.emit(Op::LoadLocal(items), span);
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::Index, span);
        self.emit(Op::StoreLocal(item), span);

        // A `for each` pattern is one that always matches, so anything it cannot
        // take apart simply skips that item rather than stopping the program.
        let mut skipped = Vec::new();
        self.take_apart(&loop_.pattern, item, &mut skipped);
        self.block_discard(&loop_.body);

        self.land_all(&skipped);
        self.advance(position, span);
        self.emit(Op::Jump(start), span);
        self.land(done);
    }

    fn while_loop(&mut self, loop_: &'a vaab_syntax::ast::WhileStmt, span: Span) {
        let start = self.here();
        self.expression(&loop_.condition);
        let done = self.jump(Op::JumpIfFalse(0), span);
        self.block_discard(&loop_.body);
        self.emit(Op::Jump(start), span);
        self.land(done);
    }

    fn repeat(&mut self, loop_: &'a vaab_syntax::ast::RepeatStmt, span: Span) {
        let times = self.temporary();
        let position = self.temporary();

        self.expression(&loop_.count);
        self.emit(Op::StoreLocal(times), span);
        self.emit(Op::Int(0), span);
        self.emit(Op::StoreLocal(position), span);

        let start = self.here();
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::LoadLocal(times), span);
        self.emit(Op::Less, span);
        let done = self.jump(Op::JumpIfFalse(0), span);

        self.block_discard(&loop_.body);
        self.advance(position, span);
        self.emit(Op::Jump(start), span);
        self.land(done);
    }

    fn for_each_channel(&mut self, loop_: &'a vaab_syntax::ast::ForEachStmt, span: Span) {
        let channel = self.temporary();
        let received = self.temporary();
        let item = self.temporary();

        self.expression(&loop_.sequence);
        self.emit(Op::StoreLocal(channel), span);

        let start = self.here();
        self.emit(Op::LoadLocal(channel), span);
        self.emit(Op::Receive, span);
        self.emit(Op::StoreLocal(received), span);
        self.emit(Op::LoadLocal(received), span);
        self.emit(Op::IsFound, span);
        let done = self.jump(Op::JumpIfFalse(0), span);

        self.emit(Op::LoadLocal(received), span);
        self.emit(Op::Unwrap, span);
        self.emit(Op::StoreLocal(item), span);

        let mut skipped = Vec::new();
        self.take_apart(&loop_.pattern, item, &mut skipped);
        self.block_discard(&loop_.body);

        self.land_all(&skipped);
        self.emit(Op::Jump(start), span);
        self.land(done);
    }

    /// Adds one to a loop counter the compiler owns.
    pub(super) fn advance(&mut self, position: u32, span: Span) {
        self.emit(Op::LoadLocal(position), span);
        self.emit(Op::Int(1), span);
        self.emit(Op::Add, span);
        self.emit(Op::StoreLocal(position), span);
    }

    fn serve(&mut self, statement: &'a Stmt, serve: &'a ServeDecl) {
        let Some(checked) = self.checked.serves.get(&statement.id) else { return };

        if let (Some(before_frame), Some(before_block)) = (checked.before, &serve.before) {
            let body_index = self.builders.len();
            let name = Ref::from("before every request");
            self.builders.push(Builder::new(name, before_frame));
            self.open_body(body_index, before_frame, 1);
            self.block_discard(before_block);
            self.emit(Op::Nothing, before_block.span);
            self.emit(Op::Return, before_block.span);
            self.open.pop();
            self.program.before_route = Some(body_index);
        }

        let mut mounted_routes = Vec::new();
        let mut visited = std::collections::HashSet::new();
        for mount_use in &serve.mounts {
            self.collect_mount_routes(mount_use, "", &mut visited, &mut mounted_routes);
        }

        let all_routes = order_serve_route_refs(&serve.routes, mounted_routes);

        for (route, route_decl) in checked.routes.iter().zip(all_routes) {
            let body_index = self.builders.len();
            let name = Ref::from(format!("route {}", route.method).as_str());
            self.builders.push(Builder::new(name, route.frame));

            let parameters = 1
                + route.path_params.len()
                + usize::from(route.expecting.is_some());
            self.open_body(body_index, route.frame, parameters);
            self.block_discard(&route_decl.body);
            self.emit(Op::Nothing, route_decl.span);
            self.emit(Op::Return, route_decl.span);
            self.open.pop();

            self.program.routes.push(RouteHandler {
                method: route.method.clone(),
                path: route.path.clone(),
                body: body_index,
                frame: route.frame,
                path_param_count: route.path_params.len(),
                path_param_types: route
                    .path_params
                    .iter()
                    .map(|(_, declared)| declared.clone())
                    .collect(),
                expects_body: route.expecting.is_some(),
                body_layout: route.expecting.as_ref().and_then(|(declared, _)| {
                    match declared {
                        vaab_types::Type::Named(name) => {
                            self.checked.declared_types.iter().enumerate().find_map(
                                |(index, typed)| {
                                    (typed.name == *name).then_some(index as u32)
                                },
                            )
                        }
                        _ => None,
                    }
                }),
                body_is_map: route
                    .expecting
                    .as_ref()
                    .is_some_and(|(declared, _)| matches!(declared, vaab_types::Type::Map { .. })),
            });
        }

        if let (Some((_, _, error_frame)), Some(handler_decl)) =
            (&checked.error_handler, &serve.error_handler)
        {
            let body_index = self.builders.len();
            let name = Ref::from("when anything fails");
            self.builders.push(Builder::new(name, *error_frame));
            self.open_body(body_index, *error_frame, 1);
            self.block_discard(&handler_decl.body);
            self.emit(Op::Nothing, handler_decl.body.span);
            self.emit(Op::Return, handler_decl.body.span);
            self.open.pop();
            self.program.error_handler = Some(body_index);
        }
    }

    fn collect_mount_routes(
        &self,
        mount_use: &MountUse,
        inherited_prefix: &str,
        visited: &mut std::collections::HashSet<(String, String)>,
        out: &mut Vec<(Vec<RouteSegment>, &'a RouteDecl)>,
    ) {
        let Some(mount) = self.mounts.get(&mount_use.name.text).copied() else { return };
        let own_base = mount_use
            .path
            .as_deref()
            .or(mount.path.as_deref())
            .unwrap_or("");
        let effective_prefix = join_mount_prefix(inherited_prefix, own_base);
        if !visited.insert((mount.name.text.clone(), effective_prefix.clone())) {
            return;
        }
        let child_inherited = if mount.path.is_none() {
            effective_prefix.as_str()
        } else {
            inherited_prefix
        };
        for nested in &mount.mounts {
            self.collect_mount_routes(nested, child_inherited, visited, out);
        }
        for route in &mount.routes {
            out.push((prefix_route_segments(&effective_prefix, &route.path), route));
        }
    }

    fn reply(&mut self, reply: &'a vaab_syntax::ast::ReplyStmt, span: Span) {
        match &reply.kind {
            ReplyKind::With { value, status } => {
                self.expression(value);
                if let Some(status) = status {
                    self.expression(status);
                    self.emit(Op::ReplyWith(true), span);
                } else {
                    self.emit(Op::ReplyWith(false), span);
                }
            }
            ReplyKind::File { path, status } => {
                self.expression(path);
                if let Some(status) = status {
                    self.expression(status);
                    self.emit(Op::ReplyFile(true), span);
                } else {
                    self.emit(Op::ReplyFile(false), span);
                }
            }
            ReplyKind::Text {
                body,
                content_type,
                status,
            } => {
                self.expression(body);
                self.expression(content_type);
                if let Some(status) = status {
                    self.expression(status);
                    self.emit(Op::ReplyText(true), span);
                } else {
                    self.emit(Op::ReplyText(false), span);
                }
            }
            ReplyKind::Explain(value) => {
                self.expression(value);
                self.emit(Op::ReplyExplain, span);
            }
        }
    }
}

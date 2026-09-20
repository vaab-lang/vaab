//! Statements, blocks and the bodies of declarations.

use vaab_syntax::ast::{Block, Expr, ExprKind, ReplyKind, ServeDecl, Stmt, StmtKind};

use crate::bytecode::RouteHandler;
use crate::value::Ref;

use super::Builder;
use vaab_syntax::span::Span;
use vaab_types::Type;

use super::Compiler;
use crate::bytecode::Op;

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
                    Some(place) => self.store(place, span),
                    None => self.emit(Op::Pop, span),
                }
            }

            StmtKind::Return(value) => {
                match value {
                    Some(value) => self.expression(value),
                    None => self.emit(Op::Nothing, span),
                }
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

            StmtKind::Choice(_) | StmtKind::Ability(_) => {}

            StmtKind::Serve(serve) => self.serve(statement, serve),
            StmtKind::Reply(reply) => self.reply(reply, span),

            StmtKind::Expr(expression) => {
                self.expression(expression);
                self.emit(Op::Pop, span);
            }
        }
    }

    /// The place an assignment writes to.
    ///
    /// Only a `changing` name can be assigned to; a field or an item of a list is
    /// a type error, so there is nothing here for either.
    fn assignable(&mut self, target: &Expr) -> Option<super::Place> {
        let ExprKind::Name(_) = &target.kind else { return None };
        let Some(vaab_types::Resolution::Local(reference)) = self.checked.resolution(target.id)
        else {
            return None;
        };
        Some(self.place_of(reference.local))
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

        for (route, route_decl) in checked.routes.iter().zip(&serve.routes) {
            let body_index = self.builders.len();
            let name = Ref::from(format!("route {}", route.method).as_str());
            self.builders.push(Builder::new(name, route.frame));

            let parameters = 1
                + route.path_params.len()
                + usize::from(route.expecting.is_some());
            self.open_body(body_index, route.frame, parameters);
            self.block_discard(&route_decl.body);
            self.emit(Op::Return, route_decl.span);
            self.open.pop();

            self.program.routes.push(RouteHandler {
                method: route.method.clone(),
                path: route.path.clone(),
                body: body_index,
                frame: route.frame,
                path_param_count: route.path_params.len(),
                expects_body: route.expecting.is_some(),
            });
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
            ReplyKind::Explain(value) => {
                self.expression(value);
                self.emit(Op::ReplyExplain, span);
            }
        }
    }
}

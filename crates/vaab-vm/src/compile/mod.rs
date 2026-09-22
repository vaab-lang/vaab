//! Turning a checked file into bytecode.
//!
//! Almost nothing is worked out here. [`Checked`] already says what every name
//! refers to, which slot each local lives in, and where each argument of a call
//! comes from once names and defaults have been sorted out — so this walks the
//! same tree the checker walked and writes down instructions.
//!
//! The two things the compiler does decide are about *where values live*:
//!
//! * The file's own top-level values are globals, held by the world rather than
//!   by a frame, so that they outlive the statement that made them. That is what
//!   lets the REPL add a line to a session without running the session again, and
//!   what will let phase 4's tasks see the same file-level values.
//! * A local that a closure declared inside it reads is put in a cell, because
//!   assigning to it from the closure has to change the very slot the declaring
//!   frame reads. Everything else lives in the frame's window of the value stack.
//!   The checker already says which locals those are: it records how many frames
//!   out every use reaches.

mod expr;
mod pattern;
mod stmt;

use std::collections::{HashMap, HashSet};

use vaab_syntax::ast::{self, FunctionBody, FunctionDecl, Module, NodeId, Stmt, StmtKind};
use vaab_syntax::span::Span;
use vaab_types::{CastId, Checked, FrameId, FunctionId, LocalId, Owner, Resolution, TypeId};

use crate::bytecode::{Body, Capture, Op, Program};
use crate::value::{Closure, RecordLayout, Ref, Value, VariantLayout};

/// Compiles a checked module.
///
/// This cannot fail. Everything a program can get wrong has already been reported
/// by the parser or the checker; what is merely not built yet — channels, tasks,
/// reading a file — compiles to an instruction that reports its phase if the
/// program ever reaches it.
pub fn compile(module: &Module, checked: &Checked) -> Program {
    let mut compiler = Compiler::new(checked);
    compiler.module(module);
    compiler.finish()
}

/// Where an assignment writes.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Assignment {
    Local(Place),
    CastField(u32),
}

/// Where a local lives, once the compiler has worked out which frame owns it.
#[derive(Clone, Copy, Debug)]
enum Place {
    /// A top-level value of the file.
    Global(u32),
    /// A slot of the current frame.
    Local(u32),
    /// A slot of the current frame holding a cell, because something inside can
    /// see it.
    Cell(u32),
    /// A cell this body captured from the frame that made it.
    Upvalue(u32),
}

/// One body while it is being filled in.
///
/// These are kept apart from [`Program::bodies`] because a body nested inside
/// another can add a capture to the one around it long after that one started.
struct Builder {
    name: Ref<str>,
    frame: FrameId,
    code: Vec<Op>,
    spans: Vec<Span>,
    slots: usize,
    parameters: usize,
    captures: Vec<Capture>,
    /// Which local each capture stands for, so a second use finds the first.
    captured: Vec<LocalId>,
}

pub(crate) struct Compiler<'a> {
    checked: &'a Checked,
    program: Program,
    builders: Vec<Builder>,
    /// The bodies being filled, outermost first. This is the lexical nesting, so
    /// it is also the chain a capture is resolved along.
    open: Vec<usize>,
    /// The body compiled for each declared function.
    body_of: HashMap<FunctionId, usize>,
    /// The flattened number of each `(choice, variant)` pair, which is what a
    /// pattern tests against.
    variant_of: HashMap<(usize, usize), u32>,
    /// Locals that live in a cell rather than in a stack slot.
    boxed: HashSet<LocalId>,
    /// Function declarations by the statement that wrote them, so a call can find
    /// the expression behind a default argument.
    functions_at: HashMap<NodeId, FunctionId>,
    /// The same for types, whose fields may also have defaults.
    types_at: HashMap<NodeId, TypeId>,
    casts_at: HashMap<NodeId, CastId>,
    /// Declaration statements in scope, registered on the way into each block the
    /// way the checker hoists them.
    declarations: HashMap<NodeId, &'a Stmt>,
}

impl<'a> Compiler<'a> {
    fn new(checked: &'a Checked) -> Compiler<'a> {
        let mut compiler = Compiler {
            checked,
            program: Program::with_top_level(Ref::from(TOP_LEVEL_NAME)),
            builders: Vec::new(),
            open: Vec::new(),
            body_of: HashMap::new(),
            variant_of: HashMap::new(),
            boxed: HashSet::new(),
            functions_at: HashMap::new(),
            types_at: HashMap::new(),
            casts_at: HashMap::new(),
            declarations: HashMap::new(),
        };
        compiler.lay_out();
        compiler
    }

    /// Everything that can be worked out before a single instruction is written:
    /// which locals need cells, what each declared type and choice looks like, and
    /// one body per declared function.
    fn lay_out(&mut self) {
        // A use that reaches out of its own frame is what makes a local shared.
        // Top-level values are already shared by being global, so they stay put.
        for resolution in self.checked.resolutions.values() {
            let Resolution::Local(reference) = resolution else { continue };
            if reference.hops == 0 {
                continue;
            }
            let Some(local) = self.checked.local(reference.local) else { continue };
            if local.frame != Checked::TOP_LEVEL {
                self.boxed.insert(reference.local);
            }
        }

        for (number, declared) in self.checked.declared_types.iter().enumerate() {
            let mut tables = vec![Vec::new(); self.checked.abilities.len()];
            for ability in &declared.abilities {
                let Some(required) = self.checked.ability(*ability) else { continue };
                let slots: Vec<u32> = required
                    .functions
                    .iter()
                    .map(|wanted| {
                        self.checked
                            .method(TypeId(number as u32), &wanted.name)
                            .map(|found| found.index() as u32 + 1)
                            .unwrap_or_default()
                    })
                    .collect();
                if let Some(row) = tables.get_mut(ability.index()) {
                    *row = slots;
                }
            }
            self.program.layouts.push(Ref::new(RecordLayout {
                name: declared.name.clone(),
                fields: declared.fields.iter().map(|field| field.name.clone()).collect(),
                mutable: false,
                tables,
            }));
        }

        for (number, declared) in self.checked.declared_casts.iter().enumerate() {
            let mut tables = vec![Vec::new(); self.checked.abilities.len()];
            for ability in &declared.abilities {
                let Some(required) = self.checked.ability(*ability) else { continue };
                let slots: Vec<u32> = required
                    .functions
                    .iter()
                    .map(|wanted| {
                        self.checked
                            .cast_method(CastId(number as u32), &wanted.name)
                            .map(|found| found.index() as u32 + 1)
                            .unwrap_or_default()
                    })
                    .collect();
                if let Some(row) = tables.get_mut(ability.index()) {
                    *row = slots;
                }
            }
            self.program.layouts.push(Ref::new(RecordLayout {
                name: declared.name.clone(),
                fields: declared.fields.iter().map(|field| field.name.clone()).collect(),
                mutable: true,
                tables,
            }));
        }

        for (choice, declared) in self.checked.choices.iter().enumerate() {
            for (variant, shape) in declared.variants.iter().enumerate() {
                let index = self.program.variants.len() as u32;
                self.variant_of.insert((choice, variant), index);
                self.program.variants.push(Ref::new(VariantLayout {
                    index,
                    choice: declared.name.clone(),
                    name: shape.name.clone(),
                    fields: shape.fields.iter().map(|field| field.name.clone()).collect(),
                }));
            }
        }

        // Body 0 is the file itself, so a function's body is its place in the
        // checker's table, one further along.
        self.builders.push(Builder::new(Ref::from(TOP_LEVEL_NAME), Checked::TOP_LEVEL));
        for (number, function) in self.checked.functions.iter().enumerate() {
            let id = FunctionId(number as u32);
            let owner = function.owner.map(|owner| match owner {
                Owner::Type(id) => self
                    .checked
                    .declared_type(id)
                    .map(|owner| format!("{}.", owner.name))
                    .unwrap_or_default(),
                Owner::Cast(id) => self
                    .checked
                    .declared_cast(id)
                    .map(|owner| format!("{}.", owner.name))
                    .unwrap_or_default(),
            }).unwrap_or_default();
            let name: Ref<str> = Ref::from(format!("`{owner}{}`", function.name).as_str());
            self.body_of.insert(id, self.builders.len());
            self.builders.push(Builder::new(name, function.frame));
        }

        for (number, function) in self.checked.functions.iter().enumerate() {
            if function.owner.is_none() {
                self.functions_at.insert(function.declaration, FunctionId(number as u32));
            }
        }
        for (number, declared) in self.checked.declared_types.iter().enumerate() {
            self.types_at.insert(declared.declaration, TypeId(number as u32));
        }
        for (number, declared) in self.checked.declared_casts.iter().enumerate() {
            self.casts_at.insert(declared.declaration, CastId(number as u32));
        }

        self.program.globals = self
            .checked
            .frame(Checked::TOP_LEVEL)
            .map(|frame| frame.slots)
            .unwrap_or_default();
    }

    fn finish(mut self) -> Program {
        self.program.bodies = self
            .builders
            .into_iter()
            .map(|builder| Body {
                name: builder.name,
                code: builder.code,
                spans: builder.spans,
                slots: builder.slots,
                parameters: builder.parameters,
                captures: builder.captures,
            })
            .collect();
        self.program
    }

    // -----------------------------------------------------------------------
    // The file itself
    // -----------------------------------------------------------------------

    fn module(&mut self, module: &'a Module) {
        self.open.push(Program::TOP_LEVEL);
        self.register_declarations(&module.statements);

        let last = module.statements.len().saturating_sub(1);
        let mut ends_in_a_value = false;

        for (position, statement) in module.statements.iter().enumerate() {
            let start = self.here();
            self.program.statements.push(start);

            // "The last expression in a block is its value" holds for a file too,
            // which is what gives the REPL something to show after a line.
            if position == last {
                if let StmtKind::Expr(expression) = &statement.kind {
                    self.expression(expression);
                    ends_in_a_value = true;
                    continue;
                }
            }
            self.statement(statement);
        }

        let end = self.here();
        self.program.statements.push(end);

        if !ends_in_a_value {
            self.emit(Op::Nothing, module.span);
        }
        self.emit(Op::Return, module.span);
        self.open.pop();
    }

    /// Notes the declarations of one block, the way the checker hoists them, so
    /// that a call written above a declaration still finds it.
    fn register_declarations(&mut self, statements: &'a [Stmt]) {
        for statement in statements {
            if matches!(
                statement.kind,
                StmtKind::Function(_) | StmtKind::Type(_) | StmtKind::Cast(_)
            ) {
                self.declarations.insert(statement.id, statement);
            }
        }
    }

    pub(crate) fn cast_layout(&self, id: CastId) -> u32 {
        self.checked.declared_types.len() as u32 + id.0
    }

    // -----------------------------------------------------------------------
    // Bodies
    // -----------------------------------------------------------------------

    /// Compiles one declared function, method or validated `to new`.
    fn function(&mut self, id: FunctionId, declaration: &'a FunctionDecl) {
        let Some(body) = self.body_of.get(&id).copied() else { return };
        let Some(function) = self.checked.function(id) else { return };
        let frame = function.frame;
        let parameters = function.parameters.clone();

        self.open_body(body, frame, parameters.len());
        for local in &parameters {
            self.box_if_shared(*local, declaration.span);
        }

        match &declaration.body {
            Some(FunctionBody::Expr(expression)) => self.expression(expression),
            Some(FunctionBody::Block(block)) => self.block_value(block),
            // An ability's required signature has no body to compile.
            None => self.emit(Op::Nothing, declaration.span),
        }
        self.emit(Op::Return, declaration.span);
        self.open.pop();
    }

    /// Opens a body whose builder already exists, and gives it the slots the
    /// checker counted plus room for whatever temporaries the compiler needs.
    fn open_body(&mut self, body: usize, frame: FrameId, parameters: usize) {
        let slots = self.checked.frame(frame).map(|frame| frame.slots).unwrap_or_default();
        if let Some(builder) = self.builders.get_mut(body) {
            builder.frame = frame;
            builder.slots = slots;
            builder.parameters = parameters;
        }
        self.open.push(body);
    }

    /// A parameter or pattern binding that something inside can see has to be in a
    /// cell before anything captures it.
    fn box_if_shared(&mut self, local: LocalId, span: Span) {
        if !self.boxed.contains(&local) {
            return;
        }
        let Some(slot) = self.checked.local(local).map(|local| local.slot as u32) else { return };
        self.emit(Op::Box(slot), span);
    }

    // -----------------------------------------------------------------------
    // Where values live
    // -----------------------------------------------------------------------

    fn current_frame(&self) -> Option<FrameId> {
        let body = self.open.last().copied()?;
        self.builders.get(body).map(|builder| builder.frame)
    }

    fn place_of(&mut self, local: LocalId) -> Place {
        let Some(held) = self.checked.local(local) else { return Place::Local(0) };
        let (frame, slot) = (held.frame, held.slot as u32);

        if frame == Checked::TOP_LEVEL {
            return Place::Global(slot);
        }
        if Some(frame) == self.current_frame() {
            return if self.boxed.contains(&local) { Place::Cell(slot) } else { Place::Local(slot) };
        }
        Place::Upvalue(self.capture(self.open.len().saturating_sub(1), local))
    }

    /// Finds, or arranges, the capture through which the body at `depth` reaches a
    /// local declared further out.
    ///
    /// A capture always holds a cell, and the chain is built one body at a time:
    /// if the body immediately outside declared the local it is taken from a slot,
    /// and otherwise that body has to capture it first.
    fn capture(&mut self, depth: usize, local: LocalId) -> u32 {
        let Some(body) = self.open.get(depth).copied() else { return 0 };
        if let Some(builder) = self.builders.get(body) {
            if let Some(found) = builder.captured.iter().position(|held| *held == local) {
                return found as u32;
            }
        }

        let Some(slot) = self.checked.local(local).map(|held| held.slot as u32) else { return 0 };
        let declared_in = self.checked.local(local).map(|held| held.frame);
        let enclosing = depth.checked_sub(1).and_then(|outer| self.open.get(outer).copied());
        let takes_it_from_a_slot = enclosing
            .and_then(|outer| self.builders.get(outer))
            .is_some_and(|outer| Some(outer.frame) == declared_in);

        let capture = if enclosing.is_none() || takes_it_from_a_slot {
            Capture::Local(slot)
        } else {
            Capture::Upvalue(self.capture(depth - 1, local))
        };

        let Some(builder) = self.builders.get_mut(body) else { return 0 };
        builder.captures.push(capture);
        builder.captured.push(local);
        (builder.captures.len() - 1) as u32
    }

    fn load(&mut self, place: Place, span: Span) {
        let op = match place {
            Place::Global(slot) => Op::LoadGlobal(slot),
            Place::Local(slot) => Op::LoadLocal(slot),
            Place::Cell(slot) => Op::LoadCell(slot),
            Place::Upvalue(slot) => Op::LoadUpvalue(slot),
        };
        self.emit(op, span);
    }

    /// Stores the top of the stack into a place that already exists.
    fn store(&mut self, place: Place, span: Span) {
        let op = match place {
            Place::Global(slot) => Op::StoreGlobal(slot),
            Place::Local(slot) => Op::StoreLocal(slot),
            Place::Cell(slot) => Op::StoreCell(slot),
            Place::Upvalue(slot) => Op::StoreUpvalue(slot),
        };
        self.emit(op, span);
    }

    /// Stores the top of the stack into a place being declared for the first time.
    ///
    /// A `let` inside a loop declares a new value each time round, so a shared one
    /// gets a new cell each time too: closures made on different turns of the loop
    /// then have a value each, which is what the program says.
    fn declare(&mut self, local: LocalId, span: Span) {
        let place = self.place_of(local);
        match place {
            Place::Cell(slot) => {
                self.emit(Op::StoreLocal(slot), span);
                self.emit(Op::Box(slot), span);
            }
            other => self.store(other, span),
        }
    }

    /// The local a `let`, a `for each` or a pattern introduces.
    fn binding_at(&self, node: NodeId) -> Option<LocalId> {
        self.checked.bindings.get(&node).copied()
    }

    // -----------------------------------------------------------------------
    // Writing instructions
    // -----------------------------------------------------------------------

    fn emit(&mut self, op: Op, span: Span) {
        let Some(body) = self.open.last().copied() else { return };
        let Some(builder) = self.builders.get_mut(body) else { return };
        builder.code.push(op);
        builder.spans.push(span);
    }

    /// The position the next instruction will have.
    fn here(&self) -> u32 {
        self.open
            .last()
            .and_then(|body| self.builders.get(*body))
            .map(|builder| builder.code.len() as u32)
            .unwrap_or_default()
    }

    /// Writes a jump whose target is not known yet, and gives back where to patch.
    fn jump(&mut self, op: Op, span: Span) -> u32 {
        let at = self.here();
        self.emit(op, span);
        at
    }

    /// Points a jump written earlier at the next instruction.
    fn land(&mut self, at: u32) {
        let target = self.here();
        let Some(body) = self.open.last().copied() else { return };
        let Some(builder) = self.builders.get_mut(body) else { return };
        let Some(op) = builder.code.get_mut(at as usize) else { return };
        match op {
            Op::Jump(to) | Op::JumpIfFalse(to) | Op::JumpIfTrue(to) => *to = target,
            // Only a jump is ever patched, so reaching here would mean the
            // compiler had lost track of its own output.
            _ => {}
        }
    }

    fn land_all(&mut self, targets: &[u32]) {
        for target in targets {
            self.land(*target);
        }
    }

    /// A slot of the current frame for the compiler's own use: a loop counter, the
    /// subject of a `match`, the list `.map` is filling in.
    fn temporary(&mut self) -> u32 {
        let Some(body) = self.open.last().copied() else { return 0 };
        let Some(builder) = self.builders.get_mut(body) else { return 0 };
        let slot = builder.slots;
        builder.slots += 1;
        slot as u32
    }

    fn constant(&mut self, value: Value) -> u32 {
        self.program.constants.push(value);
        (self.program.constants.len() - 1) as u32
    }

    fn push_constant(&mut self, value: Value, span: Span) {
        let index = self.constant(value);
        self.emit(Op::Constant(index), span);
    }

    // -----------------------------------------------------------------------
    // Functions as values
    // -----------------------------------------------------------------------

    /// Whether a body made in `frame` could have anything to capture.
    ///
    /// Only a frame nested inside another frame's locals can: a body written at
    /// the top level reaches its surroundings through globals, which are there
    /// whoever is running.
    fn may_capture(&self, frame: FrameId) -> bool {
        self.checked.frame(frame).and_then(|frame| frame.parent) != Some(Checked::TOP_LEVEL)
    }

    /// Pushes a function value for a body.
    ///
    /// A body with nothing to capture is one value for the whole run, built here
    /// and kept in the constant pool, so calling a top-level function allocates
    /// nothing at all.
    fn push_function(&mut self, body: usize, frame: FrameId, span: Span) {
        if self.may_capture(frame) {
            self.emit(Op::MakeFunction(body as u32), span);
            return;
        }
        let name = self
            .builders
            .get(body)
            .map(|builder| Ref::clone(&builder.name))
            .unwrap_or_else(|| Ref::from("a function"));
        let value = Value::Function(Ref::new(Closure {
            body: body as u32,
            name,
            captures: Vec::new(),
        }));
        self.push_constant(value, span);
    }

    // -----------------------------------------------------------------------
    // Finding declarations again
    // -----------------------------------------------------------------------

    /// The declaration a function was written as, so that a call can compile the
    /// expression behind a default argument.
    fn declaration_of(&self, id: FunctionId) -> Option<&'a FunctionDecl> {
        let function = self.checked.function(id)?;
        let statement = self.declarations.get(&function.declaration)?;
        match &statement.kind {
            StmtKind::Function(declaration) => Some(declaration),
            // A method has no statement of its own, so its declaration is the type
            // that holds it and the name picks it out.
            StmtKind::Type(declaration) => {
                declaration.functions.iter().find(|written| written.name.text == function.name)
            }
            StmtKind::Cast(declaration) => declaration
                .functions
                .iter()
                .find(|written| written.name.text == function.name),
            _ => None,
        }
    }

    /// The fields of a declared type as they were written, for their defaults.
    fn fields_of(&self, id: TypeId) -> Option<&'a [ast::Field]> {
        let declared = self.checked.declared_type(id)?;
        let statement = self.declarations.get(&declared.declaration)?;
        match &statement.kind {
            StmtKind::Type(declaration) => Some(&declaration.fields),
            _ => None,
        }
    }

    fn cast_fields_of(&self, id: CastId) -> Option<&'a [ast::Field]> {
        let declared = self.checked.declared_cast(id)?;
        let statement = self.declarations.get(&declared.declaration)?;
        match &statement.kind {
            StmtKind::Cast(declaration) => Some(&declaration.fields),
            _ => None,
        }
    }

    pub(crate) fn file_error_not_found(&self) -> u32 {
        self.variant_layout("FileError", "NotFound")
    }

    pub(crate) fn env_error_missing(&self) -> u32 {
        self.variant_layout("EnvError", "Missing")
    }

    pub(crate) fn db_error_failed(&self) -> u32 {
        self.variant_layout("DbError", "Failed")
    }

    pub(crate) fn store_error_failed(&self) -> u32 {
        self.variant_layout("StoreError", "Failed")
    }

    /// Terminal query methods fail with the error type baked into `Query`.
    pub(crate) fn query_error_failed(&self, target: Option<&vaab_syntax::ast::Expr>) -> u32 {
        let error = target
            .and_then(|expr| self.checked.type_of(expr.id))
            .and_then(|declared| match declared {
                vaab_types::Type::Query { error } => Some(error.as_ref()),
                _ => None,
            });
        match error {
            Some(vaab_types::Type::Named(name)) if name == "StoreError" => {
                self.store_error_failed()
            }
            _ => self.db_error_failed(),
        }
    }

    pub(crate) fn http_error_failed(&self) -> u32 {
        self.variant_layout("HttpError", "Failed")
    }

    pub(crate) fn auth_error_unauthorized(&self) -> u32 {
        self.variant_layout("AuthError", "Unauthorized")
    }

    pub(crate) fn user_layout(&self) -> u32 {
        for (index, declared) in self.checked.declared_types.iter().enumerate() {
            if declared.name == "User" {
                return index as u32;
            }
        }
        0
    }

    fn variant_layout(&self, choice: &str, variant: &str) -> u32 {
        for (choice_index, declared) in self.checked.choices.iter().enumerate() {
            if declared.name != choice {
                continue;
            }
            for (variant_index, shape) in declared.variants.iter().enumerate() {
                if shape.name == variant {
                    return self
                        .variant_of
                        .get(&(choice_index, variant_index))
                        .copied()
                        .unwrap_or(0);
                }
            }
        }
        0
    }
}

impl Builder {
    fn new(name: Ref<str>, frame: FrameId) -> Builder {
        Builder {
            name,
            frame,
            code: Vec::new(),
            spans: Vec::new(),
            slots: 0,
            parameters: 0,
            captures: Vec::new(),
            captured: Vec::new(),
        }
    }
}

/// What a trace calls the file's own statements.
const TOP_LEVEL_NAME: &str = "the top level";

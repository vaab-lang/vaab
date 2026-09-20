//! The checker itself.
//!
//! Checking runs in two halves. First every `type`, `choice` and `ability` in the
//! file is collected, so that a signature may mention a type declared further down;
//! then the statements are walked in order, and each function's body is checked
//! where it is written. That order is what makes "a function can use the values
//! declared above it" true without any risk of using one before it exists.
//!
//! Checking is *bidirectional*. [`Checker::expression`] is given what the
//! surrounding code wants, as a [`Wanted`], and uses it where knowing helps: to
//! give a closure's parameters their types, to tell an empty list what it holds, and
//! to decide which `maybe` a bare `nothing` is. Where it does not help, the type is
//! worked out from the expression alone and then compared.
//!
//! The submodules hold the parts:
//!
//! * [`stmt`] — statements, blocks and declarations
//! * [`expr`] — expressions
//! * [`call`] — matching arguments to parameters
//! * [`member`] — `.field`, `.method`, `Type.new`
//! * [`pattern`] — patterns
//! * [`exhaustive`] — whether a `match` covers everything
//! * [`pure`] — what a `pure` function may not do
//! * [`sendable`] — what may cross between tasks
//! * [`walk`] — one walk over a subtree, which [`sendable`] and [`pure`] ask their
//!   questions with

mod call;
mod exhaustive;
mod expr;
mod member;
mod pattern;
mod pure;
mod sendable;
mod serve;
mod stmt;
mod walk;

use std::collections::{HashMap, HashSet};

use vaab_syntax::ast::{
    AbilityDecl, ChoiceDecl, Expr, FunctionDecl, Module, Name, NodeId, Stmt, StmtKind, TypeDecl,
    TypeExpr, TypeKind,
};
use vaab_syntax::diagnostic::Diagnostic;
use vaab_syntax::span::Span;

use crate::checked::{
    Ability, AbilityId, Checked, Choice, ChoiceId, Constructor, DeclaredType, Field, Frame,
    FrameId, FrameKind, Function, FunctionId, Local, LocalId, LocalRef, Required, Resolution,
    TypeId, Variant,
};
use crate::json::can_json;
use crate::messages;
use crate::prelude;
use crate::types::{Parameter, Signature, Type};
use crate::unify::Variables;

/// What the surrounding code wants an expression to be.
#[derive(Clone, Debug)]
pub(crate) enum Wanted {
    /// Nothing is known, so work the type out from the expression alone.
    Anything,
    /// The value is thrown away. Nothing is known *and* nothing needs to agree, so
    /// the branches of an `if` used as a statement may differ.
    Discarded,
    /// This type exactly.
    Exactly(Type),
}

impl Wanted {
    fn of(declared: Option<Type>) -> Wanted {
        match declared {
            Some(declared) => Wanted::Exactly(declared),
            None => Wanted::Anything,
        }
    }

    /// The type wanted, when one is.
    fn as_type(&self) -> Option<&Type> {
        match self {
            Wanted::Exactly(declared) => Some(declared),
            _ => None,
        }
    }

    fn is_discarded(&self) -> bool {
        matches!(self, Wanted::Discarded)
    }
}

/// A declaration reachable by name from anywhere in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Global {
    Type(TypeId),
    Choice(ChoiceId),
    Ability(AbilityId),
}

/// One level of names. Blocks, function bodies and match arms each open one.
struct Scope {
    frame: FrameId,
    locals: Vec<(String, LocalId)>,
    functions: Vec<(String, FunctionId)>,
}

/// A value that has to take its type from somewhere else: an empty list, an empty
/// map, or a bare `nothing`.
///
/// Each carries a variable that something later may pin down — the other arm of an
/// `if`, the annotation on the `let` it ends up in. Whatever is still open once the
/// file has been walked is a type Vaab genuinely could not work out, and is reported
/// then rather than on the spot.
struct Undecided {
    hole: Type,
    span: Span,
    kind: Undecidable,
}

enum Undecidable {
    /// `[]` and `{}`, with the annotation to suggest.
    Empty { what: &'static str, example: &'static str },
    /// A bare `nothing`.
    Nothing,
}

/// The function whose body is being checked, which is what `return` and `try` need.
struct Enclosing {
    name: String,
    returns: Type,
    /// The declared result, for pointing at when a `return` or a `try` disagrees.
    signature: Span,
}

pub(crate) struct Checker {
    checked: Checked,
    diagnostics: Vec<Diagnostic>,
    variables: Variables,
    scopes: Vec<Scope>,
    /// The frames being filled, innermost last.
    frames: Vec<FrameId>,
    /// Functions whose bodies are being checked, innermost last.
    enclosing: Vec<Enclosing>,
    /// The type whose body is being checked, which is what `self` and `raw` need.
    inside: Option<TypeId>,
    /// Types still waiting to be pinned down. See [`Undecided`].
    undecided: Vec<Undecided>,
    globals: HashMap<String, Global>,
    /// Where each global was declared, so a repeat can point at the first one.
    global_spans: HashMap<String, Span>,
    builtin_functions: Vec<prelude::Function>,
    builtin_methods: Vec<prelude::Method>,
    /// What each closure uses from outside itself. A function value is not sendable
    /// on its own, so this is what lets a closure be judged by its captures when a
    /// task is given the local holding it. See [`sendable`].
    closure_captures: sendable::ClosureCaptures,
    /// The closure a fixed local was bound straight to, when it was.
    closure_of_local: sendable::ClosuresOfLocals,
    /// Which captures have already been refused, so that a task nested inside
    /// another does not have the same value explained to it twice.
    reported_captures: HashSet<(LocalId, Span)>,
    route_depth: u32,
}

/// Checks a module.
///
/// On success every expression in the module has a type in [`Checked::types`]. On
/// failure the diagnostics are in source order, as they are after parsing.
pub fn check(module: &Module) -> Result<Checked, Vec<Diagnostic>> {
    let mut checker = Checker::new();
    checker.check_module(module);
    checker.finish()
}

impl Checker {
    fn new() -> Self {
        let mut checked = Checked::default();
        // Frame 0 is the file itself, which holds its top-level values.
        checked.frames.push(Frame { kind: FrameKind::Module, slots: 0, parent: None });

        Checker {
            checked,
            diagnostics: Vec::new(),
            variables: Variables::default(),
            scopes: vec![Scope {
                frame: Checked::TOP_LEVEL,
                locals: Vec::new(),
                functions: Vec::new(),
            }],
            frames: vec![Checked::TOP_LEVEL],
            enclosing: Vec::new(),
            inside: None,
            undecided: Vec::new(),
            globals: HashMap::new(),
            global_spans: HashMap::new(),
            builtin_functions: prelude::functions(),
            builtin_methods: prelude::methods(),
            closure_captures: sendable::ClosureCaptures::default(),
            closure_of_local: sendable::ClosuresOfLocals::default(),
            reported_captures: HashSet::new(),
            route_depth: 0,
        }
    }

    fn check_module(&mut self, module: &Module) {
        self.register_builtin_abilities();
        self.collect_declarations(&module.statements);
        self.check_ability_promises(&module.statements);

        self.hoist_functions(&module.statements);
        for statement in &module.statements {
            self.statement(statement);
        }
    }

    /// Hands back the tables, or the problems that stopped them being trustworthy.
    fn finish(mut self) -> Result<Checked, Vec<Diagnostic>> {
        self.report_undecided();

        if !self.diagnostics.is_empty() {
            // People read a file from the top, so that is the order to report in.
            self.diagnostics.sort_by_key(|diagnostic| diagnostic.primary_span().start);
            return Err(self.diagnostics);
        }

        // A type variable is a hole that unification fills, and it may be filled
        // after the type mentioning it was first written down. Resolving everything
        // once at the end is what guarantees no hole reaches the next phase.
        let variables = &self.variables;
        for declared in self.checked.types.values_mut() {
            *declared = variables.resolve(declared);
        }
        for local in &mut self.checked.locals {
            local.declared = variables.resolve(&local.declared);
        }
        Ok(self.checked)
    }

    fn report(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Records a value whose type has to arrive from elsewhere, and hands back the
    /// hole it will arrive in. See [`Undecided`].
    fn undecided(&mut self, kind: Undecidable, span: Span) -> Type {
        let hole = self.variables.fresh();
        self.undecided.push(Undecided { hole: hole.clone(), span, kind });
        hole
    }

    fn report_undecided(&mut self) {
        for waiting in std::mem::take(&mut self.undecided) {
            // Anything pinned down since is no longer waiting on anyone.
            if !matches!(self.variables.resolve(&waiting.hole), Type::Variable(_)) {
                continue;
            }
            self.report(match waiting.kind {
                Undecidable::Nothing => messages::cannot_infer_nothing(waiting.span),
                Undecidable::Empty { what, example } => {
                    messages::cannot_infer_empty(what, example, waiting.span)
                }
            });
        }
    }

    // -----------------------------------------------------------------------
    // Built-in abilities
    // -----------------------------------------------------------------------

    fn register_builtin_abilities(&mut self) {
        self.register_builtin_ability("Json");
    }

    fn register_builtin_ability(&mut self, name: &str) {
        let id = AbilityId(self.checked.abilities.len() as u32);
        self.checked.abilities.push(Ability {
            name: name.to_string(),
            functions: Vec::new(),
            declaration: NodeId(0),
            span: Span::default(),
        });
        self.globals.insert(name.to_string(), Global::Ability(id));
    }

    // -----------------------------------------------------------------------
    // Collecting declarations
    // -----------------------------------------------------------------------

    /// Registers every `type`, `choice` and `ability` in the file.
    ///
    /// Names come first and contents second, so that two types may mention each
    /// other regardless of which is written first.
    fn collect_declarations(&mut self, statements: &[Stmt]) {
        for statement in statements {
            match &statement.kind {
                StmtKind::Type(declaration) => self.register_type(statement.id, declaration),
                StmtKind::Choice(declaration) => self.register_choice(statement.id, declaration),
                StmtKind::Ability(declaration) => self.register_ability(statement.id, declaration),
                _ => {}
            }
        }

        // Contents, now that every name is known.
        for statement in statements {
            match &statement.kind {
                StmtKind::Type(declaration) => self.fill_type(statement.id, declaration),
                StmtKind::Choice(declaration) => self.fill_choice(declaration),
                StmtKind::Ability(declaration) => self.fill_ability(declaration),
                _ => {}
            }
        }
    }

    /// Notes a name as taken, or reports that it already was.
    fn claim(&mut self, name: &Name) -> bool {
        match self.global_spans.get(&name.text) {
            Some(first) => {
                let first = *first;
                self.report(messages::duplicate_declaration(&name.text, name.span, first));
                false
            }
            None => {
                self.global_spans.insert(name.text.clone(), name.span);
                true
            }
        }
    }

    fn register_type(&mut self, statement: NodeId, declaration: &TypeDecl) {
        if !self.claim(&declaration.name) {
            return;
        }
        let id = TypeId(self.checked.declared_types.len() as u32);
        self.checked.declared_types.push(DeclaredType {
            name: declaration.name.text.clone(),
            fields: Vec::new(),
            methods: Vec::new(),
            abilities: Vec::new(),
            constructor: Constructor::Automatic,
            declaration: statement,
            span: declaration.name.span,
        });
        self.globals.insert(declaration.name.text.clone(), Global::Type(id));
    }

    fn register_choice(&mut self, statement: NodeId, declaration: &ChoiceDecl) {
        if !self.claim(&declaration.name) {
            return;
        }
        let id = ChoiceId(self.checked.choices.len() as u32);
        self.checked.choices.push(Choice {
            name: declaration.name.text.clone(),
            variants: Vec::new(),
            declaration: statement,
            span: declaration.name.span,
        });
        self.globals.insert(declaration.name.text.clone(), Global::Choice(id));
    }

    fn register_ability(&mut self, statement: NodeId, declaration: &AbilityDecl) {
        if !self.claim(&declaration.name) {
            return;
        }
        let id = AbilityId(self.checked.abilities.len() as u32);
        self.checked.abilities.push(Ability {
            name: declaration.name.text.clone(),
            functions: Vec::new(),
            declaration: statement,
            span: declaration.name.span,
        });
        self.globals.insert(declaration.name.text.clone(), Global::Ability(id));
    }

    fn fill_type(&mut self, statement: NodeId, declaration: &TypeDecl) {
        let Some(Global::Type(id)) = self.globals.get(&declaration.name.text).copied() else {
            return;
        };

        let fields: Vec<Field> = declaration
            .fields
            .iter()
            .map(|field| Field {
                name: field.name.text.clone(),
                declared: self.resolve_type(&field.declared_type),
                has_default: field.default.is_some(),
                span: field.name.span,
            })
            .collect();

        let abilities: Vec<AbilityId> = declaration
            .abilities
            .iter()
            .filter_map(|name| self.resolve_ability(name))
            .collect();

        // Methods are functions like any other, so they go in the same table; only
        // their `owner` says they belong to a type.
        let mut methods = Vec::new();
        let mut constructor = Constructor::Automatic;
        for function in &declaration.functions {
            let id = self.register_function(function, Some(id), statement);
            if function.name.text == "new" {
                constructor = Constructor::UserDefined(id);
            }
            methods.push(id);
        }

        if let Some(declared) = self.checked.declared_types.get_mut(id.index()) {
            declared.fields = fields;
            declared.abilities = abilities.clone();
            declared.methods = methods;
            declared.constructor = constructor;
        }

        for ability in &abilities {
            if let Some(ability) = self.checked.ability(*ability) {
                let ability = ability.name.clone();
                self.variables.record_provider(&ability, &declaration.name.text);
            }
        }
    }

    fn fill_choice(&mut self, declaration: &ChoiceDecl) {
        let Some(Global::Choice(id)) = self.globals.get(&declaration.name.text).copied() else {
            return;
        };

        let variants: Vec<Variant> = declaration
            .variants
            .iter()
            .map(|variant| Variant {
                name: variant.name.text.clone(),
                fields: variant
                    .fields
                    .iter()
                    .map(|field| Field {
                        name: field.name.text.clone(),
                        declared: self.resolve_type(&field.declared_type),
                        has_default: false,
                        span: field.name.span,
                    })
                    .collect(),
                span: variant.name.span,
            })
            .collect();

        if let Some(choice) = self.checked.choices.get_mut(id.index()) {
            choice.variants = variants;
        }
    }

    fn fill_ability(&mut self, declaration: &AbilityDecl) {
        let Some(Global::Ability(id)) = self.globals.get(&declaration.name.text).copied() else {
            return;
        };

        let functions: Vec<Required> = declaration
            .functions
            .iter()
            .map(|function| {
                let signature = self.signature_of(function);
                Required {
                    name: function.name.text.clone(),
                    signature,
                    span: function.name.span,
                }
            })
            .collect();

        if let Some(ability) = self.checked.abilities.get_mut(id.index()) {
            ability.functions = functions;
        }
    }

    /// Adds a function to the table and gives back its handle.
    fn register_function(
        &mut self,
        declaration: &FunctionDecl,
        owner: Option<TypeId>,
        statement: NodeId,
    ) -> FunctionId {
        let id = FunctionId(self.checked.functions.len() as u32);
        let frame = FrameId(self.checked.frames.len() as u32);
        self.checked.frames.push(Frame {
            kind: FrameKind::Function(id),
            slots: 0,
            // A function body can read the values around it, so the frame it sits in
            // is where those come from.
            parent: self.frames.last().copied(),
        });
        let signature = self.signature_of(declaration);
        self.checked.functions.push(Function {
            name: declaration.name.text.clone(),
            owner,
            signature,
            frame,
            parameters: Vec::new(),
            pure: declaration.pure,
            declaration: statement,
            span: declaration.name.span,
        });
        id
    }

    fn signature_of(&mut self, declaration: &FunctionDecl) -> Signature {
        let parameters = declaration
            .parameters
            .iter()
            .map(|parameter| Parameter {
                name: parameter.name.text.clone(),
                declared: self.resolve_type(&parameter.declared_type),
                optional: parameter.default.is_some(),
            })
            .collect();

        let returns = match &declaration.returns {
            Some(declared) => self.resolve_type(declared),
            // No `returns` means the function gives back Nothing.
            None => Type::Nothing,
        };

        Signature::new(parameters, returns)
    }

    /// Registers every function declared directly in `statements`, so that they may
    /// call each other whichever order they are written in.
    fn hoist_functions(&mut self, statements: &[Stmt]) {
        for statement in statements {
            if let StmtKind::Function(declaration) = &statement.kind {
                let id = self.register_function(declaration, None, statement.id);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.functions.push((declaration.name.text.clone(), id));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Abilities
    // -----------------------------------------------------------------------

    /// Checks that every type claiming an ability really provides it.
    fn check_ability_promises(&mut self, statements: &[Stmt]) {
        for statement in statements {
            let StmtKind::Type(declaration) = &statement.kind else { continue };
            let Some(Global::Type(id)) = self.globals.get(&declaration.name.text).copied() else {
                continue;
            };
            let Some(declared) = self.checked.declared_type(id) else { continue };
            let provider = declared.name.clone();
            let abilities = declared.abilities.clone();

            for (position, ability) in abilities.iter().enumerate() {
                let Some(ability) = self.checked.ability(*ability) else { continue };
                let wanted = ability.name.clone();
                let claim = declaration
                    .abilities
                    .get(position)
                    .map(|name| name.span)
                    .unwrap_or(declaration.name.span);

                if wanted == "Json" {
                    let provider_type = Type::named(&provider);
                    if !can_json(&provider_type, &self.checked) {
                        self.report(messages::not_json(&provider_type, claim));
                    }
                    continue;
                }

                let required = ability.functions.clone();
                for required in &required {
                    self.check_one_promise(&provider, id, &wanted, required, claim);
                }
            }
        }
    }

    fn check_one_promise(
        &mut self,
        provider: &str,
        id: TypeId,
        ability: &str,
        required: &Required,
        claim: Span,
    ) {
        let Some(function) = self.checked.method(id, &required.name) else {
            self.report(messages::missing_ability_function(
                provider,
                ability,
                &required.signature,
                &required.name,
                claim,
                required.span,
            ));
            return;
        };

        let Some(found) = self.checked.function(function) else { return };
        let signature = found.signature.clone();
        let span = found.span;

        if signature != required.signature {
            self.report(messages::ability_signature_mismatch(
                provider,
                ability,
                &required.name,
                &signature,
                &required.signature,
                span,
                required.span,
            ));
        }
    }

    // -----------------------------------------------------------------------
    // Types as written
    // -----------------------------------------------------------------------

    /// Turns a type as written in the source into a [`Type`].
    fn resolve_type(&mut self, declared: &TypeExpr) -> Type {
        match &declared.kind {
            TypeKind::Named(name) => self.resolve_named_type(name),
            TypeKind::List(item) => Type::list(self.resolve_type(item)),
            TypeKind::Maybe(item) => Type::maybe(self.resolve_type(item)),
            TypeKind::Channel(item) => Type::channel(self.resolve_type(item)),
            TypeKind::Task(item) => Type::task(self.resolve_type(item)),
            TypeKind::Shared(item) => Type::shared(self.resolve_type(item)),
            TypeKind::Map { key, value } => {
                Type::map(self.resolve_type(key), self.resolve_type(value))
            }
            TypeKind::Fallible { ok, error } => {
                Type::fallible(self.resolve_type(ok), self.resolve_type(error))
            }
            TypeKind::Tuple(items) => {
                Type::Tuple(items.iter().map(|item| self.resolve_type(item)).collect())
            }
            TypeKind::Function { parameters, returns } => Type::function(
                parameters.iter().map(|item| self.resolve_type(item)).collect(),
                match returns {
                    Some(returns) => self.resolve_type(returns),
                    None => Type::Nothing,
                },
            ),
        }
    }

    fn resolve_named_type(&mut self, name: &Name) -> Type {
        match name.text.as_str() {
            "Int" => return Type::Int,
            "Float" => return Type::Float,
            "Bool" => return Type::Bool,
            "Text" => return Type::Text,
            "Nothing" => return Type::Nothing,
            _ => {}
        }

        match self.globals.get(&name.text) {
            Some(Global::Type(_) | Global::Choice(_)) => return Type::named(&name.text),
            Some(Global::Ability(_)) => return Type::Ability(name.text.clone()),
            None => {}
        }

        // A single capital letter is a type parameter, and is generic without being
        // declared: `to first_of(items: list of T) returns maybe T`.
        if is_type_parameter(&name.text) {
            return Type::parameter(&name.text);
        }

        let known = self.known_type_names();
        self.report(messages::undefined_type(&name.text, name.span, &known));
        Type::Unknown
    }

    fn resolve_ability(&mut self, name: &Name) -> Option<AbilityId> {
        match self.globals.get(&name.text) {
            Some(Global::Ability(id)) => Some(*id),
            _ => {
                let known: Vec<String> = self
                    .checked
                    .abilities
                    .iter()
                    .map(|ability| ability.name.clone())
                    .collect();
                self.report(messages::undefined_ability(&name.text, name.span, &known));
                None
            }
        }
    }

    fn known_type_names(&self) -> Vec<String> {
        let mut names: Vec<String> =
            ["Int", "Float", "Bool", "Text", "Nothing"].iter().map(|n| n.to_string()).collect();
        names.extend(self.globals.keys().cloned());
        names
    }

    // -----------------------------------------------------------------------
    // Frames, scopes and locals
    // -----------------------------------------------------------------------

    fn push_scope(&mut self) {
        let frame = self.frames.last().copied().unwrap_or(Checked::TOP_LEVEL);
        self.scopes.push(Scope { frame, locals: Vec::new(), functions: Vec::new() });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Enters a frame that already exists, as a function's does.
    fn enter_frame(&mut self, frame: FrameId) {
        self.frames.push(frame);
        self.push_scope();
    }

    fn leave_frame(&mut self) {
        self.pop_scope();
        self.frames.pop();
    }

    /// Makes a frame for a closure, whose body has no declaration to hang one on.
    fn new_frame(&mut self, kind: FrameKind) -> FrameId {
        let id = FrameId(self.checked.frames.len() as u32);
        self.checked.frames.push(Frame {
            kind,
            slots: 0,
            parent: self.frames.last().copied(),
        });
        id
    }

    fn declare_local(&mut self, name: &Name, declared: Type, changing: bool) -> LocalId {
        let frame = self.frames.last().copied().unwrap_or(Checked::TOP_LEVEL);
        let slot = match self.checked.frames.get_mut(frame.index()) {
            Some(holder) => {
                let slot = holder.slots;
                holder.slots += 1;
                slot
            }
            None => 0,
        };

        let id = LocalId(self.checked.locals.len() as u32);
        self.checked.locals.push(Local {
            name: name.text.clone(),
            frame,
            slot,
            changing,
            declared,
            span: name.span,
        });
        if let Some(scope) = self.scopes.last_mut() {
            scope.locals.push((name.text.clone(), id));
        }
        id
    }

    /// Finds a local, counting the frames crossed on the way out so the compiler
    /// knows whether it is a slot or a capture.
    fn lookup_local(&self, name: &str) -> Option<LocalRef> {
        let mut hops = 0u32;
        let mut frame = self.frames.last().copied();

        for scope in self.scopes.iter().rev() {
            if Some(scope.frame) != frame {
                hops += 1;
                frame = Some(scope.frame);
            }
            if let Some((_, local)) = scope.locals.iter().rev().find(|(held, _)| held == name) {
                return Some(LocalRef { local: *local, hops });
            }
        }
        None
    }

    fn lookup_function(&self, name: &str) -> Option<FunctionId> {
        self.scopes.iter().rev().find_map(|scope| {
            scope.functions.iter().rev().find(|(held, _)| held == name).map(|(_, id)| *id)
        })
    }

    /// Every name a value could have had, for the "did you mean" in an error.
    fn visible_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for scope in &self.scopes {
            names.extend(scope.locals.iter().map(|(name, _)| name.clone()));
            names.extend(scope.functions.iter().map(|(name, _)| name.clone()));
        }
        names.extend(self.builtin_functions.iter().map(|f| f.name.to_string()));
        names
    }

    fn local_type(&self, local: LocalId) -> Type {
        self.checked
            .local(local)
            .map(|local| local.declared.clone())
            .unwrap_or(Type::Unknown)
    }

    // -----------------------------------------------------------------------
    // Recording answers
    // -----------------------------------------------------------------------

    /// Notes the type of an expression, which is the map phase 3 reads.
    fn record(&mut self, expr: &Expr, found: Type) -> Type {
        let found = self.variables.resolve(&found);
        self.checked.types.insert(expr.id, found.clone());
        found
    }

    fn resolve_to(&mut self, node: NodeId, resolution: Resolution) {
        self.checked.resolutions.insert(node, resolution);
    }

    /// Reports a mismatch unless the value fits, and hands back the type to carry
    /// on with — the wanted one, so that one disagreement makes one message.
    fn expect_type(&mut self, found: Type, wanted: &Wanted, span: Span) -> Type {
        let Some(wanted) = wanted.as_type() else { return found };
        if self.variables.fits(&found, wanted) {
            return self.variables.resolve(&found);
        }

        let found = self.variables.resolve(&found);
        let wanted = self.variables.resolve(wanted);
        self.report(messages::mismatch(&wanted, &found, span));
        wanted
    }
}

/// Whether a name is a type parameter: a single capital letter, as `T`, `K` and `V`.
fn is_type_parameter(name: &str) -> bool {
    let mut characters = name.chars();
    matches!((characters.next(), characters.next()), (Some(only), None) if only.is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_capital_letter_is_a_type_parameter() {
        assert!(is_type_parameter("T"));
        assert!(is_type_parameter("V"));
        assert!(!is_type_parameter("Int"));
        assert!(!is_type_parameter("t"));
        assert!(!is_type_parameter(""));
    }
}

//! Printing an AST back out as an indented tree.
//!
//! This is what `vaab parse` shows, and what the snapshot tests compare against.
//! It deliberately omits spans: a snapshot full of byte offsets changes whenever
//! anyone reformats an example, which teaches you to stop reading diffs.

use std::fmt::Write as _;

use crate::ast::*;

/// Renders a module as an indented tree, ending with a newline.
pub fn print_module(module: &Module) -> String {
    let mut printer = Printer::default();
    printer.line("module");
    printer.indented(|printer| {
        for statement in &module.statements {
            printer.statement(statement);
        }
    });
    printer.output
}

#[derive(Default)]
struct Printer {
    output: String,
    depth: usize,
}

impl Printer {
    fn line(&mut self, text: impl AsRef<str>) {
        for _ in 0..self.depth {
            self.output.push_str("  ");
        }
        self.output.push_str(text.as_ref());
        self.output.push('\n');
    }

    fn indented(&mut self, body: impl FnOnce(&mut Self)) {
        self.depth += 1;
        body(self);
        self.depth -= 1;
    }

    /// Writes `label`, then everything `body` writes one level further in.
    fn node(&mut self, label: impl AsRef<str>, body: impl FnOnce(&mut Self)) {
        self.line(label);
        self.indented(body);
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    fn statement(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Let(declaration) => {
                let word = if declaration.changing { "let changing" } else { "let" };
                self.node(format!("{word} {}", declaration.name.text), |printer| {
                    if let Some(declared) = &declaration.declared_type {
                        printer.node("declared type", |printer| printer.type_expression(declared));
                    }
                    printer.expression(&declaration.value);
                });
            }
            StmtKind::Assign(assignment) => self.node("assign", |printer| {
                printer.node("target", |printer| printer.expression(&assignment.target));
                printer.node("value", |printer| printer.expression(&assignment.value));
            }),
            StmtKind::Return(value) => match value {
                Some(value) => self.node("return", |printer| printer.expression(value)),
                None => self.line("return"),
            },
            StmtKind::ForEach(loop_) => self.node("for each", |printer| {
                printer.node("pattern", |printer| printer.pattern(&loop_.pattern));
                printer.node("in", |printer| printer.expression(&loop_.sequence));
                printer.block(&loop_.body);
            }),
            StmtKind::While(loop_) => self.node("while", |printer| {
                printer.node("condition", |printer| printer.expression(&loop_.condition));
                printer.block(&loop_.body);
            }),
            StmtKind::Repeat(loop_) => self.node("repeat", |printer| {
                printer.node("times", |printer| printer.expression(&loop_.count));
                printer.block(&loop_.body);
            }),
            StmtKind::Send(send) => self.node("send", |printer| {
                printer.node("value", |printer| printer.expression(&send.value));
                printer.node("to", |printer| printer.expression(&send.channel));
            }),
            StmtKind::Close(channel) => {
                self.node("close", |printer| printer.expression(channel))
            }
            StmtKind::Together(body) => self.node("together", |printer| printer.block(body)),
            StmtKind::Function(function) => self.function(function),
            StmtKind::Type(declaration) => self.type_declaration(declaration),
            StmtKind::Choice(declaration) => self.choice_declaration(declaration),
            StmtKind::Ability(declaration) => self.ability_declaration(declaration),
            StmtKind::Expr(expression) => self.expression(expression),
        }
    }

    fn block(&mut self, block: &Block) {
        if block.statements.is_empty() {
            self.line("block (empty)");
            return;
        }
        self.node("block", |printer| {
            for statement in &block.statements {
                printer.statement(statement);
            }
        });
    }

    // -----------------------------------------------------------------------
    // Declarations
    // -----------------------------------------------------------------------

    fn function(&mut self, function: &FunctionDecl) {
        let purity = if function.pure { "pure " } else { "" };
        self.node(format!("{purity}to {}", function.name.text), |printer| {
            for parameter in &function.parameters {
                printer.node(format!("parameter {}", parameter.name.text), |printer| {
                    printer.type_expression(&parameter.declared_type);
                    if let Some(default) = &parameter.default {
                        printer.node("default", |printer| printer.expression(default));
                    }
                });
            }
            if let Some(returns) = &function.returns {
                printer.node("returns", |printer| printer.type_expression(returns));
            }
            match &function.body {
                Some(FunctionBody::Block(block)) => printer.block(block),
                Some(FunctionBody::Expr(expression)) => {
                    printer.node("body", |printer| printer.expression(expression))
                }
                None => printer.line("required (no body)"),
            }
        });
    }

    fn type_declaration(&mut self, declaration: &TypeDecl) {
        let mut label = format!("type {}", declaration.name.text);
        if !declaration.abilities.is_empty() {
            let abilities: Vec<&str> =
                declaration.abilities.iter().map(|name| name.text.as_str()).collect();
            let _ = write!(label, " can {}", abilities.join(", "));
        }
        self.node(label, |printer| {
            for field in &declaration.fields {
                printer.node(format!("field {}", field.name.text), |printer| {
                    printer.type_expression(&field.declared_type);
                    if let Some(default) = &field.default {
                        printer.node("default", |printer| printer.expression(default));
                    }
                });
            }
            for function in &declaration.functions {
                printer.function(function);
            }
        });
    }

    fn choice_declaration(&mut self, declaration: &ChoiceDecl) {
        self.node(format!("choice {}", declaration.name.text), |printer| {
            for variant in &declaration.variants {
                if variant.fields.is_empty() {
                    printer.line(format!("variant {}", variant.name.text));
                    continue;
                }
                printer.node(format!("variant {}", variant.name.text), |printer| {
                    for field in &variant.fields {
                        printer.node(format!("field {}", field.name.text), |printer| {
                            printer.type_expression(&field.declared_type);
                        });
                    }
                });
            }
        });
    }

    fn ability_declaration(&mut self, declaration: &AbilityDecl) {
        self.node(format!("ability {}", declaration.name.text), |printer| {
            for function in &declaration.functions {
                printer.function(function);
            }
        });
    }

    // -----------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------

    fn expression(&mut self, expression: &Expr) {
        match &expression.kind {
            ExprKind::Int(value) => self.line(format!("int {value}")),
            ExprKind::Float(value) => self.line(format!("float {value}")),
            ExprKind::Bool(value) => self.line(format!("bool {}", if *value { "yes" } else { "no" })),
            ExprKind::Nothing => self.line("nothing"),
            ExprKind::Name(name) => self.line(format!("name {}", name.text)),
            ExprKind::SelfValue => self.line("self"),

            ExprKind::Text(parts) => self.node("text", |printer| {
                for part in parts {
                    match part {
                        TextPart::Literal(text) => {
                            printer.line(format!("literal {:?}", text));
                        }
                        TextPart::Interpolation(inner) => {
                            printer.node("hole", |printer| printer.expression(inner));
                        }
                    }
                }
            }),

            ExprKind::List(items) => self.sequence("list", items),
            ExprKind::Tuple(items) => self.sequence("tuple", items),
            ExprKind::Map(entries) => self.node("map", |printer| {
                for entry in entries {
                    printer.node("entry", |printer| {
                        printer.node("key", |printer| printer.expression(&entry.key));
                        printer.node("value", |printer| printer.expression(&entry.value));
                    });
                }
            }),
            ExprKind::Range { start, end } => self.node("range", |printer| {
                printer.expression(start);
                printer.expression(end);
            }),

            ExprKind::Unary { operator, operand } => {
                self.node(format!("unary {}", operator.spelling()), |printer| {
                    printer.expression(operand)
                })
            }
            ExprKind::Binary { operator, left, right } => {
                self.node(format!("binary {}", operator.spelling()), |printer| {
                    printer.expression(left);
                    printer.expression(right);
                })
            }

            ExprKind::Call { callee, arguments } => self.node("call", |printer| {
                printer.node("callee", |printer| printer.expression(callee));
                for argument in arguments {
                    let label = match &argument.name {
                        Some(name) => format!("argument {}:", name.text),
                        None => "argument".to_string(),
                    };
                    printer.node(label, |printer| printer.expression(&argument.value));
                }
            }),
            ExprKind::Member { target, name } => {
                self.node(format!("member .{}", name.text), |printer| printer.expression(target))
            }
            ExprKind::Index { target, index } => self.node("index", |printer| {
                printer.node("of", |printer| printer.expression(target));
                printer.node("at", |printer| printer.expression(index));
            }),

            ExprKind::Closure { parameters, body } => {
                let names: Vec<&str> = parameters.iter().map(|name| name.text.as_str()).collect();
                self.node(format!("closure ({})", names.join(", ")), |printer| match &**body {
                    FunctionBody::Block(block) => printer.block(block),
                    FunctionBody::Expr(expression) => printer.expression(expression),
                });
            }

            ExprKind::If(branch) => self.if_expression(branch),
            ExprKind::Match(expression) => self.node("match", |printer| {
                printer.node("subject", |printer| printer.expression(&expression.subject));
                for arm in &expression.arms {
                    printer.match_arm(arm);
                }
            }),

            ExprKind::Found(inner) => self.node("found", |printer| printer.expression(inner)),
            ExprKind::Success(inner) => self.node("success", |printer| printer.expression(inner)),
            ExprKind::Failure(inner) => self.node("failure", |printer| printer.expression(inner)),
            ExprKind::Try(inner) => self.node("try", |printer| printer.expression(inner)),
            ExprKind::Otherwise { value, fallback } => self.node("otherwise", |printer| {
                printer.node("value", |printer| printer.expression(value));
                printer.node("fallback", |printer| printer.expression(fallback));
            }),

            ExprKind::Receive { channel } => {
                self.node("receive from", |printer| printer.expression(channel))
            }
            ExprKind::Start(body) => self.node("start", |printer| printer.block(body)),
            ExprKind::Select(expression) => self.node("select", |printer| {
                for arm in &expression.arms {
                    match arm {
                        SelectArm::Receive { channel, binding, body, .. } => {
                            let label = match binding {
                                Some(name) => format!("when receive as {}", name.text),
                                None => "when receive".to_string(),
                            };
                            printer.node(label, |printer| {
                                printer.node("from", |printer| printer.expression(channel));
                                printer.block(body);
                            });
                        }
                        SelectArm::Timeout { amount, unit, body, .. } => {
                            printer.node(format!("when timeout after ({})", unit.text), |printer| {
                                printer.expression(amount);
                                printer.block(body);
                            });
                        }
                    }
                }
                if let Some(otherwise) = &expression.otherwise {
                    printer.node("otherwise", |printer| printer.block(otherwise));
                }
            }),
        }
    }

    fn sequence(&mut self, label: &str, items: &[Expr]) {
        if items.is_empty() {
            self.line(format!("{label} (empty)"));
            return;
        }
        self.node(label, |printer| {
            for item in items {
                printer.expression(item);
            }
        });
    }

    fn if_expression(&mut self, branch: &IfExpr) {
        self.node("if", |printer| {
            printer.node("condition", |printer| printer.expression(&branch.condition));
            printer.node("then", |printer| printer.block(&branch.then_block));
            match &branch.else_branch {
                Some(ElseBranch::Block(block)) => {
                    printer.node("else", |printer| printer.block(block))
                }
                Some(ElseBranch::If(nested)) => {
                    printer.node("else", |printer| printer.if_expression(nested))
                }
                None => {}
            }
        });
    }

    fn match_arm(&mut self, arm: &MatchArm) {
        let label = match &arm.pattern {
            ArmPattern::Otherwise => "otherwise".to_string(),
            ArmPattern::Pattern(_) => "when".to_string(),
        };
        self.node(label, |printer| {
            if let ArmPattern::Pattern(pattern) = &arm.pattern {
                printer.pattern(pattern);
            }
            if let Some(guard) = &arm.guard {
                printer.node("if", |printer| printer.expression(guard));
            }
            printer.node("then", |printer| match &arm.body {
                ArmBody::Block(block) => printer.block(block),
                ArmBody::Expr(expression) => printer.expression(expression),
            });
        });
    }

    // -----------------------------------------------------------------------
    // Patterns and types
    // -----------------------------------------------------------------------

    fn pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Int(value) => self.line(format!("int {value}")),
            PatternKind::Float(value) => self.line(format!("float {value}")),
            PatternKind::Bool(value) => {
                self.line(format!("bool {}", if *value { "yes" } else { "no" }))
            }
            PatternKind::Text(text) => self.line(format!("text {text:?}")),
            PatternKind::Binding(name) => self.line(format!("bind {}", name.text)),
            PatternKind::Nothing => self.line("nothing"),
            PatternKind::Found(inner) => self.node("found", |printer| printer.pattern(inner)),
            PatternKind::Success(inner) => self.node("success", |printer| printer.pattern(inner)),
            PatternKind::Failure(inner) => self.node("failure", |printer| printer.pattern(inner)),
            PatternKind::Variant { path, fields } => {
                let joined: Vec<&str> = path.iter().map(|name| name.text.as_str()).collect();
                let label = format!("variant {}", joined.join("."));
                if fields.is_empty() {
                    self.line(label);
                } else {
                    self.node(label, |printer| {
                        for field in fields {
                            printer.pattern(field);
                        }
                    });
                }
            }
            PatternKind::List { elements, rest } => {
                let label = if *rest { "list pattern (and the rest)" } else { "list pattern" };
                if elements.is_empty() {
                    self.line(label);
                } else {
                    self.node(label, |printer| {
                        for element in elements {
                            printer.pattern(element);
                        }
                    });
                }
            }
            PatternKind::Tuple(items) => self.node("tuple pattern", |printer| {
                for item in items {
                    printer.pattern(item);
                }
            }),
        }
    }

    fn type_expression(&mut self, declared: &TypeExpr) {
        match &declared.kind {
            TypeKind::Named(name) => self.line(format!("type {}", name.text)),
            TypeKind::List(item) => self.node("list of", |printer| printer.type_expression(item)),
            TypeKind::Map { key, value } => self.node("map of", |printer| {
                printer.node("key", |printer| printer.type_expression(key));
                printer.node("value", |printer| printer.type_expression(value));
            }),
            TypeKind::Maybe(item) => self.node("maybe", |printer| printer.type_expression(item)),
            TypeKind::Channel(item) => {
                self.node("channel of", |printer| printer.type_expression(item))
            }
            TypeKind::Task(item) => self.node("task of", |printer| printer.type_expression(item)),
            TypeKind::Shared(item) => self.node("shared", |printer| printer.type_expression(item)),
            TypeKind::Tuple(items) => self.node("tuple type", |printer| {
                for item in items {
                    printer.type_expression(item);
                }
            }),
            TypeKind::Function { parameters, returns } => self.node("function type", |printer| {
                for parameter in parameters {
                    printer.node("takes", |printer| printer.type_expression(parameter));
                }
                if let Some(returns) = returns {
                    printer.node("returns", |printer| printer.type_expression(returns));
                }
            }),
            TypeKind::Fallible { ok, error } => self.node("or fails", |printer| {
                printer.node("ok", |printer| printer.type_expression(ok));
                printer.node("error", |printer| printer.type_expression(error));
            }),
        }
    }
}

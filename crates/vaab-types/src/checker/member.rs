//! `.field`, `.method()`, `Account.new(...)` and the rest of what follows a dot.
//!
//! A dot means one of two quite different things, depending on what is to its left.
//! `Account.new` reaches into a *declaration*; `account.owner` reaches into a
//! *value*. [`Checker::look_up_member`] decides which, and everything else works the
//! same either way.

use vaab_syntax::ast::{Expr, ExprKind, Name};
use vaab_syntax::span::Span;

use super::Inside;
use super::{Checker, Global, Wanted};
use crate::checked::{CastId, ChoiceId, Constructor, Resolution, TypeId};
use crate::messages;
use crate::prelude;
use crate::types::{Parameter, Signature, Type};

/// What a `.something` turned out to be.
pub(super) enum Member {
    /// Something to read: a field, a property such as `.is_empty`, or a variant with
    /// no payload.
    Value {
        declared: Type,
        resolution: Resolution,
    },
    /// Something to call.
    Callable {
        signature: Signature,
        resolution: Resolution,
        shape: CallShape,
    },
    /// `Channel.new` and `Shared.new`, which take a type rather than a value and so
    /// are checked on their own.
    Handle(Handle),
    /// Something already reported.
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle {
    Channel,
    Hub,
    Shared,
    Db,
    Store,
    LoggerStdout,
    LoggerStderr,
    LoggerFile,
    LoggerMemory,
    LoggerMulti,
}

/// How a particular callable's arguments are matched up. Constructors, ordinary
/// functions and functions held in a value differ here and nowhere else.
#[derive(Clone)]
pub(super) struct CallShape {
    /// What to call it in a message: "greet", or "Account.new".
    pub name: String,
    /// The type whose fields are being filled in, for a constructor or `.with`.
    pub owner: Option<String>,
    /// Where the thing being called was declared, for a "declared here" label.
    pub declared: Option<Span>,
    pub missing: WhenMissing,
    /// `.new` on a type takes its fields by name, so that two fields of the same
    /// type can never be given in the wrong order.
    pub names_required: bool,
    /// A function held in a value has a type but no parameter names, so there is
    /// nothing for an argument to be named after.
    pub names_allowed: bool,
}

/// What to do about a parameter the call did not mention.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WhenMissing {
    /// Use the default written on the declaration, or name what is missing.
    Default,
    /// `Account.new(...)`: the specification's "Account.new is missing `owner`."
    NewField,
    /// `.with(...)`: keep whatever the original had.
    Keep,
    /// A function value: the only thing to say is how many were expected.
    Count,
}

impl CallShape {
    /// The ordinary case: something with a declaration to point at.
    pub(super) fn function(name: impl Into<String>, declared: Option<Span>) -> Self {
        CallShape {
            name: name.into(),
            owner: None,
            declared,
            missing: WhenMissing::Default,
            names_required: false,
            names_allowed: true,
        }
    }

    /// A function reached through a value, which knows its types but not its names.
    pub(super) fn value(name: impl Into<String>) -> Self {
        CallShape {
            name: name.into(),
            owner: None,
            declared: None,
            missing: WhenMissing::Count,
            names_required: false,
            names_allowed: false,
        }
    }

    /// Filling in a type's fields, by name, as `.new`, `.raw` and `.with` all do.
    fn fields(name: String, owner: String, declared: Span, missing: WhenMissing) -> Self {
        CallShape {
            name,
            owner: Some(owner),
            declared: Some(declared),
            missing,
            names_required: true,
            names_allowed: true,
        }
    }
}

impl Checker {
    /// `account.balance`, read rather than called.
    pub(super) fn member(&mut self, expression: &Expr, target: &Expr, name: &Name) -> Type {
        match self.look_up_member(target, name) {
            Member::Value {
                declared,
                resolution,
            } => {
                self.resolve_to(expression.id, resolution);
                declared
            }
            Member::Callable { .. } | Member::Handle(_) => {
                self.report(messages::needs_call(&name.text, name.span));
                Type::Unknown
            }
            Member::Unknown => Type::Unknown,
        }
    }

    /// Works out what the dot in `target.name` reaches.
    pub(super) fn look_up_member(&mut self, target: &Expr, name: &Name) -> Member {
        if let ExprKind::Name(written) = &target.kind {
            if written.text == "request" && self.route_depth > 0 {
                if name.text == "who" {
                    return Member::Value {
                        declared: Type::fallible(Type::named("User"), Type::named("AuthError")),
                        resolution: Resolution::RequestWho,
                    };
                }
                if name.text == "headers" {
                    return Member::Value {
                        declared: Type::map(Type::Text, Type::Text),
                        resolution: Resolution::RequestField(3),
                    };
                }
                if let Some(index) = match name.text.as_str() {
                    "method" => Some(0),
                    "path" => Some(1),
                    "body" => Some(2),
                    _ => None,
                } {
                    return Member::Value {
                        declared: Type::Text,
                        resolution: Resolution::RequestField(index),
                    };
                }
            }
        }

        // A name to the left of a dot may be a declaration rather than a value. A
        // local of the same name wins, so `let account = ...` is not shadowed by a
        // type called `account`.
        if let ExprKind::Name(written) = &target.kind {
            if self.lookup_local(&written.text).is_none() {
                if written.text == "Json" && name.text == "parse" {
                    return Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("text", Type::Text)],
                            Type::fallible(Type::named("Json"), Type::named("JsonError")),
                        ),
                        resolution: Resolution::BuiltinMethod("json_parse"),
                        shape: CallShape::function("Json.parse", None),
                    };
                }
                match self.globals.get(&written.text).copied() {
                    Some(Global::Type(id)) => return self.type_member(id, name),
                    Some(Global::Cast(id)) => return self.cast_member(id, name),
                    Some(Global::Choice(id)) => return self.variant_member(id, name),
                    Some(Global::Ability(_)) | None => {}
                }

                // The two built-in handles are constructed like a type even though
                // they are not one.
                if written.text == "File" && name.text == "read" {
                    return Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("path", Type::Text)],
                            Type::fallible(Type::Text, Type::named("FileError")),
                        ),
                        resolution: Resolution::BuiltinMethod("file_read"),
                        shape: CallShape::function("File.read", None),
                    };
                }
                if written.text == "Socket" {
                    return match name.text.as_str() {
                        "connect" => Member::Callable {
                            signature: Signature::new(
                                vec![
                                    Parameter::new("url", Type::Text),
                                    Parameter::optional(
                                        "headers",
                                        Type::map(Type::Text, Type::Text),
                                    ),
                                    Parameter::optional("buffer", Type::Int),
                                ],
                                Type::fallible(Type::Socket, Type::named("SocketError")),
                            ),
                            resolution: Resolution::BuiltinMethod("socket_connect"),
                            shape: CallShape::function("Socket.connect", None),
                        },
                        _ => {
                            self.report(messages::unknown_member(
                                &Type::named("Socket"),
                                &name.text,
                                name.span,
                                &["connect".into()],
                            ));
                            Member::Unknown
                        }
                    };
                }
                if written.text == "Bytes" {
                    return match name.text.as_str() {
                        "from_base64" => Member::Callable {
                            signature: Signature::new(
                                vec![Parameter::new("text", Type::Text)],
                                Type::fallible(Type::Bytes, Type::named("BytesError")),
                            ),
                            resolution: Resolution::BuiltinMethod("bytes_from_base64"),
                            shape: CallShape::function("Bytes.from_base64", None),
                        },
                        "from_hex" => Member::Callable {
                            signature: Signature::new(
                                vec![Parameter::new("text", Type::Text)],
                                Type::fallible(Type::Bytes, Type::named("BytesError")),
                            ),
                            resolution: Resolution::BuiltinMethod("bytes_from_hex"),
                            shape: CallShape::function("Bytes.from_hex", None),
                        },
                        "from_base58" => Member::Callable {
                            signature: Signature::new(
                                vec![Parameter::new("text", Type::Text)],
                                Type::fallible(Type::Bytes, Type::named("BytesError")),
                            ),
                            resolution: Resolution::BuiltinMethod("bytes_from_base58"),
                            shape: CallShape::function("Bytes.from_base58", None),
                        },
                        _ => {
                            self.report(messages::unknown_bytes_member(
                                &name.text,
                                name.span,
                                &[
                                    "from_base64".into(),
                                    "from_hex".into(),
                                    "from_base58".into(),
                                ],
                            ));
                            Member::Unknown
                        }
                    };
                }
                if written.text == "Channel" && name.text == "new" {
                    return Member::Handle(Handle::Channel);
                }
                if written.text == "Hub" && name.text == "new" {
                    return Member::Handle(Handle::Hub);
                }
                if written.text == "Shared" && name.text == "new" {
                    return Member::Handle(Handle::Shared);
                }
                if written.text == "Db" && name.text == "connect" {
                    return Member::Handle(Handle::Db);
                }
                if written.text == "Store" && name.text == "open" {
                    return Member::Handle(Handle::Store);
                }
                if written.text == "Logger" {
                    return match name.text.as_str() {
                        "stdout" => Member::Handle(Handle::LoggerStdout),
                        "stderr" => Member::Handle(Handle::LoggerStderr),
                        "file" => Member::Handle(Handle::LoggerFile),
                        "memory" => Member::Handle(Handle::LoggerMemory),
                        "multi" => Member::Handle(Handle::LoggerMulti),
                        "debug" | "info" | "warn" | "error" => Member::Callable {
                            signature: Signature::new(
                                vec![Parameter::new("message", Type::Text)],
                                Type::Nothing,
                            ),
                            resolution: Resolution::BuiltinMethod(match name.text.as_str() {
                                "debug" => "log_debug",
                                "info" => "log_info",
                                "warn" => "log_warn",
                                _ => "log_error",
                            }),
                            shape: CallShape::function(format!("Logger.{}", name.text), None),
                        },
                        _ => {
                            self.report(messages::unknown_member(
                                &Type::Logger,
                                &name.text,
                                name.span,
                                &[
                                    "stdout".into(),
                                    "stderr".into(),
                                    "file".into(),
                                    "memory".into(),
                                    "multi".into(),
                                    "debug".into(),
                                    "info".into(),
                                    "warn".into(),
                                    "error".into(),
                                ],
                            ));
                            Member::Unknown
                        }
                    };
                }
            }
        }

        if let ExprKind::Name(written) = &target.kind {
            if self.lookup_local(&written.text).is_none() && written.text == "env" {
                return match name.text.as_str() {
                    "get" => Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("name", Type::Text)],
                            Type::maybe(Type::Text),
                        ),
                        resolution: Resolution::BuiltinMethod("env_get"),
                        shape: CallShape::function("env.get", None),
                    },
                    "required" => Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("name", Type::Text)],
                            Type::fallible(Type::Text, Type::named("EnvError")),
                        ),
                        resolution: Resolution::BuiltinMethod("env_required"),
                        shape: CallShape::function("env.required", None),
                    },
                    _ => {
                        self.report(messages::unknown_member(
                            &Type::named("Env"),
                            &name.text,
                            name.span,
                            &["get".into(), "required".into()],
                        ));
                        Member::Unknown
                    }
                };
            }
            if self.lookup_local(&written.text).is_none() && written.text == "log" {
                return match name.text.as_str() {
                    "debug" | "info" | "warn" | "error" => Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("message", Type::Text)],
                            Type::Nothing,
                        ),
                        resolution: Resolution::BuiltinMethod(match name.text.as_str() {
                            "debug" => "log_debug",
                            "info" => "log_info",
                            "warn" => "log_warn",
                            _ => "log_error",
                        }),
                        shape: CallShape::function(format!("log.{}", name.text), None),
                    },
                    "set_level" => Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("level", Type::Text)],
                            Type::Nothing,
                        ),
                        resolution: Resolution::BuiltinMethod("log_set_level"),
                        shape: CallShape::function("log.set_level", None),
                    },
                    "set_format" => Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("format", Type::Text)],
                            Type::Nothing,
                        ),
                        resolution: Resolution::BuiltinMethod("log_set_format"),
                        shape: CallShape::function("log.set_format", None),
                    },
                    _ => {
                        self.report(messages::unknown_member(
                            &Type::named("Log"),
                            &name.text,
                            name.span,
                            &[
                                "debug".into(),
                                "info".into(),
                                "warn".into(),
                                "error".into(),
                                "set_level".into(),
                                "set_format".into(),
                            ],
                        ));
                        Member::Unknown
                    }
                };
            }
            if self.lookup_local(&written.text).is_none() && written.text == "http" {
                return match name.text.as_str() {
                    "get" => Member::Callable {
                        signature: Signature::new(
                            vec![Parameter::new("url", Type::Text)],
                            Type::fallible(Type::Text, Type::named("HttpError")),
                        ),
                        resolution: Resolution::BuiltinMethod("http_get"),
                        shape: CallShape::function("http.get", None),
                    },
                    "post" => Member::Callable {
                        signature: Signature::new(
                            vec![
                                Parameter::new("url", Type::Text),
                                Parameter::new("body", Type::Text),
                            ],
                            Type::fallible(Type::Text, Type::named("HttpError")),
                        ),
                        resolution: Resolution::BuiltinMethod("http_post"),
                        shape: CallShape::function("http.post", None),
                    },
                    "send" => Member::Callable {
                        signature: Signature::new(
                            vec![
                                Parameter::new("method", Type::Text),
                                Parameter::new("url", Type::Text),
                                Parameter::new("body", Type::Text),
                                Parameter::new("headers", Type::map(Type::Text, Type::Text)),
                            ],
                            Type::fallible(Type::Text, Type::named("HttpError")),
                        ),
                        resolution: Resolution::BuiltinMethod("http_send"),
                        shape: CallShape::function("http.send", None),
                    },
                    _ => {
                        self.report(messages::unknown_member(
                            &Type::named("Http"),
                            &name.text,
                            name.span,
                            &["get".into(), "post".into(), "send".into()],
                        ));
                        Member::Unknown
                    }
                };
            }
            if self.lookup_local(&written.text).is_none() && written.text == "Clock" {
                return match name.text.as_str() {
                    "millis" => Member::Callable {
                        signature: Signature::new(Vec::new(), Type::Int),
                        resolution: Resolution::BuiltinMethod("clock_millis"),
                        shape: CallShape::function("Clock.millis", None),
                    },
                    "monotonic_millis" => Member::Callable {
                        signature: Signature::new(Vec::new(), Type::Int),
                        resolution: Resolution::BuiltinMethod("clock_monotonic_millis"),
                        shape: CallShape::function("Clock.monotonic_millis", None),
                    },
                    _ => {
                        self.report(messages::unknown_member(
                            &Type::named("Clock"),
                            &name.text,
                            name.span,
                            &["millis".into(), "monotonic_millis".into()],
                        ));
                        Member::Unknown
                    }
                };
            }
            if self.lookup_local(&written.text).is_none() && written.text == "Float" {
                return match name.text.as_str() {
                    "pi" => Member::Value {
                        declared: Type::Float,
                        resolution: Resolution::BuiltinMethod("float_pi"),
                    },
                    "infinity" => Member::Value {
                        declared: Type::Float,
                        resolution: Resolution::BuiltinMethod("float_infinity"),
                    },
                    _ => {
                        self.report(messages::unknown_member(
                            &Type::named("Float"),
                            &name.text,
                            name.span,
                            &["pi".into(), "infinity".into()],
                        ));
                        Member::Unknown
                    }
                };
            }
        }

        let found = self.expression(target, Wanted::Anything);
        self.value_member(&found, name, target.span)
    }

    // -----------------------------------------------------------------------
    // Reaching into a declaration
    // -----------------------------------------------------------------------

    /// `Account.new`, `Email.raw`.
    fn type_member(&mut self, id: TypeId, name: &Name) -> Member {
        let Some(declared) = self.checked.declared_type(id) else {
            return Member::Unknown;
        };
        let owner = declared.name.clone();
        let constructor = declared.constructor;
        let fields = declared.fields.clone();
        let at = declared.span;

        match name.text.as_str() {
            "new" => match constructor {
                Constructor::Automatic => Member::Callable {
                    signature: Signature::new(
                        fields
                            .iter()
                            .map(|field| Parameter {
                                name: field.name.clone(),
                                declared: field.declared.clone(),
                                optional: field.has_default,
                            })
                            .collect(),
                        Type::named(&owner),
                    ),
                    resolution: Resolution::AutomaticNew(id),
                    shape: CallShape::fields(
                        format!("{owner}.new"),
                        owner,
                        at,
                        WhenMissing::NewField,
                    ),
                },
                Constructor::UserDefined(function) => {
                    let Some(found) = self.checked.function(function) else {
                        return Member::Unknown;
                    };
                    Member::Callable {
                        signature: found.signature.clone(),
                        resolution: Resolution::UserNew {
                            declared: id,
                            function,
                        },
                        shape: CallShape::function(format!("{owner}.new"), Some(found.span)),
                    }
                }
            },

            "raw" => {
                if !matches!(self.inside, Some(Inside::Type(inside)) if inside == id) {
                    self.report(messages::raw_outside_type(&owner, name.span));
                }
                Member::Callable {
                    // `raw` takes every field, defaults and all: it is the way to
                    // build a value without going through the checks in `to new`.
                    signature: Signature::new(
                        fields
                            .iter()
                            .map(|field| Parameter::new(field.name.clone(), field.declared.clone()))
                            .collect(),
                        Type::named(&owner),
                    ),
                    resolution: Resolution::Raw(id),
                    shape: CallShape::fields(
                        format!("{owner}.raw"),
                        owner,
                        at,
                        WhenMissing::NewField,
                    ),
                }
            }

            _ => {
                let known = vec!["new".to_string(), "raw".to_string()];
                self.report(messages::unknown_member(
                    &Type::named(&owner),
                    &name.text,
                    name.span,
                    &known,
                ));
                Member::Unknown
            }
        }
    }

    /// `Counter.new`, `Counter.zero`, `Counter.raw`.
    fn cast_member(&mut self, id: CastId, name: &Name) -> Member {
        let Some(declared) = self.checked.declared_cast(id) else {
            return Member::Unknown;
        };
        let owner = declared.name.clone();
        let constructor = declared.constructor;
        let fields = declared.fields.clone();
        let at = declared.span;

        if let Some(function) = self.checked.cast_class_method(id, &name.text) {
            let Some(method) = self.checked.function(function) else {
                return Member::Unknown;
            };
            return Member::Callable {
                signature: method.signature.clone(),
                resolution: Resolution::CastClassMethod { cast: id, function },
                shape: CallShape::function(format!("{owner}.{}", name.text), Some(method.span)),
            };
        }

        match name.text.as_str() {
            "new" => match constructor {
                Constructor::Automatic => Member::Callable {
                    signature: Signature::new(
                        fields
                            .iter()
                            .map(|field| Parameter {
                                name: field.name.clone(),
                                declared: field.declared.clone(),
                                optional: field.has_default,
                            })
                            .collect(),
                        Type::named(&owner),
                    ),
                    resolution: Resolution::AutomaticCastNew(id),
                    shape: CallShape::fields(
                        format!("{owner}.new"),
                        owner,
                        at,
                        WhenMissing::NewField,
                    ),
                },
                Constructor::UserDefined(function) => {
                    let Some(found) = self.checked.function(function) else {
                        return Member::Unknown;
                    };
                    Member::Callable {
                        signature: found.signature.clone(),
                        resolution: Resolution::UserCastNew { cast: id, function },
                        shape: CallShape::function(format!("{owner}.new"), Some(found.span)),
                    }
                }
            },

            "raw" => {
                if !matches!(self.inside, Some(Inside::Cast(inside)) if inside == id) {
                    self.report(messages::raw_outside_type(&owner, name.span));
                }
                Member::Callable {
                    signature: Signature::new(
                        fields
                            .iter()
                            .map(|field| Parameter::new(field.name.clone(), field.declared.clone()))
                            .collect(),
                        Type::named(&owner),
                    ),
                    resolution: Resolution::RawCast(id),
                    shape: CallShape::fields(
                        format!("{owner}.raw"),
                        owner,
                        at,
                        WhenMissing::NewField,
                    ),
                }
            }

            _ => {
                let mut known = vec!["new".to_string(), "raw".to_string()];
                known.extend(declared.methods.iter().filter_map(|function| {
                    self.checked
                        .function(*function)
                        .filter(|function| function.class_method)
                        .map(|function| function.name.clone())
                }));
                self.report(messages::unknown_member(
                    &Type::named(&owner),
                    &name.text,
                    name.span,
                    &known,
                ));
                Member::Unknown
            }
        }
    }

    /// `AccountError.Frozen`, `AccountError.InvalidAmount(5)`.
    fn variant_member(&mut self, id: ChoiceId, name: &Name) -> Member {
        let Some(choice) = self.checked.choice(id) else {
            return Member::Unknown;
        };
        let owner = choice.name.clone();
        let at = choice.span;

        let Some((index, variant)) = choice.variant(&name.text) else {
            let known: Vec<String> = choice
                .variants
                .iter()
                .map(|variant| variant.name.clone())
                .collect();
            self.report(messages::unknown_variant(
                &owner, &name.text, name.span, &known,
            ));
            return Member::Unknown;
        };

        let resolution = Resolution::Variant {
            choice: id,
            variant: index,
        };
        if variant.fields.is_empty() {
            return Member::Value {
                declared: Type::named(&owner),
                resolution,
            };
        }

        let fields = variant.fields.clone();
        Member::Callable {
            signature: Signature::new(
                fields
                    .iter()
                    .map(|field| Parameter::new(field.name.clone(), field.declared.clone()))
                    .collect(),
                Type::named(&owner),
            ),
            resolution,
            shape: CallShape::function(format!("{owner}.{}", name.text), Some(at)),
        }
    }

    // -----------------------------------------------------------------------
    // Reaching into a value
    // -----------------------------------------------------------------------

    fn value_member(&mut self, found: &Type, name: &Name, target: Span) -> Member {
        let found = self.variables.resolve(found);
        if found.is_unknown() {
            return Member::Unknown;
        }

        match &found {
            Type::Named(owner) if owner == "Json" => self.json_value_member(name),
            Type::Maybe(inner) if matches!(inner.as_ref(), Type::Named(owner) if owner == "Json") => {
                self.json_value_member(name)
            }
            Type::Named(owner) => self.named_member(owner, &found, name),
            Type::Ability(ability) => self.ability_member(ability, &found, name),
            Type::Db => self.builtin_member(&found, name, target),
            Type::Store => self.store_member(name, target),
            Type::Logger => self.logger_member(name),
            Type::Socket => self.socket_member(name),
            Type::Hub(item) => self.hub_member(item, name),
            Type::Query { error } => self.query_member(error, name),
            _ => self.builtin_member(&found, name, target),
        }
    }

    /// Walking methods on a `Json` document, or on `maybe Json` so `.at` chains.
    fn json_value_member(&mut self, name: &Name) -> Member {
        let json = Type::named("Json");
        match name.text.as_str() {
            "at" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("key", Type::parameter("K"))],
                    Type::maybe(json),
                ),
                resolution: Resolution::BuiltinMethod("json_at"),
                shape: CallShape::function("Json.at", None),
            },
            "as_text" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::maybe(Type::Text)),
                resolution: Resolution::BuiltinMethod("json_as_text"),
                shape: CallShape::function("Json.as_text", None),
            },
            "as_int" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::maybe(Type::Int)),
                resolution: Resolution::BuiltinMethod("json_as_int"),
                shape: CallShape::function("Json.as_int", None),
            },
            "as_float" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::maybe(Type::Float)),
                resolution: Resolution::BuiltinMethod("json_as_float"),
                shape: CallShape::function("Json.as_float", None),
            },
            "as_bool" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::maybe(Type::Bool)),
                resolution: Resolution::BuiltinMethod("json_as_bool"),
                shape: CallShape::function("Json.as_bool", None),
            },
            "as_list" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::maybe(Type::list(json))),
                resolution: Resolution::BuiltinMethod("json_as_list"),
                shape: CallShape::function("Json.as_list", None),
            },
            "as_map" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::maybe(Type::map(Type::Text, json))),
                resolution: Resolution::BuiltinMethod("json_as_map"),
                shape: CallShape::function("Json.as_map", None),
            },
            _ => {
                self.report(messages::unknown_member(
                    &json,
                    &name.text,
                    name.span,
                    &[
                        "at".into(),
                        "as_text".into(),
                        "as_int".into(),
                        "as_float".into(),
                        "as_bool".into(),
                        "as_list".into(),
                        "as_map".into(),
                    ],
                ));
                Member::Unknown
            }
        }
    }

    /// A field, a method, or `.with`, on a value of a declared type.
    fn named_member(&mut self, owner: &str, found: &Type, name: &Name) -> Member {
        if let Some(Global::Cast(id)) = self.globals.get(owner).copied() {
            return self.cast_named_member(id, owner, found, name);
        }

        let Some(Global::Type(id)) = self.globals.get(owner).copied() else {
            // The only other named type is a choice, and a choice's values are its
            // variants: there is nothing inside one to reach for.
            self.report(messages::unknown_member(found, &name.text, name.span, &[]));
            return Member::Unknown;
        };
        let Some(declared) = self.checked.declared_type(id) else {
            return Member::Unknown;
        };
        let at = declared.span;

        if let Some((index, field)) = declared.field(&name.text) {
            return Member::Value {
                declared: field.declared.clone(),
                resolution: Resolution::Field {
                    declared: id,
                    field: index,
                },
            };
        }

        // `.with(field: value)` gives back a changed copy, which is how an
        // immutable type is "changed".
        if name.text == "with" {
            let fields = declared.fields.clone();
            return Member::Callable {
                signature: Signature::new(
                    fields
                        .iter()
                        .map(|field| Parameter {
                            name: field.name.clone(),
                            declared: field.declared.clone(),
                            optional: true,
                        })
                        .collect(),
                    Type::named(owner),
                ),
                resolution: Resolution::With(id),
                shape: CallShape::fields(
                    format!("{owner}.with"),
                    owner.to_string(),
                    at,
                    WhenMissing::Keep,
                ),
            };
        }

        if let Some(function) = self.checked.method(id, &name.text) {
            let Some(method) = self.checked.function(function) else {
                return Member::Unknown;
            };
            return Member::Callable {
                signature: method.signature.clone(),
                resolution: Resolution::Method {
                    declared: id,
                    function,
                },
                shape: CallShape::function(format!("{owner}.{}", name.text), Some(method.span)),
            };
        }

        let known = self.members_of_named(id);
        self.report(messages::unknown_member(
            found, &name.text, name.span, &known,
        ));
        Member::Unknown
    }

    /// A field or an instance method on a cast value. Casts have no `.with`.
    fn cast_named_member(&mut self, id: CastId, owner: &str, found: &Type, name: &Name) -> Member {
        let Some(declared) = self.checked.declared_cast(id) else {
            return Member::Unknown;
        };

        if let Some((index, field)) = declared.field(&name.text) {
            return Member::Value {
                declared: field.declared.clone(),
                resolution: Resolution::CastField {
                    cast: id,
                    field: index,
                },
            };
        }

        if let Some(function) = self.checked.cast_method(id, &name.text) {
            let Some(method) = self.checked.function(function) else {
                return Member::Unknown;
            };
            return Member::Callable {
                signature: method.signature.clone(),
                resolution: Resolution::CastMethod { cast: id, function },
                shape: CallShape::function(format!("{owner}.{}", name.text), Some(method.span)),
            };
        }

        let known = self.members_of_cast(id);
        self.report(messages::unknown_member(
            found, &name.text, name.span, &known,
        ));
        Member::Unknown
    }

    /// A required function, on a value known only by the ability it provides.
    fn ability_member(&mut self, ability: &str, found: &Type, name: &Name) -> Member {
        let Some(Global::Ability(id)) = self.globals.get(ability).copied() else {
            return Member::Unknown;
        };
        let Some(declared) = self.checked.ability(id) else {
            return Member::Unknown;
        };

        let Some((slot, required)) = declared.required(&name.text) else {
            let known: Vec<String> = declared
                .functions
                .iter()
                .map(|function| function.name.clone())
                .collect();
            self.report(messages::unknown_member(
                found, &name.text, name.span, &known,
            ));
            return Member::Unknown;
        };

        let at = required.span;
        Member::Callable {
            signature: required.signature.clone(),
            resolution: Resolution::AbilityMethod {
                ability: id,
                function: slot,
            },
            shape: CallShape::function(format!("{ability}.{}", name.text), Some(at)),
        }
    }

    /// Methods on an open `Store`. These use their own resolution names so they
    /// never collide with `.get` on a map.
    fn store_member(&mut self, name: &Name, _target: Span) -> Member {
        match name.text.as_str() {
            "get" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("key", Type::Text)],
                    Type::maybe(Type::Text),
                ),
                resolution: Resolution::BuiltinMethod("store_get"),
                shape: CallShape::function("store.get", None),
            },
            "set" => Member::Callable {
                signature: Signature::new(
                    vec![
                        Parameter::new("key", Type::Text),
                        Parameter::new("value", Type::Text),
                    ],
                    Type::fallible(Type::Int, Type::named("StoreError")),
                ),
                resolution: Resolution::BuiltinMethod("store_set"),
                shape: CallShape::function("store.set", None),
            },
            "remove" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("key", Type::Text)],
                    Type::fallible(Type::Int, Type::named("StoreError")),
                ),
                resolution: Resolution::BuiltinMethod("store_remove"),
                shape: CallShape::function("store.remove", None),
            },
            "keys" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("prefix", Type::Text)],
                    Type::fallible(Type::list(Type::Text), Type::named("StoreError")),
                ),
                resolution: Resolution::BuiltinMethod("store_keys"),
                shape: CallShape::function("store.keys", None),
            },
            "from" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("prefix", Type::Text)],
                    Type::query(Type::named("StoreError")),
                ),
                resolution: Resolution::BuiltinMethod("store_from"),
                shape: CallShape::function("store.from", None),
            },
            _ => {
                let known = vec![
                    "get".into(),
                    "set".into(),
                    "remove".into(),
                    "keys".into(),
                    "from".into(),
                ];
                self.report(messages::unknown_member(
                    &Type::Store,
                    &name.text,
                    name.span,
                    &known,
                ));
                Member::Unknown
            }
        }
    }

    /// Methods on a process-wide `Hub`.
    fn hub_member(&mut self, item: &Type, name: &Name) -> Member {
        match name.text.as_str() {
            "join" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("client", Type::Socket)],
                    Type::Nothing,
                ),
                resolution: Resolution::BuiltinMethod("hub_join"),
                shape: CallShape::function("hub.join", None),
            },
            "broadcast" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("value", item.clone())],
                    Type::Nothing,
                ),
                resolution: Resolution::BuiltinMethod("hub_broadcast"),
                shape: CallShape::function("hub.broadcast", None),
            },
            _ => {
                self.report(messages::unknown_member(
                    &Type::hub(item.clone()),
                    &name.text,
                    name.span,
                    &["join".into(), "broadcast".into()],
                ));
                Member::Unknown
            }
        }
    }

    /// Properties and methods on an open `Socket`.
    fn socket_member(&mut self, name: &Name) -> Member {
        match name.text.as_str() {
            "messages" => Member::Value {
                declared: Type::channel(Type::Text),
                resolution: Resolution::BuiltinMethod("socket_messages"),
            },
            "binary" => Member::Value {
                declared: Type::channel(Type::Bytes),
                resolution: Resolution::BuiltinMethod("socket_binary"),
            },
            "closed_because" => Member::Value {
                declared: Type::maybe(Type::Text),
                resolution: Resolution::BuiltinMethod("socket_closed_because"),
            },
            "send" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("text", Type::Text)],
                    Type::fallible(Type::Nothing, Type::named("SocketError")),
                ),
                resolution: Resolution::BuiltinMethod("socket_send"),
                shape: CallShape::function("socket.send", None),
            },
            "close" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::Nothing),
                resolution: Resolution::BuiltinMethod("socket_close"),
                shape: CallShape::function("socket.close", None),
            },
            _ => {
                self.report(messages::unknown_member(
                    &Type::Socket,
                    &name.text,
                    name.span,
                    &[
                        "messages".into(),
                        "binary".into(),
                        "closed_because".into(),
                        "send".into(),
                        "close".into(),
                    ],
                ));
                Member::Unknown
            }
        }
    }

    /// Methods on a configured `Logger`.
    fn logger_member(&mut self, name: &Name) -> Member {
        match name.text.as_str() {
            "set_level" => Member::Callable {
                signature: Signature::new(vec![Parameter::new("level", Type::Text)], Type::Nothing),
                resolution: Resolution::BuiltinMethod("logger_set_level"),
                shape: CallShape::function("logger.set_level", None),
            },
            "set_format" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("format", Type::Text)],
                    Type::Nothing,
                ),
                resolution: Resolution::BuiltinMethod("logger_set_format"),
                shape: CallShape::function("logger.set_format", None),
            },
            "debug" | "info" | "warn" | "error" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("message", Type::Text)],
                    Type::Nothing,
                ),
                resolution: Resolution::BuiltinMethod(match name.text.as_str() {
                    "debug" => "logger_debug",
                    "info" => "logger_info",
                    "warn" => "logger_warn",
                    _ => "logger_error",
                }),
                shape: CallShape::function(format!("logger.{}", name.text), None),
            },
            "write" => Member::Callable {
                signature: Signature::new(
                    vec![
                        Parameter::new("level", Type::Text),
                        Parameter::new("message", Type::Text),
                        Parameter::new("fields", Type::map(Type::Text, Type::Text)),
                    ],
                    Type::Nothing,
                ),
                resolution: Resolution::BuiltinMethod("logger_write"),
                shape: CallShape::function("logger.write", None),
            },
            "lines" => Member::Value {
                declared: Type::list(Type::Text),
                resolution: Resolution::BuiltinMethod("logger_lines"),
            },
            _ => {
                let known = vec![
                    "set_level".into(),
                    "set_format".into(),
                    "debug".into(),
                    "info".into(),
                    "warn".into(),
                    "error".into(),
                    "write".into(),
                    "lines".into(),
                ];
                self.report(messages::unknown_member(
                    &Type::Logger,
                    &name.text,
                    name.span,
                    &known,
                ));
                Member::Unknown
            }
        }
    }

    /// Fluent AREL-style methods on a `Query`.
    fn query_member(&mut self, error: &Type, name: &Name) -> Member {
        let query_type = Type::query(error.clone());
        let row = Type::map(Type::Text, Type::Text);
        let rows = Type::list(row.clone());
        match name.text.as_str() {
            "where_eq" | "equals" | "where_not" | "differs" | "where_gt" | "above"
            | "where_gte" | "at_least" | "where_lt" | "below" | "where_lte" | "at_most"
            | "where_like" | "like" => Member::Callable {
                signature: Signature::new(
                    vec![
                        Parameter::new("column", Type::Text),
                        Parameter::new("value", Type::Text),
                    ],
                    query_type.clone(),
                ),
                resolution: Resolution::BuiltinMethod(match name.text.as_str() {
                    "where_eq" | "equals" => "query_where_eq",
                    "where_not" | "differs" => "query_where_not",
                    "where_gt" | "above" => "query_where_gt",
                    "where_gte" | "at_least" => "query_where_gte",
                    "where_lt" | "below" => "query_where_lt",
                    "where_lte" | "at_most" => "query_where_lte",
                    _ => "query_where_like",
                }),
                shape: CallShape::function(format!("query.{}", name.text), None),
            },
            // `.where("owner").is("ada")` — column first, then the comparison.
            "where" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("column", Type::Text)],
                    query_type.clone(),
                ),
                resolution: Resolution::BuiltinMethod("query_where_column"),
                shape: CallShape::function("query.where", None),
            },
            "is" | "is_not" | "is_above" | "is_at_least" | "is_below" | "is_at_most"
            | "is_like" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("value", Type::Text)],
                    query_type.clone(),
                ),
                resolution: Resolution::BuiltinMethod(match name.text.as_str() {
                    "is" => "query_is",
                    "is_not" => "query_is_not",
                    "is_above" => "query_is_above",
                    "is_at_least" => "query_is_at_least",
                    "is_below" => "query_is_below",
                    "is_at_most" => "query_is_at_most",
                    _ => "query_is_like",
                }),
                shape: CallShape::function(format!("query.{}", name.text), None),
            },
            "order" | "order_desc" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("column", Type::Text)],
                    query_type.clone(),
                ),
                resolution: Resolution::BuiltinMethod(if name.text == "order" {
                    "query_order"
                } else {
                    "query_order_desc"
                }),
                shape: CallShape::function(format!("query.{}", name.text), None),
            },
            "limit" | "offset" => Member::Callable {
                signature: Signature::new(vec![Parameter::new("n", Type::Int)], query_type.clone()),
                resolution: Resolution::BuiltinMethod(if name.text == "limit" {
                    "query_limit"
                } else {
                    "query_offset"
                }),
                shape: CallShape::function(format!("query.{}", name.text), None),
            },
            "select" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new("columns", Type::list(Type::Text))],
                    query_type.clone(),
                ),
                resolution: Resolution::BuiltinMethod("query_select"),
                shape: CallShape::function("query.select", None),
            },
            "all" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::fallible(rows, error.clone())),
                resolution: Resolution::BuiltinMethod("query_all"),
                shape: CallShape::function("query.all", None),
            },
            "first" => Member::Callable {
                signature: Signature::new(
                    Vec::new(),
                    Type::fallible(Type::maybe(row), error.clone()),
                ),
                resolution: Resolution::BuiltinMethod("query_first"),
                shape: CallShape::function("query.first", None),
            },
            "count" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::fallible(Type::Int, error.clone())),
                resolution: Resolution::BuiltinMethod("query_count"),
                shape: CallShape::function("query.count", None),
            },
            "insert" | "update" => Member::Callable {
                signature: Signature::new(
                    vec![Parameter::new(
                        if name.text == "insert" {
                            "row"
                        } else {
                            "values"
                        },
                        Type::map(Type::Text, Type::Text),
                    )],
                    Type::fallible(Type::Int, error.clone()),
                ),
                resolution: Resolution::BuiltinMethod(if name.text == "insert" {
                    "query_insert"
                } else {
                    "query_update"
                }),
                shape: CallShape::function(format!("query.{}", name.text), None),
            },
            "delete" => Member::Callable {
                signature: Signature::new(Vec::new(), Type::fallible(Type::Int, error.clone())),
                resolution: Resolution::BuiltinMethod("query_delete"),
                shape: CallShape::function("query.delete", None),
            },
            _ => {
                let known = [
                    "where_eq",
                    "where_not",
                    "where_gt",
                    "where_gte",
                    "where_lt",
                    "where_lte",
                    "where_like",
                    "order",
                    "order_desc",
                    "limit",
                    "offset",
                    "select",
                    "all",
                    "first",
                    "count",
                    "insert",
                    "update",
                    "delete",
                ];
                self.report(messages::unknown_member(
                    &query_type,
                    &name.text,
                    name.span,
                    &known
                        .iter()
                        .map(|name| (*name).to_string())
                        .collect::<Vec<_>>(),
                ));
                Member::Unknown
            }
        }
    }

    /// A method from the prelude, such as `.map` or `.get`.
    fn builtin_member(&mut self, found: &Type, name: &Name, target: Span) -> Member {
        if matches!(found, Type::Db) && name.text == "transaction" {
            self.report(messages::transaction_needs_block(name.span));
            return Member::Unknown;
        }

        let mut mismatch: Option<Type> = None;
        for method in &self.builtin_methods {
            if method.name != name.text || !prelude::same_shape(&method.receiver, found) {
                continue;
            }

            // The receiver and the signature share their type parameters, so they have
            // to be given the same fresh variables: that is what carries the `T` of a
            // `list of T` into what `.map` gives back.
            let (receiver, signature) = self
                .variables
                .instantiate_together(&method.receiver, &method.signature);

            if !self.variables.fits(found, &receiver) {
                if mismatch.is_none() {
                    mismatch = Some(self.variables.resolve(&receiver));
                }
                continue;
            }

            let code = prelude::builtin_code(method.name, &method.receiver);
            if method.property {
                return Member::Value {
                    declared: signature.returns,
                    resolution: Resolution::BuiltinMethod(code),
                };
            }

            return Member::Callable {
                signature,
                resolution: Resolution::BuiltinMethod(code),
                shape: CallShape::function(method.name, None),
            };
        }

        if let Some(wanted) = mismatch {
            self.report(messages::bad_receiver(&name.text, &wanted, found, target));
            return Member::Unknown;
        }

        let known = prelude::members_of(found);
        if matches!(found, Type::Bytes) {
            self.report(messages::unknown_bytes_member(
                &name.text, name.span, &known,
            ));
        } else {
            self.report(messages::unknown_member(
                found, &name.text, name.span, &known,
            ));
        }
        Member::Unknown
    }

    /// Everything a value of a declared type offers, for a "did you mean".
    fn members_of_named(&self, id: TypeId) -> Vec<String> {
        let Some(declared) = self.checked.declared_type(id) else {
            return Vec::new();
        };
        let mut known: Vec<String> = declared
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();
        for method in &declared.methods {
            if let Some(function) = self.checked.function(*method) {
                known.push(function.name.clone());
            }
        }
        known.push("with".to_string());
        known
    }

    fn members_of_cast(&self, id: CastId) -> Vec<String> {
        let Some(declared) = self.checked.declared_cast(id) else {
            return Vec::new();
        };
        let mut known: Vec<String> = declared
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect();
        for method in &declared.methods {
            if let Some(function) = self.checked.function(*method) {
                if !function.class_method {
                    known.push(function.name.clone());
                }
            }
        }
        known
    }
}

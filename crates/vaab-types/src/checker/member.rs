//! `.field`, `.method()`, `Account.new(...)` and the rest of what follows a dot.
//!
//! A dot means one of two quite different things, depending on what is to its left.
//! `Account.new` reaches into a *declaration*; `account.owner` reaches into a
//! *value*. [`Checker::look_up_member`] decides which, and everything else works the
//! same either way.

use vaab_syntax::ast::{Expr, ExprKind, Name};
use vaab_syntax::span::Span;

use super::{Checker, Global, Wanted};
use crate::checked::{ChoiceId, Constructor, Resolution, TypeId};
use crate::messages;
use crate::prelude;
use crate::types::{Parameter, Signature, Type};

/// What a `.something` turned out to be.
pub(super) enum Member {
    /// Something to read: a field, a property such as `.is_empty`, or a variant with
    /// no payload.
    Value { declared: Type, resolution: Resolution },
    /// Something to call.
    Callable { signature: Signature, resolution: Resolution, shape: CallShape },
    /// `Channel.new` and `Shared.new`, which take a type rather than a value and so
    /// are checked on their own.
    Handle(Handle),
    /// Something already reported.
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle {
    Channel,
    Shared,
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
    fn fields(
        name: String,
        owner: String,
        declared: Span,
        missing: WhenMissing,
    ) -> Self {
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
            Member::Value { declared, resolution } => {
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
        // A name to the left of a dot may be a declaration rather than a value. A
        // local of the same name wins, so `let account = ...` is not shadowed by a
        // type called `account`.
        if let ExprKind::Name(written) = &target.kind {
            if self.lookup_local(&written.text).is_none() {
                match self.globals.get(&written.text).copied() {
                    Some(Global::Type(id)) => return self.type_member(id, name),
                    Some(Global::Choice(id)) => return self.variant_member(id, name),
                    Some(Global::Ability(_)) | None => {}
                }

                // The two built-in handles are constructed like a type even though
                // they are not one.
                if written.text == "Channel" && name.text == "new" {
                    return Member::Handle(Handle::Channel);
                }
                if written.text == "Shared" && name.text == "new" {
                    return Member::Handle(Handle::Shared);
                }
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
        let Some(declared) = self.checked.declared_type(id) else { return Member::Unknown };
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
                        resolution: Resolution::UserNew { declared: id, function },
                        shape: CallShape::function(format!("{owner}.new"), Some(found.span)),
                    }
                }
            },

            "raw" => {
                if self.inside != Some(id) {
                    self.report(messages::raw_outside_type(&owner, name.span));
                }
                Member::Callable {
                    // `raw` takes every field, defaults and all: it is the way to
                    // build a value without going through the checks in `to new`.
                    signature: Signature::new(
                        fields
                            .iter()
                            .map(|field| {
                                Parameter::new(field.name.clone(), field.declared.clone())
                            })
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

    /// `AccountError.Frozen`, `AccountError.InvalidAmount(5)`.
    fn variant_member(&mut self, id: ChoiceId, name: &Name) -> Member {
        let Some(choice) = self.checked.choice(id) else { return Member::Unknown };
        let owner = choice.name.clone();
        let at = choice.span;

        let Some((index, variant)) = choice.variant(&name.text) else {
            let known: Vec<String> =
                choice.variants.iter().map(|variant| variant.name.clone()).collect();
            self.report(messages::unknown_variant(&owner, &name.text, name.span, &known));
            return Member::Unknown;
        };

        let resolution = Resolution::Variant { choice: id, variant: index };
        if variant.fields.is_empty() {
            return Member::Value { declared: Type::named(&owner), resolution };
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
            Type::Named(owner) => self.named_member(owner, &found, name),
            Type::Ability(ability) => self.ability_member(ability, &found, name),
            _ => self.builtin_member(&found, name, target),
        }
    }

    /// A field, a method, or `.with`, on a value of a declared type.
    fn named_member(&mut self, owner: &str, found: &Type, name: &Name) -> Member {
        let Some(Global::Type(id)) = self.globals.get(owner).copied() else {
            // The only other named type is a choice, and a choice's values are its
            // variants: there is nothing inside one to reach for.
            self.report(messages::unknown_member(found, &name.text, name.span, &[]));
            return Member::Unknown;
        };
        let Some(declared) = self.checked.declared_type(id) else { return Member::Unknown };
        let at = declared.span;

        if let Some((index, field)) = declared.field(&name.text) {
            return Member::Value {
                declared: field.declared.clone(),
                resolution: Resolution::Field { declared: id, field: index },
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
            let Some(method) = self.checked.function(function) else { return Member::Unknown };
            return Member::Callable {
                signature: method.signature.clone(),
                resolution: Resolution::Method { declared: id, function },
                shape: CallShape::function(format!("{owner}.{}", name.text), Some(method.span)),
            };
        }

        let known = self.members_of_named(id);
        self.report(messages::unknown_member(found, &name.text, name.span, &known));
        Member::Unknown
    }

    /// A required function, on a value known only by the ability it provides.
    fn ability_member(&mut self, ability: &str, found: &Type, name: &Name) -> Member {
        let Some(Global::Ability(id)) = self.globals.get(ability).copied() else {
            return Member::Unknown;
        };
        let Some(declared) = self.checked.ability(id) else { return Member::Unknown };

        let Some((slot, required)) = declared.required(&name.text) else {
            let known: Vec<String> =
                declared.functions.iter().map(|function| function.name.clone()).collect();
            self.report(messages::unknown_member(found, &name.text, name.span, &known));
            return Member::Unknown;
        };

        let at = required.span;
        Member::Callable {
            signature: required.signature.clone(),
            resolution: Resolution::AbilityMethod { ability: id, function: slot },
            shape: CallShape::function(format!("{ability}.{}", name.text), Some(at)),
        }
    }

    /// A method from the prelude, such as `.map` or `.get`.
    fn builtin_member(&mut self, found: &Type, name: &Name, target: Span) -> Member {
        let candidate = self
            .builtin_methods
            .iter()
            .find(|method| {
                method.name == name.text && prelude::same_shape(&method.receiver, found)
            })
            .map(|method| (method.name, method.receiver.clone(), method.signature.clone(), method.property));

        let Some((name_of, receiver, signature, property)) = candidate else {
            let known = prelude::members_of(found);
            self.report(messages::unknown_member(found, &name.text, name.span, &known));
            return Member::Unknown;
        };

        // The receiver and the signature share their type parameters, so they have to
        // be given the same fresh variables: that is what carries the `T` of a
        // `list of T` into what `.map` gives back.
        let (receiver, signature) = self.variables.instantiate_together(&receiver, &signature);

        if !self.variables.fits(found, &receiver) {
            let receiver = self.variables.resolve(&receiver);
            self.report(messages::bad_receiver(&name.text, &receiver, found, target));
            return Member::Unknown;
        }

        if property {
            return Member::Value {
                declared: signature.returns,
                resolution: Resolution::BuiltinMethod(name_of),
            };
        }

        Member::Callable {
            signature,
            resolution: Resolution::BuiltinMethod(name_of),
            shape: CallShape::function(name_of, None),
        }
    }

    /// Everything a value of a declared type offers, for a "did you mean".
    fn members_of_named(&self, id: TypeId) -> Vec<String> {
        let Some(declared) = self.checked.declared_type(id) else { return Vec::new() };
        let mut known: Vec<String> =
            declared.fields.iter().map(|field| field.name.clone()).collect();
        for method in &declared.methods {
            if let Some(function) = self.checked.function(*method) {
                known.push(function.name.clone());
            }
        }
        known.push("with".to_string());
        known
    }
}

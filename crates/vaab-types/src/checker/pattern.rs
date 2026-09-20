//! Patterns: what a `match` arm or a `for each` takes apart.
//!
//! A pattern is checked *against* a type rather than having one of its own, so this
//! is the one place the checker works downwards only. Along the way it declares the
//! locals a pattern binds and records which variant a variant pattern names, which
//! is what [`super::exhaustive`] reads back when it works out what was covered.

use vaab_syntax::ast::{Name, Pattern, PatternKind};
use vaab_syntax::span::Span;

use super::{Checker, Global};
use crate::checked::Resolution;
use crate::messages;
use crate::types::Type;

/// How to describe the shape a pattern is for, when it meets a type it cannot match.
const MAYBE: &str = "a `maybe` value";
const FALLIBLE: &str = "a result that can fail";

impl Checker {
    /// Checks `pattern` against the type of the value it will be matched with, and
    /// declares whatever it binds in the current scope.
    pub(super) fn pattern(&mut self, pattern: &Pattern, subject: &Type) {
        let subject = self.variables.resolve(subject);

        match &pattern.kind {
            PatternKind::Binding(name) => {
                // A binding accepts anything, so there is nothing to check: it is
                // the pattern that gives a name to whatever arrived.
                let local = self.declare_local(name, subject, false);
                self.checked.bindings.insert(pattern.id, local);
            }

            PatternKind::Int(_) => self.literal_pattern(Type::Int, &subject, pattern.span),
            PatternKind::Float(_) => self.literal_pattern(Type::Float, &subject, pattern.span),
            PatternKind::Bool(_) => self.literal_pattern(Type::Bool, &subject, pattern.span),
            PatternKind::Text(_) => self.literal_pattern(Type::Text, &subject, pattern.span),

            PatternKind::Nothing => match &subject {
                Type::Maybe(_) | Type::Unknown => {}
                found => {
                    self.report(messages::pattern_mismatch(found, MAYBE, pattern.span));
                }
            },

            PatternKind::Found(inner) => match &subject {
                Type::Maybe(held) => {
                    let held = held.as_ref().clone();
                    self.pattern(inner, &held);
                }
                Type::Unknown => self.pattern(inner, &Type::Unknown),
                found => {
                    self.report(messages::pattern_mismatch(found, MAYBE, pattern.span));
                    self.pattern(inner, &Type::Unknown);
                }
            },

            PatternKind::Success(inner) => match &subject {
                Type::Fallible { ok, .. } => {
                    let ok = ok.as_ref().clone();
                    self.pattern(inner, &ok);
                }
                Type::Unknown => self.pattern(inner, &Type::Unknown),
                found => {
                    self.report(messages::pattern_mismatch(found, FALLIBLE, pattern.span));
                    self.pattern(inner, &Type::Unknown);
                }
            },

            PatternKind::Failure(inner) => match &subject {
                Type::Fallible { error, .. } => {
                    let error = error.as_ref().clone();
                    self.pattern(inner, &error);
                }
                Type::Unknown => self.pattern(inner, &Type::Unknown),
                found => {
                    self.report(messages::pattern_mismatch(found, FALLIBLE, pattern.span));
                    self.pattern(inner, &Type::Unknown);
                }
            },

            PatternKind::Variant { path, fields } => {
                self.variant_pattern(pattern, path, fields, &subject);
            }

            PatternKind::List { elements, .. } => {
                let item = match &subject {
                    Type::List(item) => item.as_ref().clone(),
                    Type::Unknown => Type::Unknown,
                    found => {
                        self.report(messages::pattern_mismatch(found, "a list", pattern.span));
                        Type::Unknown
                    }
                };
                for element in elements {
                    self.pattern(element, &item);
                }
            }

            PatternKind::Tuple(parts) => {
                let held = match &subject {
                    Type::Tuple(held) if held.len() == parts.len() => held.clone(),
                    Type::Unknown => vec![Type::Unknown; parts.len()],
                    found => {
                        self.report(messages::pattern_mismatch(
                            found,
                            &format!("a group of {}", parts.len()),
                            pattern.span,
                        ));
                        vec![Type::Unknown; parts.len()]
                    }
                };
                for (part, declared) in parts.iter().zip(held) {
                    self.pattern(part, &declared);
                }
            }
        }
    }

    /// A literal pattern matches one value, so it only has to be the right type.
    fn literal_pattern(&mut self, written: Type, subject: &Type, span: Span) {
        if subject.is_unknown() || self.variables.fits(&written, subject) {
            return;
        }
        self.report(messages::mismatch(subject, &written, span));
    }

    /// `AccountError.InvalidAmount(amount)`, or a bare `Frozen` when the subject's
    /// choice is already known.
    fn variant_pattern(
        &mut self,
        pattern: &Pattern,
        path: &[Name],
        fields: &[Pattern],
        subject: &Type,
    ) {
        let Some(last) = path.last() else { return };

        // Two names spell the choice out; one leans on the subject to say which
        // choice is being taken apart.
        let choice = match path.len() {
            1 => match subject {
                Type::Named(name) => match self.globals.get(name).copied() {
                    Some(Global::Choice(id)) => Some(id),
                    _ => None,
                },
                _ => None,
            },
            _ => match path.first().and_then(|first| self.globals.get(&first.text).copied()) {
                Some(Global::Choice(id)) => Some(id),
                _ => None,
            },
        };

        let Some(choice) = choice else {
            if !subject.is_unknown() {
                if path.len() == 1 {
                    self.report(messages::pattern_mismatch(
                        subject,
                        "a choice variant",
                        pattern.span,
                    ));
                } else if let Some(first) = path.first() {
                    let known = self.choice_names();
                    self.report(messages::not_a_choice(&first.text, first.span, &known));
                }
            }
            self.unknown_fields(fields);
            return;
        };

        // Everything the checked choice has to say, taken out before reporting
        // anything, so that the tables are not borrowed while diagnostics are added.
        let Some(declared) = self.checked.choice(choice) else { return };
        let owner = declared.name.clone();
        let known: Vec<String> =
            declared.variants.iter().map(|variant| variant.name.clone()).collect();
        let named = declared.variant(&last.text).map(|(index, variant)| {
            let carried: Vec<Type> =
                variant.fields.iter().map(|field| field.declared.clone()).collect();
            (index, carried)
        });

        // A pattern naming one choice cannot match a value of another.
        if let Type::Named(name) = subject {
            if name != &owner {
                self.report(messages::pattern_mismatch(
                    subject,
                    &format!("a `{owner}`"),
                    pattern.span,
                ));
            }
        }

        let Some((index, carried)) = named else {
            self.report(messages::unknown_variant(&owner, &last.text, last.span, &known));
            self.unknown_fields(fields);
            return;
        };

        self.resolve_to(pattern.id, Resolution::Variant { choice, variant: index });

        if carried.len() != fields.len() {
            self.report(messages::wrong_variant_parts(
                &format!("{owner}.{}", last.text),
                carried.len(),
                fields.len(),
                pattern.span,
            ));
        }

        // Checking as far as the two agree still names whatever the arm binds, so
        // the body is worth checking afterwards either way.
        for (position, field) in fields.iter().enumerate() {
            let declared = carried.get(position).cloned().unwrap_or(Type::Unknown);
            self.pattern(field, &declared);
        }
    }

    /// Binds what a pattern names even though its type is no longer known, so that
    /// its arm's body does not then complain about undefined names.
    fn unknown_fields(&mut self, fields: &[Pattern]) {
        for field in fields {
            self.pattern(field, &Type::Unknown);
        }
    }

    fn choice_names(&self) -> Vec<String> {
        self.checked.choices.iter().map(|choice| choice.name.clone()).collect()
    }
}

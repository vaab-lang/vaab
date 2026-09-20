//! Whether a `match` covers every case.
//!
//! The rule is deliberately plain: a `match` on something with a known, finite set
//! of cases — a `maybe`, a fallible result, a `Bool`, a `choice` — has to name every
//! one, and a `match` on anything else has to finish with `otherwise`. Nothing here
//! reasons about ranges or about lists of a particular length; a pattern that only
//! matches *some* values of its case simply does not count towards covering it.
//!
//! This runs after the arms have been checked, so it reads the variant each pattern
//! resolved to rather than working it out a second time.

use vaab_syntax::ast::{ArmPattern, MatchArm, Pattern, PatternKind};
use vaab_syntax::span::Span;

use super::{Checker, Global};
use crate::checked::Resolution;
use crate::messages;
use crate::types::Type;

/// What one arm covers, as far as exhaustiveness is concerned.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Covers {
    /// Every remaining value: a bare binding, or `otherwise`.
    Everything,
    /// `nothing`, or `found` with a pattern inside that itself covers everything.
    Absent,
    Present,
    Success,
    Failure,
    Bool(bool),
    Variant(usize),
    /// A pattern that matches only some values of its case, such as `found 0`.
    Partly,
}

impl Checker {
    pub(super) fn check_exhaustive(&mut self, subject: &Type, arms: &[MatchArm], span: Span) {
        // A `match` on something already wrong has nothing dependable to cover.
        if subject.is_unknown() {
            return;
        }

        let mut covered = Vec::new();
        for arm in arms {
            // A guard may not run, so an arm that has one covers nothing on its own.
            if arm.guard.is_some() {
                continue;
            }
            covered.push(match &arm.pattern {
                ArmPattern::Otherwise => Covers::Everything,
                ArmPattern::Pattern(pattern) => self.covers(pattern),
            });
        }

        if covered.contains(&Covers::Everything) {
            return;
        }

        let uncovered = match subject {
            // The names are spelled as an arm would be, so that the help a
            // programmer is shown can be typed straight in.
            Type::Maybe(_) => {
                let mut missing = Vec::new();
                if !covered.contains(&Covers::Present) {
                    missing.push("found value".to_string());
                }
                if !covered.contains(&Covers::Absent) {
                    missing.push("nothing".to_string());
                }
                missing
            }

            Type::Fallible { .. } => {
                let mut missing = Vec::new();
                if !covered.contains(&Covers::Success) {
                    missing.push("success value".to_string());
                }
                if !covered.contains(&Covers::Failure) {
                    missing.push("failure error".to_string());
                }
                missing
            }

            Type::Bool => [(true, "yes"), (false, "no")]
                .into_iter()
                .filter(|(value, _)| !covered.contains(&Covers::Bool(*value)))
                .map(|(_, spelling)| spelling.to_string())
                .collect(),

            Type::Named(name) => match self.globals.get(name).copied() {
                Some(Global::Choice(id)) => {
                    let Some(choice) = self.checked.choice(id) else { return };
                    choice
                        .variants
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !covered.contains(&Covers::Variant(*index)))
                        .map(|(_, variant)| format!("{name}.{}", variant.name))
                        .collect()
                }
                // A declared `type` has as many values as its fields do, so the only
                // way to cover one is to bind it or finish with `otherwise`.
                _ => {
                    self.report(messages::not_exhaustive_open(subject, span));
                    return;
                }
            },

            // Ints, text, lists and the rest are open-ended.
            _ => {
                self.report(messages::not_exhaustive_open(subject, span));
                return;
            }
        };

        if !uncovered.is_empty() {
            self.report(messages::not_exhaustive(&uncovered, subject, span));
        }
    }

    fn covers(&self, pattern: &Pattern) -> Covers {
        match &pattern.kind {
            PatternKind::Binding(_) => Covers::Everything,
            PatternKind::Nothing => Covers::Absent,
            PatternKind::Bool(value) => Covers::Bool(*value),

            PatternKind::Found(inner) => self.wrapping(Covers::Present, inner),
            PatternKind::Success(inner) => self.wrapping(Covers::Success, inner),
            PatternKind::Failure(inner) => self.wrapping(Covers::Failure, inner),

            PatternKind::Variant { fields, .. } => {
                match self.checked.resolution(pattern.id) {
                    Some(Resolution::Variant { variant, .. }) => {
                        if fields.iter().all(|field| self.covers(field) == Covers::Everything) {
                            Covers::Variant(*variant)
                        } else {
                            Covers::Partly
                        }
                    }
                    // The pattern did not resolve, so something is already reported.
                    _ => Covers::Everything,
                }
            }

            // `[...]` alone accepts any list; anything more asks for a length.
            PatternKind::List { elements, rest } => {
                if *rest && elements.is_empty() {
                    Covers::Everything
                } else {
                    Covers::Partly
                }
            }

            PatternKind::Tuple(parts) => {
                if parts.iter().all(|part| self.covers(part) == Covers::Everything) {
                    Covers::Everything
                } else {
                    Covers::Partly
                }
            }

            PatternKind::Int(_) | PatternKind::Float(_) | PatternKind::Text(_) => Covers::Partly,
        }
    }

    /// `found`, `success` and `failure` cover their half only when what they hold
    /// covers everything: `found x` does, `found 0` does not.
    fn wrapping(&self, whole: Covers, inner: &Pattern) -> Covers {
        if self.covers(inner) == Covers::Everything {
            whole
        } else {
            Covers::Partly
        }
    }
}

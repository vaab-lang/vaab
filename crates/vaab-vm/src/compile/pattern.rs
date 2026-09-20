//! Patterns: testing a value and taking it apart.
//!
//! A pattern compiles to two woven-together things: tests that jump away when
//! they do not hold, and stores that give names to the pieces. The value being
//! matched sits in a slot the compiler owns rather than on the stack, because a
//! pattern looks at it several times over and nested patterns need their own.

use vaab_syntax::ast::{Pattern, PatternKind};
use vaab_types::Resolution;

use super::Compiler;
use crate::bytecode::Op;
use crate::value::Value;

impl<'a> Compiler<'a> {
    /// Matches `pattern` against the value in slot `subject`.
    ///
    /// Every way the pattern can fail adds a jump to `failed`, which the caller
    /// points at whatever comes next: the following `match` arm, or the next turn
    /// of a `for each`.
    pub(super) fn take_apart(&mut self, pattern: &'a Pattern, subject: u32, failed: &mut Vec<u32>) {
        let span = pattern.span;

        match &pattern.kind {
            PatternKind::Binding(_) => {
                self.emit(Op::LoadLocal(subject), span);
                match self.binding_at(pattern.id) {
                    Some(local) => self.declare(local, span),
                    None => self.emit(Op::Pop, span),
                }
            }

            PatternKind::Int(number) => {
                self.emit(Op::LoadLocal(subject), span);
                self.emit(Op::Int(*number), span);
                self.equal_or_fail(failed, span);
            }
            PatternKind::Float(number) => {
                self.emit(Op::LoadLocal(subject), span);
                self.push_constant(Value::Float(*number), span);
                self.equal_or_fail(failed, span);
            }
            PatternKind::Bool(held) => {
                self.emit(Op::LoadLocal(subject), span);
                self.emit(Op::Bool(*held), span);
                self.equal_or_fail(failed, span);
            }
            PatternKind::Text(text) => {
                self.emit(Op::LoadLocal(subject), span);
                self.push_constant(Value::text(text), span);
                self.equal_or_fail(failed, span);
            }

            PatternKind::Nothing => {
                self.emit(Op::LoadLocal(subject), span);
                self.emit(Op::IsFound, span);
                failed.push(self.jump(Op::JumpIfTrue(0), span));
            }

            PatternKind::Found(inner) => self.unwrap_into(inner, subject, Op::IsFound, failed),
            PatternKind::Success(inner) => {
                self.unwrap_into(inner, subject, Op::IsSuccess, failed)
            }
            PatternKind::Failure(inner) => {
                self.unwrap_into(inner, subject, Op::IsFailure, failed)
            }

            PatternKind::Variant { fields, .. } => {
                let named = match self.checked.resolution(pattern.id).cloned() {
                    Some(Resolution::Variant { choice, variant }) => {
                        self.variant_of.get(&(choice.index(), variant)).copied()
                    }
                    _ => None,
                };
                let Some(layout) = named else { return };

                self.emit(Op::LoadLocal(subject), span);
                self.emit(Op::IsVariant(layout), span);
                failed.push(self.jump(Op::JumpIfFalse(0), span));

                for (position, field) in fields.iter().enumerate() {
                    let held = self.temporary();
                    self.emit(Op::LoadLocal(subject), span);
                    self.emit(Op::VariantField(position as u32), span);
                    self.emit(Op::StoreLocal(held), span);
                    self.take_apart(field, held, failed);
                }
            }

            PatternKind::List { elements, rest } => {
                self.emit(Op::LoadLocal(subject), span);
                self.emit(
                    Op::ListLengthIs { length: elements.len() as u32, at_least: *rest },
                    span,
                );
                failed.push(self.jump(Op::JumpIfFalse(0), span));

                for (position, element) in elements.iter().enumerate() {
                    let held = self.temporary();
                    self.emit(Op::LoadLocal(subject), span);
                    self.emit(Op::Int(position as i64), span);
                    self.emit(Op::Index, span);
                    self.emit(Op::StoreLocal(held), span);
                    self.take_apart(element, held, failed);
                }
            }

            PatternKind::Tuple(parts) => {
                // A tuple's size is part of its type, so there is nothing to test.
                for (position, part) in parts.iter().enumerate() {
                    let held = self.temporary();
                    self.emit(Op::LoadLocal(subject), span);
                    self.emit(Op::TupleItem(position as u32), span);
                    self.emit(Op::StoreLocal(held), span);
                    self.take_apart(part, held, failed);
                }
            }
        }
    }

    fn equal_or_fail(&mut self, failed: &mut Vec<u32>, span: vaab_syntax::span::Span) {
        self.emit(Op::Equal, span);
        failed.push(self.jump(Op::JumpIfFalse(0), span));
    }

    /// `found x`, `success x` and `failure e`: one test, then the value inside.
    fn unwrap_into(
        &mut self,
        inner: &'a Pattern,
        subject: u32,
        test: Op,
        failed: &mut Vec<u32>,
    ) {
        let span = inner.span;
        self.emit(Op::LoadLocal(subject), span);
        self.emit(test, span);
        failed.push(self.jump(Op::JumpIfFalse(0), span));

        let held = self.temporary();
        self.emit(Op::LoadLocal(subject), span);
        self.emit(Op::Unwrap, span);
        self.emit(Op::StoreLocal(held), span);
        self.take_apart(inner, held, failed);
    }
}

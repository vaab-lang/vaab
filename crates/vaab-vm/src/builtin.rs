//! The standard library, as far as this phase builds it.
//!
//! Every builtin here has a signature in the type checker's prelude, and the two
//! tables have to agree: a name in one and not the other is either a function
//! nobody can call or a call with nothing behind it. `tests/library.rs` calls
//! every one of them from Vaab, which is what keeps the two in step.
//!
//! `.map` and `.each` are the exception. They take a closure and have to call it,
//! and a Vaab call is a frame pushed onto the machine — never a Rust recursion —
//! so the compiler turns those two into loops of ordinary instructions instead of
//! calling in here. See `compile::expr`.

use indexmap::IndexMap;

use crate::error::{Fault, Operation};
use crate::value::{Key, Value};

/// One function or method with a native implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    Print,

    Upper,
    Lower,
    Contains,

    Join,
    First,

    Get,
    Keys,

    Abs,
    Min,
    Max,
    ToFloat,
    Round,
}

impl Builtin {
    /// The name Vaab calls it by, which is also the name in the checker's prelude.
    pub fn name(self) -> &'static str {
        match self {
            Builtin::Print => "print",
            Builtin::Upper => "upper",
            Builtin::Lower => "lower",
            Builtin::Contains => "contains",
            Builtin::Join => "join",
            Builtin::First => "first",
            Builtin::Get => "get",
            Builtin::Keys => "keys",
            Builtin::Abs => "abs",
            Builtin::Min => "min",
            Builtin::Max => "max",
            Builtin::ToFloat => "to_float",
            Builtin::Round => "round",
        }
    }

    /// How many values it takes off the stack, counting the receiver of a method.
    pub fn arity(self) -> usize {
        match self {
            Builtin::Print
            | Builtin::Upper
            | Builtin::Lower
            | Builtin::First
            | Builtin::Keys
            | Builtin::Abs
            | Builtin::ToFloat
            | Builtin::Round => 1,
            Builtin::Contains | Builtin::Join | Builtin::Get | Builtin::Min | Builtin::Max => 2,
        }
    }

    /// Runs it. `print` is not here: it is the one builtin that needs somewhere to
    /// write to, so the machine handles it.
    pub fn apply(self, arguments: &[Value]) -> Result<Value, Fault> {
        match (self, arguments) {
            (Builtin::Upper, [Value::Text(text)]) => Ok(Value::text(text.to_uppercase())),
            (Builtin::Lower, [Value::Text(text)]) => Ok(Value::text(text.to_lowercase())),
            (Builtin::Contains, [Value::Text(text), Value::Text(part)]) => {
                Ok(Value::Bool(text.contains(part.as_ref())))
            }

            (Builtin::Join, [Value::List(items), Value::Text(separator)]) => {
                let pieces: Vec<String> = items.iter().map(Value::show).collect();
                Ok(Value::text(pieces.join(separator)))
            }
            (Builtin::First, [Value::List(items)]) => match items.first() {
                Some(first) => Ok(Value::found(first.clone())),
                None => Ok(Value::absent()),
            },

            (Builtin::Get, [Value::Map(entries), key]) => {
                match entries.get(&Key(key.clone())) {
                    Some(found) => Ok(Value::found(found.clone())),
                    None => Ok(Value::absent()),
                }
            }
            (Builtin::Keys, [Value::Map(entries)]) => {
                Ok(Value::list(entries.keys().map(|key| key.0.clone()).collect()))
            }

            (Builtin::Abs, [Value::Int(number)]) => match number.checked_abs() {
                Some(size) => Ok(Value::Int(size)),
                None => Err(Fault::Overflowed(Operation::Size)),
            },
            (Builtin::Abs, [Value::Float(number)]) => Ok(Value::Float(number.abs())),
            (Builtin::Min, [Value::Int(left), Value::Int(right)]) => {
                Ok(Value::Int(*left.min(right)))
            }
            (Builtin::Max, [Value::Int(left), Value::Int(right)]) => {
                Ok(Value::Int(*left.max(right)))
            }
            (Builtin::ToFloat, [Value::Int(number)]) => Ok(Value::Float(*number as f64)),
            (Builtin::Round, [Value::Float(number)]) => {
                let rounded = number.round();
                // `as` would clamp silently, and a number too big to be a whole one
                // is exactly the sort of thing that should stop the program.
                if rounded.is_finite() && (MIN_WHOLE..=MAX_WHOLE).contains(&rounded) {
                    Ok(Value::Int(rounded as i64))
                } else {
                    Err(Fault::Overflowed(Operation::Rounding))
                }
            }

            // Every builtin is reached through a checked call, so a value of the
            // wrong shape here means the compiler and the checker have fallen out.
            _ => Err(Fault::Confused("a builtin was given a value of the wrong shape")),
        }
    }
}

/// The widest whole numbers a `Float` can be turned into without losing the range.
const MIN_WHOLE: f64 = i64::MIN as f64;
const MAX_WHOLE: f64 = i64::MAX as f64;

/// Builds a map from keys and values that were pushed one pair after another.
pub fn table(pairs: Vec<Value>) -> Value {
    let mut entries = IndexMap::with_capacity(pairs.len() / 2);
    let mut pairs = pairs.into_iter();
    while let (Some(key), Some(value)) = (pairs.next(), pairs.next()) {
        entries.insert(Key(key), value);
    }
    Value::map(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_changes_case_without_changing_anything_else() {
        let shouted = Builtin::Upper.apply(&[Value::text("ada")]);
        assert!(matches!(shouted, Ok(Value::Text(text)) if text.as_ref() == "ADA"));
    }

    #[test]
    fn the_first_of_an_empty_list_is_nothing() {
        let first = Builtin::First.apply(&[Value::list(Vec::new())]);
        assert!(matches!(first, Ok(Value::Maybe(None))));
    }

    #[test]
    fn the_size_of_the_smallest_int_does_not_fit() {
        let size = Builtin::Abs.apply(&[Value::Int(i64::MIN)]);
        assert!(matches!(size, Err(Fault::Overflowed(Operation::Size))));
    }

    #[test]
    fn rounding_a_float_too_large_to_be_a_whole_number_is_an_error() {
        let rounded = Builtin::Round.apply(&[Value::Float(1e30)]);
        assert!(matches!(rounded, Err(Fault::Overflowed(Operation::Rounding))));
    }

    #[test]
    fn a_builtin_given_the_wrong_shape_reports_rather_than_panics() {
        let confused = Builtin::Upper.apply(&[Value::Int(1)]);
        assert!(matches!(confused, Err(Fault::Confused(_))));
    }
}

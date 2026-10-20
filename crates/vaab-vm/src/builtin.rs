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
    ToJson,
    BasicAuthOk,

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
    Sqrt,
    Ln,
    Exp,
    Pow,
    Floor,
    Ceil,
    IsFinite,
    ToInt,
    DividedBy,
    SumInt,
    SumFloat,
    ListMin,
    ListMax,
    ListMean,
    ListSorted,
}

impl Builtin {
    /// The name Vaab calls it by, which is also the name in the checker's prelude.
    pub fn name(self) -> &'static str {
        match self {
            Builtin::Print => "print",
            Builtin::ToJson => "to_json",
            Builtin::BasicAuthOk => "basic_auth_ok",
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
            Builtin::Sqrt => "sqrt",
            Builtin::Ln => "ln",
            Builtin::Exp => "exp",
            Builtin::Pow => "pow",
            Builtin::Floor => "floor",
            Builtin::Ceil => "ceil",
            Builtin::IsFinite => "is_finite",
            Builtin::ToInt => "to_int",
            Builtin::DividedBy => "divided_by",
            Builtin::SumInt => "sum",
            Builtin::SumFloat => "sum",
            Builtin::ListMin => "min",
            Builtin::ListMax => "max",
            Builtin::ListMean => "mean",
            Builtin::ListSorted => "sorted",
        }
    }

    /// How many values it takes off the stack, counting the receiver of a method.
    pub fn arity(self) -> usize {
        match self {
            Builtin::Print
            | Builtin::ToJson
            | Builtin::Upper
            | Builtin::Lower
            | Builtin::First
            | Builtin::Keys
            | Builtin::Abs
            | Builtin::ToFloat
            | Builtin::Round
            | Builtin::Sqrt
            | Builtin::Ln
            | Builtin::Exp
            | Builtin::Floor
            | Builtin::Ceil
            | Builtin::IsFinite
            | Builtin::ToInt
            | Builtin::SumInt
            | Builtin::SumFloat
            | Builtin::ListMin
            | Builtin::ListMax
            | Builtin::ListMean
            | Builtin::ListSorted => 1,
            Builtin::Contains
            | Builtin::Join
            | Builtin::Get
            | Builtin::Min
            | Builtin::Max
            | Builtin::Pow
            | Builtin::DividedBy => 2,
            Builtin::BasicAuthOk => 3,
        }
    }

    /// Runs it. `print` is not here: it is the one builtin that needs somewhere to
    /// write to, so the machine handles it.
    pub fn apply(self, arguments: &[Value]) -> Result<Value, Fault> {
        match (self, arguments) {
            (Builtin::ToJson, [value]) => crate::json::encode(value).map(Value::text),
            (
                Builtin::BasicAuthOk,
                [Value::Text(header), Value::Text(username), Value::Text(password)],
            ) => Ok(Value::Bool(crate::app_io::basic_auth_ok(
                header,
                username,
                password,
            ))),
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
            (Builtin::Min, [Value::Float(left), Value::Float(right)]) => {
                Ok(Value::Float(left.min(*right)))
            }
            (Builtin::Max, [Value::Float(left), Value::Float(right)]) => {
                Ok(Value::Float(left.max(*right)))
            }
            (Builtin::ToFloat, [Value::Int(number)]) => Ok(Value::Float(*number as f64)),
            (Builtin::Round, [Value::Float(number)]) => float_to_int(number.round()),
            (Builtin::Sqrt, [Value::Float(number)]) => {
                if *number < 0.0 {
                    return Err(Fault::NotARealNumber);
                }
                Ok(Value::Float(number.sqrt()))
            }
            (Builtin::Ln, [Value::Float(number)]) => Ok(Value::Float(number.ln())),
            (Builtin::Exp, [Value::Float(number)]) => Ok(Value::Float(number.exp())),
            (Builtin::Pow, [Value::Float(base), Value::Float(exponent)]) => {
                Ok(Value::Float(base.powf(*exponent)))
            }
            (Builtin::Floor, [Value::Float(number)]) => Ok(Value::Float(number.floor())),
            (Builtin::Ceil, [Value::Float(number)]) => Ok(Value::Float(number.ceil())),
            (Builtin::IsFinite, [Value::Float(number)]) => Ok(Value::Bool(number.is_finite())),
            (Builtin::ToInt, [Value::Float(number)]) => float_to_int(number.trunc()),
            (Builtin::DividedBy, [Value::Float(left), Value::Float(right)]) => {
                if *right == 0.0 || !right.is_finite() {
                    Ok(Value::absent())
                } else {
                    Ok(Value::found(Value::Float(left / right)))
                }
            }
            (Builtin::SumInt, [Value::List(items)]) => sum_ints(items),
            (Builtin::SumFloat, [Value::List(items)]) => Ok(Value::Float(sum_floats(items))),
            (Builtin::ListMin, [Value::List(items)]) => list_min(items),
            (Builtin::ListMax, [Value::List(items)]) => list_max(items),
            (Builtin::ListMean, [Value::List(items)]) => list_mean(items),
            (Builtin::ListSorted, [Value::List(items)]) => Ok(Value::list(sorted_list(items))),

            // Every builtin is reached through a checked call, so a value of the
            // wrong shape here means the compiler and the checker have fallen out.
            _ => Err(Fault::Confused("a builtin was given a value of the wrong shape")),
        }
    }
}

/// The widest whole numbers a `Float` can be turned into without losing the range.
const MIN_WHOLE: f64 = i64::MIN as f64;
const MAX_WHOLE: f64 = i64::MAX as f64;

fn float_to_int(number: f64) -> Result<Value, Fault> {
    // `as` would clamp silently, and a number too big to be a whole one
    // is exactly the sort of thing that should stop the program.
    if number.is_finite() && (MIN_WHOLE..=MAX_WHOLE).contains(&number) {
        Ok(Value::Int(number as i64))
    } else {
        Err(Fault::Overflowed(Operation::Rounding))
    }
}

fn sum_ints(items: &[Value]) -> Result<Value, Fault> {
    let mut total = 0i64;
    for item in items {
        let Value::Int(number) = item else {
            return Err(Fault::Confused("a list sum was given a value that is not a whole number"));
        };
        total = total
            .checked_add(*number)
            .ok_or(Fault::Overflowed(Operation::Sum))?;
    }
    Ok(Value::Int(total))
}

fn sum_floats(items: &[Value]) -> f64 {
    items.iter().fold(0.0, |total, item| match item {
        Value::Float(number) => total + number,
        Value::Int(number) => total + *number as f64,
        _ => total,
    })
}

fn list_min(items: &[Value]) -> Result<Value, Fault> {
    let Some(first) = items.first() else {
        return Ok(Value::absent());
    };
    match first {
        Value::Int(_) => {
            let mut least = match first {
                Value::Int(number) => *number,
                _ => unreachable!(),
            };
            for item in &items[1..] {
                let Value::Int(number) = item else {
                    return Err(Fault::Confused("a list min mixed whole numbers with something else"));
                };
                least = least.min(*number);
            }
            Ok(Value::found(Value::Int(least)))
        }
        Value::Float(_) => {
            let mut least = match first {
                Value::Float(number) => *number,
                _ => unreachable!(),
            };
            for item in &items[1..] {
                let Value::Float(number) = item else {
                    return Err(Fault::Confused("a list min mixed decimals with something else"));
                };
                least = least.min(*number);
            }
            Ok(Value::found(Value::Float(least)))
        }
        _ => Err(Fault::Confused("a list min was given a value that is not a number")),
    }
}

fn list_max(items: &[Value]) -> Result<Value, Fault> {
    let Some(first) = items.first() else {
        return Ok(Value::absent());
    };
    match first {
        Value::Int(_) => {
            let mut most = match first {
                Value::Int(number) => *number,
                _ => unreachable!(),
            };
            for item in &items[1..] {
                let Value::Int(number) = item else {
                    return Err(Fault::Confused("a list max mixed whole numbers with something else"));
                };
                most = most.max(*number);
            }
            Ok(Value::found(Value::Int(most)))
        }
        Value::Float(_) => {
            let mut most = match first {
                Value::Float(number) => *number,
                _ => unreachable!(),
            };
            for item in &items[1..] {
                let Value::Float(number) = item else {
                    return Err(Fault::Confused("a list max mixed decimals with something else"));
                };
                most = most.max(*number);
            }
            Ok(Value::found(Value::Float(most)))
        }
        _ => Err(Fault::Confused("a list max was given a value that is not a number")),
    }
}

fn list_mean(items: &[Value]) -> Result<Value, Fault> {
    if items.is_empty() {
        return Ok(Value::absent());
    }
    Ok(Value::found(Value::Float(sum_floats(items) / items.len() as f64)))
}

fn sorted_list(items: &[Value]) -> Vec<Value> {
    let mut ordered = items.to_vec();
    ordered.sort_by(|left, right| match (left, right) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => a.total_cmp(b),
        _ => std::cmp::Ordering::Equal,
    });
    ordered
}

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

//! The built-in functions and methods the checker knows about.
//!
//! Phase 5 builds the real standard library. Until then the checker needs to know
//! the shape of the handful of functions the examples and the specification use,
//! or every example would fail to check for want of `print`. So this is a table of
//! signatures and nothing else: no implementations, no runtime.
//!
//! It is deliberately the smallest table that keeps `examples/` honest. Adding to
//! it is how a standard-library function becomes visible to the checker.

use crate::types::{Parameter, Signature, Type};

/// A free function, callable by name from anywhere.
pub struct Function {
    pub name: &'static str,
    pub signature: Signature,
}

/// A method or property reached with `.`.
pub struct Method {
    pub name: &'static str,
    /// What it can be called on. Type parameters here are pinned down by matching
    /// against the real receiver, so `list of T` picks up its `T` from the value.
    pub receiver: Type,
    pub signature: Signature,
    /// `true` for `.is_empty` and `.value`, which are read rather than called.
    pub property: bool,
}

/// Every built-in free function.
pub fn functions() -> Vec<Function> {
    vec![
        Function {
            // `print` shows anything at all, so its parameter is a type parameter.
            name: "print",
            signature: Signature::new(
                vec![Parameter::new("value", Type::parameter("T"))],
                Type::Nothing,
            ),
        },
        Function {
            name: "read_file",
            signature: Signature::new(
                vec![Parameter::new("path", Type::Text)],
                Type::fallible(Type::Text, Type::named("FileError")),
            ),
        },
        Function {
            name: "now",
            signature: Signature::new(Vec::new(), Type::Int),
        },
        Function {
            name: "to_json",
            signature: Signature::new(
                vec![Parameter::new("value", Type::parameter("T"))],
                Type::Text,
            ),
        },
        Function {
            name: "sleep",
            signature: Signature::new(
                vec![Parameter::new("milliseconds", Type::Int)],
                Type::Nothing,
            ),
        },
        Function {
            // `as` is a type, not a value, so the checker special-cases the call.
            // The signature here is what `from_json` looks like as a name.
            name: "from_json",
            signature: Signature::new(
                vec![Parameter::new("text", Type::Text)],
                Type::fallible(Type::parameter("T"), Type::named("JsonError")),
            ),
        },
    ]
}

/// Every built-in method and property.
pub fn methods() -> Vec<Method> {
    let item = || Type::parameter("T");
    let result = || Type::parameter("U");

    vec![
        // ---- Lists --------------------------------------------------------
        Method {
            name: "map",
            receiver: Type::list(item()),
            signature: Signature::new(
                vec![Parameter::new("change", Type::function(vec![item()], result()))],
                Type::list(result()),
            ),
            property: false,
        },
        Method {
            name: "each",
            receiver: Type::list(item()),
            signature: Signature::new(
                vec![Parameter::new("body", Type::function(vec![item()], Type::Nothing))],
                Type::Nothing,
            ),
            property: false,
        },
        Method {
            name: "is_empty",
            receiver: Type::list(item()),
            signature: Signature::new(Vec::new(), Type::Bool),
            property: true,
        },
        Method {
            name: "count",
            receiver: Type::list(item()),
            signature: Signature::new(Vec::new(), Type::Int),
            property: true,
        },
        Method {
            // A list that might be empty has no first item, so this is a `maybe`.
            name: "first",
            receiver: Type::list(item()),
            signature: Signature::new(Vec::new(), Type::maybe(item())),
            property: true,
        },
        Method {
            // Only a list of text can be joined, so the receiver says so and a list
            // of anything else is told what is wrong with it.
            name: "join",
            receiver: Type::list(Type::Text),
            signature: Signature::new(
                vec![Parameter::new("separator", Type::Text)],
                Type::Text,
            ),
            property: false,
        },
        Method {
            name: "sum",
            receiver: Type::list(Type::Int),
            signature: Signature::new(Vec::new(), Type::Int),
            property: false,
        },
        Method {
            name: "sum",
            receiver: Type::list(Type::Float),
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "min",
            receiver: Type::list(Type::Int),
            signature: Signature::new(Vec::new(), Type::maybe(Type::Int)),
            property: false,
        },
        Method {
            name: "min",
            receiver: Type::list(Type::Float),
            signature: Signature::new(Vec::new(), Type::maybe(Type::Float)),
            property: false,
        },
        Method {
            name: "max",
            receiver: Type::list(Type::Int),
            signature: Signature::new(Vec::new(), Type::maybe(Type::Int)),
            property: false,
        },
        Method {
            name: "max",
            receiver: Type::list(Type::Float),
            signature: Signature::new(Vec::new(), Type::maybe(Type::Float)),
            property: false,
        },
        Method {
            name: "mean",
            receiver: Type::list(Type::Int),
            signature: Signature::new(Vec::new(), Type::maybe(Type::Float)),
            property: false,
        },
        Method {
            name: "mean",
            receiver: Type::list(Type::Float),
            signature: Signature::new(Vec::new(), Type::maybe(Type::Float)),
            property: false,
        },
        Method {
            name: "sorted",
            receiver: Type::list(Type::Int),
            signature: Signature::new(Vec::new(), Type::list(Type::Int)),
            property: false,
        },
        Method {
            name: "sorted",
            receiver: Type::list(Type::Float),
            signature: Signature::new(Vec::new(), Type::list(Type::Float)),
            property: false,
        },
        // ---- Maps ---------------------------------------------------------
        Method {
            name: "get",
            receiver: Type::map(Type::parameter("K"), Type::parameter("V")),
            signature: Signature::new(
                vec![Parameter::new("key", Type::parameter("K"))],
                Type::maybe(Type::parameter("V")),
            ),
            property: false,
        },
        Method {
            name: "is_empty",
            receiver: Type::map(Type::parameter("K"), Type::parameter("V")),
            signature: Signature::new(Vec::new(), Type::Bool),
            property: true,
        },
        Method {
            name: "count",
            receiver: Type::map(Type::parameter("K"), Type::parameter("V")),
            signature: Signature::new(Vec::new(), Type::Int),
            property: true,
        },
        Method {
            // In the order the keys were first put in, which is the order a map
            // keeps.
            name: "keys",
            receiver: Type::map(Type::parameter("K"), Type::parameter("V")),
            signature: Signature::new(Vec::new(), Type::list(Type::parameter("K"))),
            property: true,
        },
        // ---- Text ---------------------------------------------------------
        Method {
            name: "upper",
            receiver: Type::Text,
            signature: Signature::new(Vec::new(), Type::Text),
            property: false,
        },
        Method {
            name: "lower",
            receiver: Type::Text,
            signature: Signature::new(Vec::new(), Type::Text),
            property: false,
        },
        Method {
            name: "contains",
            receiver: Type::Text,
            signature: Signature::new(vec![Parameter::new("part", Type::Text)], Type::Bool),
            property: false,
        },
        Method {
            name: "is_empty",
            receiver: Type::Text,
            signature: Signature::new(Vec::new(), Type::Bool),
            property: true,
        },
        Method {
            // Counted in characters, which is what a reader counts. A list and a
            // map are asked for their `count`, because English says a length is a
            // measurement and a count is a number of things.
            name: "length",
            receiver: Type::Text,
            signature: Signature::new(Vec::new(), Type::Int),
            property: true,
        },
        // ---- Numbers ------------------------------------------------------
        Method {
            name: "abs",
            receiver: Type::Int,
            signature: Signature::new(Vec::new(), Type::Int),
            property: false,
        },
        Method {
            name: "min",
            receiver: Type::Int,
            signature: Signature::new(vec![Parameter::new("other", Type::Int)], Type::Int),
            property: false,
        },
        Method {
            name: "max",
            receiver: Type::Int,
            signature: Signature::new(vec![Parameter::new("other", Type::Int)], Type::Int),
            property: false,
        },
        Method {
            // Vaab has no implicit conversions, so going from Int to Float is
            // something a program asks for.
            name: "to_float",
            receiver: Type::Int,
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "abs",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "round",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Int),
            property: false,
        },
        Method {
            name: "sqrt",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "ln",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "exp",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "pow",
            receiver: Type::Float,
            signature: Signature::new(
                vec![Parameter::new("exponent", Type::Float)],
                Type::Float,
            ),
            property: false,
        },
        Method {
            name: "floor",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "ceil",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Float),
            property: false,
        },
        Method {
            name: "min",
            receiver: Type::Float,
            signature: Signature::new(vec![Parameter::new("other", Type::Float)], Type::Float),
            property: false,
        },
        Method {
            name: "max",
            receiver: Type::Float,
            signature: Signature::new(vec![Parameter::new("other", Type::Float)], Type::Float),
            property: false,
        },
        Method {
            name: "is_finite",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Bool),
            property: false,
        },
        Method {
            name: "to_int",
            receiver: Type::Float,
            signature: Signature::new(Vec::new(), Type::Int),
            property: false,
        },
        Method {
            name: "divided_by",
            receiver: Type::Float,
            signature: Signature::new(
                vec![Parameter::new("other", Type::Float)],
                Type::maybe(Type::Float),
            ),
            property: false,
        },
        // ---- Tasks and shared state ---------------------------------------
        Method {
            name: "wait",
            receiver: Type::task(item()),
            signature: Signature::new(Vec::new(), item()),
            property: false,
        },
        Method {
            name: "update",
            receiver: Type::shared(item()),
            signature: Signature::new(
                vec![Parameter::new("change", Type::function(vec![item()], item()))],
                Type::Nothing,
            ),
            property: false,
        },
        Method {
            name: "value",
            receiver: Type::shared(item()),
            signature: Signature::new(Vec::new(), item()),
            property: true,
        },
        // ---- Database -----------------------------------------------------
        Method {
            name: "execute",
            receiver: Type::Db,
            signature: Signature::new(
                vec![
                    Parameter::new("sql", Type::Text),
                    Parameter::new("args", Type::list(Type::Text)),
                ],
                Type::fallible(Type::Int, Type::named("DbError")),
            ),
            property: false,
        },
        Method {
            name: "query",
            receiver: Type::Db,
            signature: Signature::new(
                vec![
                    Parameter::new("sql", Type::Text),
                    Parameter::new("args", Type::list(Type::Text)),
                ],
                Type::fallible(Type::list(Type::map(Type::Text, Type::Text)), Type::named("DbError")),
            ),
            property: false,
        },
        Method {
            name: "from",
            receiver: Type::Db,
            signature: Signature::new(
                vec![Parameter::new("table", Type::Text)],
                Type::query(Type::named("DbError")),
            ),
            property: false,
        },
    ]
}

/// Everything a value of this type offers, for the "did you mean" in an error.
pub fn members_of(receiver: &Type) -> Vec<String> {
    let mut names = Vec::new();
    for method in methods() {
        if same_shape(&method.receiver, receiver) && !names.iter().any(|name| name == method.name) {
            names.push(method.name.to_string());
        }
    }
    names
}

/// The compiler/runtime name for a prelude method, when the user-facing name is
/// shared by more than one receiver (list `sum` of Int vs Float, list `min` vs
/// number `min`).
pub fn builtin_code(name: &'static str, receiver: &Type) -> &'static str {
    match (name, receiver) {
        ("sum", Type::List(item)) if **item == Type::Int => "sum_int",
        ("sum", Type::List(item)) if **item == Type::Float => "sum_float",
        ("min", Type::List(_)) => "list_min",
        ("max", Type::List(_)) => "list_max",
        ("mean", Type::List(_)) => "list_mean",
        ("sorted", Type::List(_)) => "list_sorted",
        _ => name,
    }
}

/// Whether two types are built the same way, ignoring what they hold.
///
/// Method lookup is by shape: `.get` belongs to every map, whatever it maps. What
/// it holds is then pinned down by unification, which is also how a `.join` on a
/// list of numbers is told precisely what is wrong.
pub fn same_shape(left: &Type, right: &Type) -> bool {
    match (left, right) {
        (Type::List(_), Type::List(_))
        | (Type::Map { .. }, Type::Map { .. })
            | (Type::Task(_), Type::Task(_))
            | (Type::Shared(_), Type::Shared(_))
            | (Type::Channel(_), Type::Channel(_))
            | (Type::Db, Type::Db)
            | (Type::Store, Type::Store)
            | (Type::Logger, Type::Logger)
            | (Type::Query { .. }, Type::Query { .. })
            | (Type::Maybe(_), Type::Maybe(_)) => true,
        (left, right) => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prelude_holds_exactly_what_the_examples_need() {
        let names: Vec<&str> = functions().iter().map(|function| function.name).collect();
        assert_eq!(names, ["print", "read_file", "now", "to_json", "sleep", "from_json"]);
    }

    #[test]
    fn a_method_is_found_by_the_shape_of_its_receiver() {
        assert!(same_shape(&Type::list(Type::Text), &Type::list(Type::Int)));
        assert!(!same_shape(&Type::list(Type::Text), &Type::Text));
        assert!(same_shape(&Type::Text, &Type::Text));
    }

    #[test]
    fn a_list_offers_the_methods_a_list_should() {
        let offered = members_of(&Type::list(Type::Int));
        for expected in ["map", "each", "is_empty", "join"] {
            assert!(offered.iter().any(|name| name == expected), "missing {expected}");
        }
        assert!(!offered.iter().any(|name| name == "upper"));
    }
}

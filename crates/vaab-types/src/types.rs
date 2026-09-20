//! What a Vaab value is.
//!
//! A [`Type`] is the checker's answer to "what is this?". It is spelled the way
//! the language is: `maybe Int`, `list of Text`, `map of Text to Int`. That matters
//! because these are printed straight into error messages, and an error that names
//! a type in a notation the programmer has never seen is an error twice.
//!
//! Generics are erased. A signature that mentions `T` holds a
//! [`Type::Parameter`], and a call site swaps each parameter for a fresh
//! [`Type::Variable`] that unification then fills in. Nothing is generalised
//! afterwards, so this is much less than Hindley-Milner — and enough for a
//! language where every signature is written out in full.

use std::collections::HashMap;
use std::fmt;

/// A hole in a type, filled in by unification. See [`crate::unify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariableId(pub u32);

/// The type of a Vaab value.
#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Text,
    /// The unit type: what a function with no `returns` gives back.
    Nothing,
    List(Box<Type>),
    Map { key: Box<Type>, value: Box<Type> },
    Maybe(Box<Type>),
    Fallible { ok: Box<Type>, error: Box<Type> },
    Tuple(Vec<Type>),
    Function(Box<FunctionType>),
    /// A declared `type` or `choice`, by name.
    Named(String),
    /// A declared `ability`, as used for a parameter that accepts any type
    /// providing it.
    Ability(String),
    Channel(Box<Type>),
    Task(Box<Type>),
    Shared(Box<Type>),
    /// A type parameter such as `T`, inside the signature that introduces it.
    Parameter(String),
    /// A hole, filled in by unification at a call site.
    Variable(VariableId),
    /// A type that could not be worked out because something was already wrong.
    ///
    /// It fits everywhere and is never reported, which is what stops one mistake
    /// from turning into ten.
    Unknown,
}

/// The type of a function *value*, which is all a caller needs to call it
/// positionally. Names and defaults live in a [`Signature`] instead, because they
/// belong to a particular declaration rather than to the type.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionType {
    pub parameters: Vec<Type>,
    pub returns: Type,
}

impl Type {
    pub fn list(item: Type) -> Type {
        Type::List(Box::new(item))
    }

    pub fn map(key: Type, value: Type) -> Type {
        Type::Map { key: Box::new(key), value: Box::new(value) }
    }

    pub fn maybe(item: Type) -> Type {
        Type::Maybe(Box::new(item))
    }

    pub fn fallible(ok: Type, error: Type) -> Type {
        Type::Fallible { ok: Box::new(ok), error: Box::new(error) }
    }

    pub fn channel(item: Type) -> Type {
        Type::Channel(Box::new(item))
    }

    pub fn task(item: Type) -> Type {
        Type::Task(Box::new(item))
    }

    pub fn shared(item: Type) -> Type {
        Type::Shared(Box::new(item))
    }

    pub fn function(parameters: Vec<Type>, returns: Type) -> Type {
        Type::Function(Box::new(FunctionType { parameters, returns }))
    }

    pub fn parameter(name: impl Into<String>) -> Type {
        Type::Parameter(name.into())
    }

    pub fn named(name: impl Into<String>) -> Type {
        Type::Named(name.into())
    }

    /// Whether anything went wrong inside this type, in which case a mismatch
    /// involving it has already been explained and should stay quiet.
    pub fn is_unknown(&self) -> bool {
        matches!(self, Type::Unknown)
    }

    /// Collects the names of every type parameter mentioned, in first-seen order.
    pub fn collect_parameters(&self, found: &mut Vec<String>) {
        match self {
            Type::Parameter(name) => {
                if !found.iter().any(|seen| seen == name) {
                    found.push(name.clone());
                }
            }
            Type::List(item)
            | Type::Maybe(item)
            | Type::Channel(item)
            | Type::Task(item)
            | Type::Shared(item) => item.collect_parameters(found),
            Type::Map { key, value } => {
                key.collect_parameters(found);
                value.collect_parameters(found);
            }
            Type::Fallible { ok, error } => {
                ok.collect_parameters(found);
                error.collect_parameters(found);
            }
            Type::Tuple(items) => {
                for item in items {
                    item.collect_parameters(found);
                }
            }
            Type::Function(function) => {
                for parameter in &function.parameters {
                    parameter.collect_parameters(found);
                }
                function.returns.collect_parameters(found);
            }
            _ => {}
        }
    }

    /// Replaces each named type parameter with whatever `swaps` says.
    ///
    /// Parameters not mentioned in `swaps` are left alone, which is what keeps a
    /// generic function's own `T` opaque while checking its body.
    pub fn substitute(&self, swaps: &HashMap<String, Type>) -> Type {
        match self {
            Type::Parameter(name) => swaps.get(name).cloned().unwrap_or_else(|| self.clone()),
            Type::List(item) => Type::list(item.substitute(swaps)),
            Type::Maybe(item) => Type::maybe(item.substitute(swaps)),
            Type::Channel(item) => Type::channel(item.substitute(swaps)),
            Type::Task(item) => Type::task(item.substitute(swaps)),
            Type::Shared(item) => Type::shared(item.substitute(swaps)),
            Type::Map { key, value } => Type::map(key.substitute(swaps), value.substitute(swaps)),
            Type::Fallible { ok, error } => {
                Type::fallible(ok.substitute(swaps), error.substitute(swaps))
            }
            Type::Tuple(items) => {
                Type::Tuple(items.iter().map(|item| item.substitute(swaps)).collect())
            }
            Type::Function(function) => Type::function(
                function.parameters.iter().map(|item| item.substitute(swaps)).collect(),
                function.returns.substitute(swaps),
            ),
            _ => self.clone(),
        }
    }
}

/// Renders a type the way Vaab spells it, because this text ends up in messages.
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => f.write_str("Int"),
            Type::Float => f.write_str("Float"),
            Type::Bool => f.write_str("Bool"),
            Type::Text => f.write_str("Text"),
            Type::Nothing => f.write_str("Nothing"),
            Type::List(item) => write!(f, "list of {item}"),
            Type::Map { key, value } => write!(f, "map of {key} to {value}"),
            Type::Maybe(item) => write!(f, "maybe {item}"),
            Type::Channel(item) => write!(f, "channel of {item}"),
            Type::Task(item) => write!(f, "task of {item}"),
            Type::Shared(item) => write!(f, "shared {item}"),
            Type::Fallible { ok, error } => write!(f, "{ok} or fails {error}"),
            Type::Tuple(items) => {
                f.write_str("(")?;
                for (position, item) in items.iter().enumerate() {
                    if position > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str(")")
            }
            Type::Function(function) => {
                f.write_str("to(")?;
                for (position, parameter) in function.parameters.iter().enumerate() {
                    if position > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{parameter}")?;
                }
                f.write_str(")")?;
                match &function.returns {
                    Type::Nothing => Ok(()),
                    returns => write!(f, " returns {returns}"),
                }
            }
            Type::Named(name) | Type::Ability(name) | Type::Parameter(name) => f.write_str(name),
            // A type Vaab could not pin down. There is no Vaab spelling for one, and
            // every message that could print one already says what went wrong.
            Type::Variable(_) | Type::Unknown => f.write_str("_"),
        }
    }
}

/// What one particular declaration accepts: its parameters, by name, with their
/// defaults, and what it gives back.
///
/// Calls that name the declaration may use argument names and leave out defaults.
/// Calls through a function *value* only have a [`FunctionType`], so they are
/// positional.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub parameters: Vec<Parameter>,
    pub returns: Type,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub declared: Type,
    /// `greeting: Text = "hello"` may be left out at a call site.
    pub optional: bool,
}

impl Signature {
    pub fn new(parameters: Vec<Parameter>, returns: Type) -> Self {
        Signature { parameters, returns }
    }

    /// This signature as a plain function type, forgetting names and defaults.
    pub fn as_type(&self) -> Type {
        Type::function(
            self.parameters.iter().map(|parameter| parameter.declared.clone()).collect(),
            self.returns.clone(),
        )
    }

    pub fn substitute(&self, swaps: &HashMap<String, Type>) -> Signature {
        Signature {
            parameters: self
                .parameters
                .iter()
                .map(|parameter| Parameter {
                    name: parameter.name.clone(),
                    declared: parameter.declared.substitute(swaps),
                    optional: parameter.optional,
                })
                .collect(),
            returns: self.returns.substitute(swaps),
        }
    }

    pub fn collect_parameters(&self, found: &mut Vec<String>) {
        for parameter in &self.parameters {
            parameter.declared.collect_parameters(found);
        }
        self.returns.collect_parameters(found);
    }

    /// Writes the signature as it would be declared, for messages that compare one
    /// signature with another: `to describe() returns Text`.
    pub fn describe(&self, name: &str) -> String {
        let parameters: Vec<String> = self
            .parameters
            .iter()
            .map(|parameter| format!("{}: {}", parameter.name, parameter.declared))
            .collect();
        let result = match &self.returns {
            Type::Nothing => String::new(),
            returns => format!(" returns {returns}"),
        };
        format!("to {name}({}){result}", parameters.join(", "))
    }
}

impl Parameter {
    pub fn new(name: impl Into<String>, declared: Type) -> Self {
        Parameter { name: name.into(), declared, optional: false }
    }

    pub fn optional(name: impl Into<String>, declared: Type) -> Self {
        Parameter { name: name.into(), declared, optional: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_print_the_way_vaab_spells_them() {
        assert_eq!(Type::maybe(Type::Int).to_string(), "maybe Int");
        assert_eq!(Type::list(Type::Text).to_string(), "list of Text");
        assert_eq!(Type::map(Type::Text, Type::Int).to_string(), "map of Text to Int");
        assert_eq!(
            Type::fallible(Type::Int, Type::named("FileError")).to_string(),
            "Int or fails FileError"
        );
        assert_eq!(Type::Tuple(vec![Type::Int, Type::Text]).to_string(), "(Int, Text)");
        assert_eq!(Type::list(Type::maybe(Type::Int)).to_string(), "list of maybe Int");
    }

    #[test]
    fn a_function_with_no_result_prints_without_returns() {
        assert_eq!(Type::function(vec![Type::Int], Type::Nothing).to_string(), "to(Int)");
        assert_eq!(
            Type::function(vec![Type::Int], Type::Bool).to_string(),
            "to(Int) returns Bool"
        );
    }

    #[test]
    fn substitution_reaches_inside_compound_types() {
        let mut swaps = HashMap::new();
        swaps.insert("T".to_string(), Type::Int);
        let generic = Type::function(vec![Type::list(Type::parameter("T"))], Type::maybe(Type::parameter("T")));
        assert_eq!(generic.substitute(&swaps).to_string(), "to(list of Int) returns maybe Int");
    }

    #[test]
    fn parameters_are_collected_in_the_order_they_appear() {
        let mut found = Vec::new();
        Type::map(Type::parameter("K"), Type::list(Type::parameter("V")))
            .collect_parameters(&mut found);
        assert_eq!(found, ["K", "V"]);
    }

    #[test]
    fn a_signature_describes_itself_as_it_was_written() {
        let signature = Signature::new(
            vec![Parameter::new("amount", Type::Int)],
            Type::fallible(Type::named("Account"), Type::named("AccountError")),
        );
        assert_eq!(
            signature.describe("deposit"),
            "to deposit(amount: Int) returns Account or fails AccountError"
        );
    }
}

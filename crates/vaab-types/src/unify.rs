//! Filling in the holes.
//!
//! A generic signature is turned into a concrete one at each call site by swapping
//! every type parameter for a fresh [`Type::Variable`]. Those variables are then
//! pinned down by matching the arguments against the parameters. There is no
//! generalisation and no constraint solving beyond that: [`Variables::fits`] walks
//! two types together and binds a variable the moment it meets one.
//!
//! [`Variables::fits`] is deliberately one-directional — it asks whether a value of
//! one type may be used where another is wanted — because Vaab has one place where
//! the answer is not symmetric: a type that `can Describable` may be used as a
//! `Describable`, but not the other way round.

use std::collections::HashMap;

use crate::types::{Signature, Type, VariableId};

/// The variables in play, and what each has been pinned to so far.
#[derive(Debug, Default)]
pub struct Variables {
    bindings: Vec<Option<Type>>,
    /// For each ability, the types that provide it. Filled in once, up front.
    providers: HashMap<String, Vec<String>>,
}

impl Variables {
    /// Records that `provider` was declared `can ability`.
    pub fn record_provider(&mut self, ability: &str, provider: &str) {
        self.providers.entry(ability.to_string()).or_default().push(provider.to_string());
    }

    pub fn provides(&self, provider: &str, ability: &str) -> bool {
        self.providers.get(ability).is_some_and(|names| names.iter().any(|name| name == provider))
    }

    pub fn fresh(&mut self) -> Type {
        let id = VariableId(self.bindings.len() as u32);
        self.bindings.push(None);
        Type::Variable(id)
    }

    /// A copy of `signature` with each of its type parameters replaced by a fresh
    /// variable, which is what makes a generic function callable.
    pub fn instantiate(&mut self, signature: &Signature) -> Signature {
        let mut parameters = Vec::new();
        signature.collect_parameters(&mut parameters);
        if parameters.is_empty() {
            return signature.clone();
        }
        signature.substitute(&self.swaps(&parameters))
    }

    /// Instantiates a receiver and a signature as one, so that a `T` mentioned in
    /// both becomes the *same* variable. This is what carries the item type of a
    /// `list of T` through to what `.map` gives back.
    pub fn instantiate_together(
        &mut self,
        receiver: &Type,
        signature: &Signature,
    ) -> (Type, Signature) {
        let mut parameters = Vec::new();
        receiver.collect_parameters(&mut parameters);
        signature.collect_parameters(&mut parameters);
        if parameters.is_empty() {
            return (receiver.clone(), signature.clone());
        }
        let swaps = self.swaps(&parameters);
        (receiver.substitute(&swaps), signature.substitute(&swaps))
    }

    fn swaps(&mut self, parameters: &[String]) -> HashMap<String, Type> {
        parameters.iter().map(|name| (name.clone(), self.fresh())).collect()
    }

    /// Replaces every variable in `declared` with whatever it has been pinned to,
    /// as deeply as it goes.
    pub fn resolve(&self, declared: &Type) -> Type {
        match declared {
            Type::Variable(id) => match self.binding(*id) {
                Some(bound) => self.resolve(&bound),
                None => declared.clone(),
            },
            Type::List(item) => Type::list(self.resolve(item)),
            Type::Maybe(item) => Type::maybe(self.resolve(item)),
            Type::Channel(item) => Type::channel(self.resolve(item)),
            Type::Task(item) => Type::task(self.resolve(item)),
            Type::Shared(item) => Type::shared(self.resolve(item)),
            Type::Map { key, value } => Type::map(self.resolve(key), self.resolve(value)),
            Type::Fallible { ok, error } => Type::fallible(self.resolve(ok), self.resolve(error)),
            Type::Query { error } => Type::query(self.resolve(error)),
            Type::Tuple(items) => {
                Type::Tuple(items.iter().map(|item| self.resolve(item)).collect())
            }
            Type::Function(function) => Type::function(
                function.parameters.iter().map(|item| self.resolve(item)).collect(),
                self.resolve(&function.returns),
            ),
            _ => declared.clone(),
        }
    }

    fn binding(&self, id: VariableId) -> Option<Type> {
        self.bindings.get(id.0 as usize).cloned().flatten()
    }

    fn bind(&mut self, id: VariableId, to: &Type) -> bool {
        // A variable standing for itself is already as pinned down as it can be.
        if matches!(to, Type::Variable(other) if *other == id) {
            return true;
        }
        match self.bindings.get_mut(id.0 as usize) {
            Some(slot) => {
                *slot = Some(to.clone());
                true
            }
            // Only this module hands out variables, so there is no id it does not
            // know. Refusing is still better than pretending the match succeeded.
            None => false,
        }
    }

    /// Whether a value of type `found` may be used where `wanted` is expected,
    /// pinning down any variables that makes possible.
    ///
    /// Vaab has no implicit conversions, so this is exact matching plus two
    /// deliberate exceptions: [`Type::Unknown`] fits anywhere, because whatever
    /// produced it has already been reported, and a type fits an ability it was
    /// declared to provide.
    pub fn fits(&mut self, found: &Type, wanted: &Type) -> bool {
        let found = self.resolve(found);
        let wanted = self.resolve(wanted);

        if found.is_unknown() || wanted.is_unknown() {
            return true;
        }

        match (&found, &wanted) {
            (Type::Variable(id), other) => self.bind(*id, other),
            (other, Type::Variable(id)) => self.bind(*id, other),

            (Type::Named(name), Type::Ability(ability)) => self.provides(name, ability),

            (Type::List(from), Type::List(to))
            | (Type::Maybe(from), Type::Maybe(to))
            | (Type::Channel(from), Type::Channel(to))
            | (Type::Task(from), Type::Task(to))
            | (Type::Shared(from), Type::Shared(to)) => self.fits(from, to),

            (
                Type::Map { key: from_key, value: from_value },
                Type::Map { key: to_key, value: to_value },
            ) => self.fits(from_key, to_key) && self.fits(from_value, to_value),

            (
                Type::Fallible { ok: from_ok, error: from_error },
                Type::Fallible { ok: to_ok, error: to_error },
            ) => self.fits(from_ok, to_ok) && self.fits(from_error, to_error),

            (Type::Query { error: from }, Type::Query { error: to }) => self.fits(from, to),

            (Type::Tuple(from), Type::Tuple(to)) => {
                from.len() == to.len()
                    && from.iter().zip(to).all(|(from, to)| self.fits(from, to))
            }

            (Type::Function(from), Type::Function(to)) => {
                from.parameters.len() == to.parameters.len()
                    && from
                        .parameters
                        .iter()
                        .zip(&to.parameters)
                        .all(|(from, to)| self.fits(from, to))
                    && self.fits(&from.returns, &to.returns)
            }

            (from, to) => from == to,
        }
    }

    /// Whether two types are the same thing, in both directions. Used where
    /// Vaab insists on an exact match, such as the error type of `try`.
    pub fn same(&mut self, left: &Type, right: &Type) -> bool {
        self.fits(left, right) && self.fits(right, left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Parameter;

    #[test]
    fn a_variable_takes_the_shape_it_meets() {
        let mut variables = Variables::default();
        let hole = variables.fresh();
        assert!(variables.fits(&Type::Int, &hole));
        assert_eq!(variables.resolve(&hole), Type::Int);
    }

    #[test]
    fn a_variable_keeps_the_first_shape_it_took() {
        let mut variables = Variables::default();
        let hole = variables.fresh();
        assert!(variables.fits(&Type::Int, &hole));
        assert!(!variables.fits(&Type::Text, &hole));
    }

    #[test]
    fn a_variable_inside_a_list_is_pinned_down_too() {
        let mut variables = Variables::default();
        let hole = variables.fresh();
        assert!(variables.fits(&Type::list(Type::Text), &Type::list(hole.clone())));
        assert_eq!(variables.resolve(&hole), Type::Text);
    }

    #[test]
    fn nothing_converts_implicitly() {
        let mut variables = Variables::default();
        assert!(!variables.fits(&Type::Int, &Type::Float));
        assert!(!variables.fits(&Type::Int, &Type::Text));
        assert!(!variables.fits(&Type::maybe(Type::Int), &Type::Int));
    }

    #[test]
    fn a_type_fits_an_ability_it_provides() {
        let mut variables = Variables::default();
        variables.record_provider("Describable", "Person");
        assert!(variables.fits(&Type::named("Person"), &Type::Ability("Describable".into())));
        assert!(!variables.fits(&Type::named("Planet"), &Type::Ability("Describable".into())));
        // And not the other way round: a Describable is not necessarily a Person.
        assert!(!variables.fits(&Type::Ability("Describable".into()), &Type::named("Person")));
    }

    #[test]
    fn an_unknown_type_fits_anywhere_so_one_mistake_stays_one_mistake() {
        let mut variables = Variables::default();
        assert!(variables.fits(&Type::Unknown, &Type::Int));
        assert!(variables.fits(&Type::Int, &Type::Unknown));
    }

    #[test]
    fn instantiating_a_signature_replaces_every_mention_of_a_parameter() {
        let mut variables = Variables::default();
        let generic = Signature::new(
            vec![Parameter::new("items", Type::list(Type::parameter("T")))],
            Type::maybe(Type::parameter("T")),
        );
        let fresh = variables.instantiate(&generic);

        // Both mentions of `T` became the *same* variable, so pinning one pins both.
        let Some(first) = fresh.parameters.first() else { unreachable!() };
        assert!(variables.fits(&Type::list(Type::Int), &first.declared));
        assert_eq!(variables.resolve(&fresh.returns), Type::maybe(Type::Int));
    }
}

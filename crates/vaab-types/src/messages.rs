//! Every message the type checker can produce.
//!
//! They live together so the voice stays one voice. The house style is the one
//! documented in `vaab_syntax::diagnostic`: the `message` says what is wrong in one
//! short sentence with the code as its subject, the primary label says what was
//! found at that exact spot, `help` says what to type instead, and `note` only
//! appears when the background genuinely helps.
//!
//! Two of these messages are fixed by the specification and must not drift:
//! [`mismatch`] produces "expected Int, found maybe Int", and [`missing_fields`]
//! produces "Account.new is missing `owner`."

use vaab_syntax::diagnostic::Diagnostic;
use vaab_syntax::span::Span;

use crate::types::{Signature, Type};

// ---------------------------------------------------------------------------
// Wording helpers
// ---------------------------------------------------------------------------

/// Joins names into an English list: "`a`", "`a` and `b`", "`a`, `b` and `c`".
pub fn listed(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

fn counted(count: usize, thing: &str) -> String {
    if count == 1 {
        format!("1 {thing}")
    } else {
        format!("{count} {thing}s")
    }
}

/// The closest of `candidates` to `written`, when one is close enough to be worth
/// suggesting. A typo is usually one or two keystrokes out; anything further away
/// is a guess, and a wrong guess is worse than none.
pub fn nearest<'a>(written: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let allowed = match written.chars().count() {
        0..=2 => 1,
        3..=5 => 2,
        _ => 3,
    };

    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        if candidate == written {
            continue;
        }
        let distance = distance(written, candidate);
        if distance <= allowed && best.map_or(true, |(closest, _)| distance < closest) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate.to_string())
}

/// How many single-character edits turn `from` into `to`.
fn distance(from: &str, to: &str) -> usize {
    let from: Vec<char> = from.chars().collect();
    let to: Vec<char> = to.chars().collect();

    let mut previous: Vec<usize> = (0..=to.len()).collect();
    let mut current = vec![0usize; to.len() + 1];

    for (row, left) in from.iter().enumerate() {
        current[0] = row + 1;
        for (column, right) in to.iter().enumerate() {
            let substitution = previous[column] + usize::from(left != right);
            let insertion = current[column] + 1;
            let deletion = previous[column + 1] + 1;
            current[column + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous.last().copied().unwrap_or(0)
}

/// A type with the article that reads right in front of it: "an Int", "a list of
/// Text". Declared types are quoted, because a name a person chose reads better
/// when it is visibly a name: "an `Account`".
fn a(declared: &Type) -> String {
    let spelled = match declared {
        Type::Named(name) | Type::Ability(name) => format!("`{name}`"),
        other => other.to_string(),
    };
    let vowel = spelled
        .chars()
        .find(|letter| letter.is_alphabetic())
        .is_some_and(|letter| "aeiouAEIOU".contains(letter));
    if vowel {
        format!("an {spelled}")
    } else {
        format!("a {spelled}")
    }
}

/// "did you mean `x`?", or a fallback when nothing was close.
fn did_you_mean(written: &str, candidates: &[String], fallback: String) -> String {
    match nearest(written, candidates.iter().map(String::as_str)) {
        Some(close) => format!("did you mean `{close}`?"),
        None => fallback,
    }
}

// ---------------------------------------------------------------------------
// Types that do not line up
// ---------------------------------------------------------------------------

/// The general mismatch: "expected Int, found maybe Int".
pub fn mismatch(wanted: &Type, found: &Type, span: Span) -> Diagnostic {
    let diagnostic = Diagnostic::error(
        "type-mismatch",
        format!("expected {wanted}, found {found}"),
    )
    .at(span, format!("this is {found}"));

    match (wanted, found) {
        (wanted, Type::Maybe(held)) if **held == *wanted => diagnostic
            .with_help(format!(
                "a `maybe {held}` might hold nothing, so say what to do then: add \
                 `otherwise <fallback>`, or take it apart with `match`"
            ))
            .with_note(
                "Vaab has no null, so a value that might be absent has a different type \
                 from one that is there",
            ),
        (wanted, Type::Fallible { ok, error }) if **ok == *wanted => diagnostic
            .with_help(
                "deal with the failure first: hand it back to the caller with `try`, or \
                 take it apart with `match`",
            )
            .with_note(format!("this can fail with `{error}`")),
        (Type::Float, Type::Int) => diagnostic
            .with_help("write the number with a decimal point, as in `1.0`")
            .with_note("Vaab never converts between Int and Float on its own"),
        (Type::Int, Type::Float) => diagnostic
            .with_help("use a whole number here")
            .with_note("Vaab never rounds on its own, because the rounding it chose would be wrong as often as it was right"),
        _ => diagnostic,
    }
}

/// A mismatch between two things that have to agree: the branches of an `if`, or
/// the arms of a `match`. `what` is "branches" or "arms".
pub fn branches_differ(
    what: &str,
    first: &Type,
    first_span: Span,
    second: &Type,
    second_span: Span,
) -> Diagnostic {
    let form = if what == "arms" { "match" } else { "if" };
    Diagnostic::error("branches-differ", format!("these {what} give different types"))
        .at(second_span, format!("this one gives {second}"))
        .also_at(first_span, format!("the first gives {first}"))
        .with_help(format!("make every one of the {what} give the same type"))
        .with_note(format!(
            "`{form}` is an expression in Vaab, so whichever way it goes the answer has one type"
        ))
}

pub fn if_without_else(span: Span, wanted: &Type) -> Diagnostic {
    Diagnostic::error("if-without-else", "this `if` has no `else`, so it sometimes has no value")
        .at(span, format!("a {wanted} is expected here"))
        .with_help("add `else { ... }` giving a value for the other case")
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

pub fn undefined_name(name: &str, span: Span, known: &[String]) -> Diagnostic {
    Diagnostic::error("undefined-name", format!("`{name}` is not defined"))
        .at(span, "Vaab has not seen this name before")
        .with_help(did_you_mean(
            name,
            known,
            format!("give it a value first with `let {name} = ...`, or check the spelling"),
        ))
}

pub fn undefined_type(name: &str, span: Span, known: &[String]) -> Diagnostic {
    Diagnostic::error("undefined-type", format!("`{name}` is not a type"))
        .at(span, "Vaab has not seen this type before")
        .with_help(did_you_mean(
            name,
            known,
            format!("declare it with `type {name} {{ ... }}`, or check the spelling"),
        ))
}

pub fn type_used_as_value(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error("type-used-as-value", format!("`{name}` is a type, not a value"))
        .at(span, "this names a type")
        .with_help(format!("build one with `{name}.new(...)`"))
}

pub fn duplicate_declaration(name: &str, span: Span, first: Span) -> Diagnostic {
    Diagnostic::error("duplicate-declaration", format!("`{name}` is declared twice"))
        .at(span, "this declaration repeats an earlier one")
        .also_at(first, "first declared here")
        .with_help("give one of them a different name, or remove it")
}

pub fn nested_declaration(what: &str, name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "nested-declaration",
        format!("a {what} has to be declared at the top level of a file"),
    )
    .at(span, format!("`{name}` is declared inside a block"))
    .with_help(format!("move `{what} {name} {{ ... }}` out to the top level"))
    .with_note(format!(
        "a {what}'s name can be used anywhere in the file, so it cannot belong to one block"
    ))
}

pub fn self_outside_type(span: Span) -> Diagnostic {
    Diagnostic::error("self-outside-type", "`self` only means something inside a type")
        .at(span, "there is no value for `self` to be here")
        .with_help("move this into a `type`'s body, or take what it needs as a parameter")
}

// ---------------------------------------------------------------------------
// Changing values
// ---------------------------------------------------------------------------

pub fn not_changing(name: &str, span: Span, declared: Span) -> Diagnostic {
    Diagnostic::error("not-changing", format!("`{name}` cannot be changed"))
        .at(span, "this assignment needs a value that may change")
        .also_at(declared, format!("`{name}` is fixed when it is declared here"))
        .with_help(format!("declare it as `let changing {name} = ...`"))
}

pub fn cannot_assign_field(owner: &str, span: Span) -> Diagnostic {
    let owner = a(&Type::named(owner));
    Diagnostic::error("cannot-assign-part", format!("{owner} cannot be changed in place"))
        .at(span, "this is one of its fields")
        .with_help("make a changed copy instead, with `.with(field: value)`")
        .with_note("every `type` in Vaab is immutable, which is what makes it safe to share")
}

pub fn cannot_assign_item(span: Span) -> Diagnostic {
    Diagnostic::error("cannot-assign-part", "an item of a list cannot be assigned")
        .at(span, "this is an item inside a list")
        .with_help("build the list you want instead, with `.map(...)`")
        .with_note("a list in Vaab is a value, not a box to be changed")
}

// ---------------------------------------------------------------------------
// Fields and members
// ---------------------------------------------------------------------------

pub fn unknown_field(owner: &str, field: &str, span: Span, known: &[String]) -> Diagnostic {
    let fallback = if known.is_empty() {
        format!("`{owner}` has no fields")
    } else {
        format!("the fields of `{owner}` are {}", listed(known))
    };
    Diagnostic::error("unknown-field", format!("`{owner}` has no field `{field}`"))
        .at(span, "this name is not one of its fields")
        .with_help(did_you_mean(field, known, fallback))
}

pub fn unknown_member(owner: &Type, member: &str, span: Span, known: &[String]) -> Diagnostic {
    let owner = a(owner);
    let diagnostic =
        Diagnostic::error("unknown-member", format!("{owner} has nothing called `{member}`"))
            .at(span, format!("this asks {owner} for `{member}`"));
    if known.is_empty() {
        diagnostic
    } else {
        diagnostic.with_help(did_you_mean(
            member,
            known,
            format!("{owner} offers {}", listed(known)),
        ))
    }
}

pub fn needs_call(member: &str, span: Span) -> Diagnostic {
    Diagnostic::error("needs-call", format!("`{member}` is a function, so it has to be called"))
        .at(span, "this reads it without calling it")
        .with_help(format!("add the brackets: `{member}()`"))
}

pub fn bad_receiver(member: &str, wanted: &Type, found: &Type, span: Span) -> Diagnostic {
    let diagnostic =
        Diagnostic::error("bad-receiver", format!("`{member}` needs {wanted}"))
            .at(span, format!("this is {found}"));
    if member == "join" {
        diagnostic.with_help("turn each item into text first, with `.map(item -> \"{item}\")`")
    } else {
        diagnostic
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// One report for a half-filled constructor, however many fields are missing: it is
/// one mistake, not four. `callable` is written as it was called — `Account.new` —
/// because that is the thing that is incomplete.
pub fn missing_fields(
    callable: &str,
    owner: &str,
    missing: &[String],
    span: Span,
) -> Diagnostic {
    let diagnostic = Diagnostic::error(
        "missing-field",
        format!("{callable} is missing {}.", listed(missing)),
    )
    .at(span, format!("this leaves out {}", listed(missing)))
    .with_help(format!(
        "add {}",
        missing.iter().map(|name| format!("`{name}: ...`")).collect::<Vec<String>>().join(", ")
    ));

    if missing.len() == 1 {
        let Some(only) = missing.first() else { return diagnostic };
        diagnostic.with_note(format!(
            "`{only}` has no default, so every `{owner}` has to be given one"
        ))
    } else {
        diagnostic.with_note(format!(
            "these fields have no defaults, so every `{owner}` has to be given them"
        ))
    }
}

pub fn new_needs_names(callable: &str, first_field: Option<&str>, span: Span) -> Diagnostic {
    let example = first_field.unwrap_or("field");
    Diagnostic::error("new-needs-names", format!("`{callable}` takes its fields by name"))
        .at(span, "this argument has no name")
        .with_help(format!("write `{callable}({example}: ...)`"))
        .with_note("naming each field means the order never matters, so two fields of the same type cannot be swapped by mistake")
}

pub fn raw_outside_type(owner: &str, span: Span) -> Diagnostic {
    Diagnostic::error("raw-outside-type", format!("`{owner}.raw` can only be used inside `{owner}`"))
        .at(span, format!("this is outside `{owner}`'s body"))
        .with_help(format!("build one with `{owner}.new(...)`, which checks it first"))
        .with_note(format!(
            "`raw` skips the checks in `{owner}`'s own `to new`, so only `{owner}` may use it"
        ))
}

pub fn needs_a_type_name(span: Span) -> Diagnostic {
    Diagnostic::error("needs-a-type-name", "`Channel.new` needs the type of what it carries")
        .at(span, "Vaab expected a type here")
        .with_help("write `Channel.new(of: Text, size: 4)`")
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

pub fn missing_argument(
    callee: &str,
    parameter: &str,
    span: Span,
    declared: Option<Span>,
) -> Diagnostic {
    let diagnostic = Diagnostic::error(
        "missing-argument",
        format!("this call to `{callee}` is missing `{parameter}`"),
    )
    .at(span, format!("`{parameter}` is never given a value"))
    .with_help(format!("pass it, as in `{callee}({parameter}: ...)`"));

    match declared {
        Some(declared) => diagnostic.also_at(declared, format!("`{callee}` is declared here")),
        None => diagnostic,
    }
}

/// A function reached through a value knows the types of its parameters but not
/// their names, so there is nothing for an argument to be named after.
pub fn names_need_a_declaration(written: &str, span: Span) -> Diagnostic {
    Diagnostic::error("names-need-a-declaration", "this call cannot name its arguments")
        .at(span, format!("this names `{written}`"))
        .with_help("pass the arguments in order instead")
        .with_note(
            "argument names come from a declaration, and a function held in a value \
             carries only its types",
        )
}

pub fn too_few_arguments(callee: &str, takes: usize, given: usize, span: Span) -> Diagnostic {
    Diagnostic::error(
        "wrong-argument-count",
        format!("`{callee}` takes {}, but this call gives {given}", counted(takes, "argument")),
    )
    .at(span, format!("this call gives {}", counted(given, "argument")))
    .with_help("pass the rest, in order")
}

pub fn unknown_argument(
    callee: &str,
    written: &str,
    span: Span,
    known: &[String],
) -> Diagnostic {
    let fallback = if known.is_empty() {
        format!("`{callee}` takes no arguments")
    } else {
        format!("the parameters of `{callee}` are {}", listed(known))
    };
    Diagnostic::error(
        "unknown-argument",
        format!("`{callee}` has no parameter called `{written}`"),
    )
    .at(span, "this name is not one of its parameters")
    .with_help(did_you_mean(written, known, fallback))
}

pub fn duplicate_argument(name: &str, span: Span, first: Span) -> Diagnostic {
    Diagnostic::error("duplicate-argument", format!("`{name}` is given twice"))
        .at(span, "this repeats an earlier argument")
        .also_at(first, "first given here")
        .with_help("remove one of them")
}

pub fn positional_after_named(span: Span) -> Diagnostic {
    Diagnostic::error("positional-after-named", "this argument comes after a named one")
        .at(span, "Vaab expected a name here")
        .with_help("name this one too, or move it in front of the named arguments")
        .with_note("once the order stops mattering it has to stop mattering for every argument, or a reader cannot tell which is which")
}

pub fn too_many_arguments(
    callee: &str,
    takes: usize,
    given: usize,
    span: Span,
    declared: Option<Span>,
) -> Diagnostic {
    let diagnostic = Diagnostic::error(
        "wrong-argument-count",
        format!(
            "`{callee}` takes {}, but this call gives {given}",
            counted(takes, "argument")
        ),
    )
    .at(span, "this argument has nowhere to go")
    .with_help(format!("remove it, or give `{callee}` another parameter"));

    match declared {
        Some(declared) => diagnostic.also_at(declared, format!("`{callee}` is declared here")),
        None => diagnostic,
    }
}

pub fn not_callable(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("not-callable", "this is not something that can be called")
        .at(span, format!("this is {found}"))
        .with_help("only a function can be called with `(...)`")
}

pub fn closure_parameter_count(
    wanted: usize,
    written: usize,
    span: Span,
) -> Diagnostic {
    Diagnostic::error(
        "closure-parameter-count",
        format!(
            "this closure takes {}, but it is given {}",
            counted(written, "parameter"),
            counted(wanted, "value")
        ),
    )
    .at(span, format!("this closure names {}", counted(written, "parameter")))
    .with_help(format!("write {} before the `->`", counted(wanted, "name")))
}

pub fn closure_needs_context(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "closure-needs-context",
        "Vaab cannot tell what this closure's parameters are",
    )
    .at(span, format!("nothing here says what `{name}` is"))
    .with_help(format!(
        "pass it to a function that says, or annotate the value: \
         `let f: to(Int) returns Int = {name} -> ...`"
    ))
    .with_note("Vaab infers local values but never a signature, and a closure's parameters are a signature")
}

// ---------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------

pub fn condition_not_bool(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("condition-not-bool", "a condition has to be a Bool")
        .at(span, format!("this is {found}"))
        .with_help(match found {
            Type::Int | Type::Float => "compare it, as in `count > 0`".to_string(),
            Type::Maybe(held) => {
                format!("a `maybe {held}` is not a yes or a no; take it apart with `match`")
            }
            _ => "compare it with something, so the answer is `yes` or `no`".to_string(),
        })
        .with_note("Vaab has no truthiness: only `yes` and `no` choose a branch")
}

pub fn not_iterable(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("not-iterable", "this cannot be looped over")
        .at(span, format!("this is {found}"))
        .with_help("loop over a list, a range such as `1..10`, a map, or a channel")
}

pub fn return_outside_function(span: Span) -> Diagnostic {
    Diagnostic::error("return-outside-function", "`return` is only allowed inside a function")
        .at(span, "there is nothing here to return from")
        .with_help("put this in a function, with `to something() { ... }`")
}

pub fn missing_return(callee: &str, returns: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("missing-return", format!("`{callee}` does not always return {returns}"))
        .at(span, "this body can reach its end without returning")
        .with_help(format!("finish it with `return ...`, or with {returns} as its last expression"))
}

// ---------------------------------------------------------------------------
// Missing values and failures
// ---------------------------------------------------------------------------

pub fn try_outside_fallible(
    span: Span,
    inside: Option<(&str, &Type)>,
) -> Diagnostic {
    let diagnostic = Diagnostic::error("try-outside-fallible", "`try` needs a function that can fail")
        .at(span, "there is nowhere for this failure to go");

    match inside {
        Some((name, returns)) => diagnostic
            .with_help(format!("declare the failure: `returns {returns} or fails SomeError`"))
            .with_note(format!("`{name}` returns {returns}, which cannot fail")),
        None => diagnostic
            .with_help("put this in a function that returns `T or fails E`")
            .with_note("a failure has to be handed to somebody, and at the top level of a file there is nobody to hand it to"),
    }
}

pub fn try_error_mismatch(
    found: &Type,
    wanted: &Type,
    span: Span,
    signature: Span,
) -> Diagnostic {
    Diagnostic::error("try-error-mismatch", "`try` needs the two failures to be the same type")
        .at(span, format!("this fails with {found}"))
        .also_at(signature, format!("but this function fails with {wanted}"))
        .with_help(format!(
            "take it apart with `match` and fail with a {wanted}, or declare `or fails {found}`"
        ))
        .with_note("Vaab does not convert one error type into another on its own")
}

pub fn try_needs_fallible(found: &Type, span: Span) -> Diagnostic {
    let diagnostic = Diagnostic::error("try-needs-fallible", "`try` needs something that can fail")
        .at(span, format!("this is {found}"));
    match found {
        Type::Maybe(held) => diagnostic
            .with_help(format!(
                "a `maybe {held}` is missing rather than failed; use `otherwise` or `match`"
            ))
            .with_note("`try` passes a failure on; there is no failure here to pass"),
        _ => diagnostic.with_help("remove the `try`"),
    }
}

pub fn otherwise_on_plain(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("otherwise-on-plain", "`otherwise` needs something that might not be there")
        .at(span, format!("this is {found}, which is always there"))
        .with_help("remove the `otherwise`")
        .with_note("`otherwise` supplies the fallback for a `maybe` or for something that can fail")
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

pub fn bad_operand(
    operator: &str,
    needs: &str,
    found: &Type,
    span: Span,
    other: Option<(Span, &Type)>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error("bad-operand", format!("`{operator}` needs {needs}"))
        .at(span, format!("this is {found}"));

    if let Some((other_span, other_type)) = other {
        diagnostic = diagnostic.also_at(other_span, format!("this is {other_type}"));
    }

    if matches!(found, Type::Text) && matches!(operator, "+") {
        diagnostic = diagnostic
            .with_help("to join text, put both pieces in one piece of text: `\"{first}{second}\"`");
    }
    diagnostic
}

pub fn mixed_list(first: &Type, first_span: Span, found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("mixed-list", "this list holds more than one type")
        .at(span, format!("this item is {found}"))
        .also_at(first_span, format!("the first item is {first}"))
        .with_help("make every item the same type")
        .with_note("a `list of Int` is a different type from a `list of Text`, so a list cannot be both")
}

pub fn mixed_map(
    part: &str,
    first: &Type,
    first_span: Span,
    found: &Type,
    span: Span,
) -> Diagnostic {
    Diagnostic::error("mixed-map", format!("this map's {part}s are not all the same type"))
        .at(span, format!("this {part} is {found}"))
        .also_at(first_span, format!("the first {part} is {first}"))
        .with_help(format!("make every {part} the same type"))
}

/// A bare `nothing` says a value is absent without saying what sort of value.
pub fn cannot_infer_nothing(span: Span) -> Diagnostic {
    Diagnostic::error("cannot-infer-empty", "Vaab cannot tell what this `nothing` stands for")
        .at(span, "this says a value is absent, but not what sort of value")
        .with_help("say so with an annotation: `let age: maybe Int = nothing`")
        .with_note("`nothing` is the empty half of a `maybe T`, so Vaab needs to know the `T`")
}

pub fn cannot_infer_empty(what: &str, example: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "cannot-infer-empty",
        format!("Vaab cannot tell what this empty {what} holds"),
    )
    .at(span, format!("there is nothing in this {what} to look at"))
    .with_help(format!("say so with an annotation: `{example}`"))
}

pub fn not_indexable(found: &Type, span: Span) -> Diagnostic {
    let diagnostic = Diagnostic::error(
        "not-indexable",
        format!("{} is not read with `[...]`", a(found)),
    )
    .at(span, format!("this is {found}"));
    match found {
        Type::Map { value, .. } => diagnostic
            .with_help(format!("use `.get(key)`, which gives a `maybe {value}`"))
            .with_note("a key may be absent, and `[...]` has no room to say so"),
        _ => diagnostic.with_help("only a list can be read with `[...]`"),
    }
}

pub fn bad_index(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("bad-index", "a list is indexed by an Int")
        .at(span, format!("this is {found}"))
        .with_help("use a whole number, counting from 0")
}

// ---------------------------------------------------------------------------
// Patterns and match
// ---------------------------------------------------------------------------

pub fn pattern_mismatch(found: &Type, describes: &str, span: Span) -> Diagnostic {
    Diagnostic::error("pattern-mismatch", format!("this pattern cannot match {found}"))
        .at(span, format!("this pattern is for {describes}"))
        .with_help(format!("match on the {found} itself"))
}

pub fn not_exhaustive(uncovered: &[String], subject: &Type, span: Span) -> Diagnostic {
    Diagnostic::error(
        "not-exhaustive",
        format!("this `match` does not cover {}", listed(uncovered)),
    )
    .at(span, format!("this is {subject}, which can also be {}", listed(uncovered)))
    .with_help(format!(
        "add {}, or finish with `otherwise then ...`",
        uncovered
            .iter()
            .map(|name| format!("`when {name} then ...`"))
            .collect::<Vec<String>>()
            .join(", ")
    ))
    .with_note("a `match` has to cover every case, so that adding a variant later cannot slip past")
}

pub fn not_exhaustive_open(subject: &Type, span: Span) -> Diagnostic {
    Diagnostic::error(
        "not-exhaustive",
        format!("this `match` does not cover every {subject}"),
    )
    .at(span, format!("this is {subject}, which has more values than there are arms"))
    .with_help("finish with `otherwise then ...`")
}

pub fn not_a_choice(name: &str, span: Span, known: &[String]) -> Diagnostic {
    let fallback = if known.is_empty() {
        format!("declare it with `choice {name} {{ ... }}`")
    } else {
        format!("the choices in this file are {}", listed(known))
    };
    Diagnostic::error("not-a-choice", format!("`{name}` is not a choice"))
        .at(span, "Vaab expected the name of a choice here")
        .with_help(did_you_mean(name, known, fallback))
}

pub fn unknown_variant(
    choice: &str,
    variant: &str,
    span: Span,
    known: &[String],
) -> Diagnostic {
    Diagnostic::error("unknown-variant", format!("`{choice}` has no variant `{variant}`"))
        .at(span, "this is not one of its variants")
        .with_help(did_you_mean(
            variant,
            known,
            format!("the variants of `{choice}` are {}", listed(known)),
        ))
}

pub fn wrong_variant_parts(
    variant: &str,
    carries: usize,
    given: usize,
    span: Span,
) -> Diagnostic {
    Diagnostic::error(
        "wrong-variant-parts",
        format!(
            "`{variant}` carries {}, but this gives {given}",
            counted(carries, "value")
        ),
    )
    .at(span, format!("this names {}", counted(given, "value")))
    .with_help(if carries == 0 {
        format!("write `{variant}` on its own")
    } else {
        format!("give it {}", counted(carries, "value"))
    })
}

// ---------------------------------------------------------------------------
// Abilities
// ---------------------------------------------------------------------------

pub fn undefined_ability(name: &str, span: Span, known: &[String]) -> Diagnostic {
    Diagnostic::error("undefined-ability", format!("`{name}` is not an ability"))
        .at(span, "Vaab expected the name of an ability here")
        .with_help(did_you_mean(
            name,
            known,
            format!("declare it with `ability {name} {{ ... }}`, or check the spelling"),
        ))
}

pub fn missing_ability_function(
    provider: &str,
    ability: &str,
    required: &Signature,
    name: &str,
    span: Span,
    required_at: Span,
) -> Diagnostic {
    Diagnostic::error(
        "missing-ability-function",
        format!("`{provider}` does not provide `{name}`"),
    )
    .at(span, format!("this says `{provider}` can `{ability}`"))
    .also_at(required_at, format!("`{ability}` requires `{name}` here"))
    .with_help(format!("add `{}` to `{provider}`", required.describe(name)))
}

pub fn ability_signature_mismatch(
    provider: &str,
    ability: &str,
    name: &str,
    found: &Signature,
    required: &Signature,
    span: Span,
    required_at: Span,
) -> Diagnostic {
    Diagnostic::error(
        "ability-signature-mismatch",
        format!("`{provider}`'s `{name}` does not match `{ability}`"),
    )
    .at(span, format!("this is `{}`", found.describe(name)))
    .also_at(required_at, format!("`{ability}` asks for `{}`", required.describe(name)))
    .with_help("change the signature so the two match exactly")
    .with_note(format!(
        "anything holding a `{ability}` calls through the signature `{ability}` promised, \
         so the two cannot differ"
    ))
}

// ---------------------------------------------------------------------------
// Channels and tasks
// ---------------------------------------------------------------------------

pub fn not_a_channel(what: &str, found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("not-a-channel", format!("`{what}` needs a channel"))
        .at(span, format!("this is {found}"))
        .with_help("make one with `Channel.new(of: Text, size: 4)`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_read_as_english() {
        assert_eq!(listed(&["a".to_string()]), "`a`");
        assert_eq!(listed(&["a".to_string(), "b".to_string()]), "`a` and `b`");
        assert_eq!(
            listed(&["a".to_string(), "b".to_string(), "c".to_string()]),
            "`a`, `b` and `c`"
        );
    }

    #[test]
    fn a_near_miss_is_suggested_and_a_wild_guess_is_not() {
        let known = ["balance", "owner"];
        assert_eq!(nearest("blance", known.into_iter()), Some("balance".to_string()));
        assert_eq!(nearest("ownr", known.into_iter()), Some("owner".to_string()));
        assert_eq!(nearest("telephone", known.into_iter()), None);
    }

    #[test]
    fn a_short_name_needs_a_closer_match_to_be_worth_suggesting() {
        // With one keystroke of leeway, `ab` should not be "did you mean `xyz`?".
        assert_eq!(nearest("ab", ["xyz"].into_iter()), None);
        assert_eq!(nearest("ab", ["abc"].into_iter()), Some("abc".to_string()));
    }

    #[test]
    fn the_specifications_two_fixed_messages_are_word_for_word() {
        let mismatch = mismatch(&Type::Int, &Type::maybe(Type::Int), Span::new(0, 1));
        assert_eq!(mismatch.message, "expected Int, found maybe Int");

        let missing =
            missing_fields("Account.new", "Account", &["owner".to_string()], Span::new(0, 1));
        assert_eq!(missing.message, "Account.new is missing `owner`.");
    }

    #[test]
    fn counting_gets_its_plurals_right() {
        assert_eq!(counted(1, "argument"), "1 argument");
        assert_eq!(counted(2, "argument"), "2 arguments");
        assert_eq!(counted(0, "value"), "0 values");
    }
}

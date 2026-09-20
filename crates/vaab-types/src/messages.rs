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

pub fn not_json(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error("not-json", format!("{} cannot be turned into JSON", a(found)))
        .at(span, format!("this is {found}"))
        .with_help("only types that `can Json` may be passed to `to_json`")
        .with_note("a type `can Json` when every field, variant and item it holds can Json too")
}

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
    let diagnostic = Diagnostic::error("not-a-channel", format!("`{what}` needs a channel"))
        .at(span, format!("this is {found}"));
    match found {
        // A `shared` is the other way tasks share, and mistaking one for the other
        // is an easy thing to do, so say where its value is kept instead.
        Type::Shared(held) => diagnostic.with_help(format!(
            "a `shared {held}` is read with `.value` and changed with `.update(...)`, \
             with no waiting either way"
        )),
        _ => diagnostic.with_help("make one with `Channel.new(of: Text, size: 4)`"),
    }
}

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

/// Why a `pure` function broke its promise. `effect` is a short phrase naming what
/// happened, as in "calls `print`" or "starts a task".
pub enum PureViolation {
    Effect(&'static str),
    Calls(String),
    ChangesOutside,
}

/// A `pure` function did something the promise rules out.
pub fn not_pure(
    function: &str,
    why: PureViolation,
    at: Span,
    declared_at: Span,
) -> Diagnostic {
    let (message, at_label, help) = match why {
        PureViolation::Effect(effect) => (
            format!("`{function}` is declared `pure`, so it cannot {effect}"),
            pure_effect_label(effect),
            pure_effect_help(effect),
        ),
        PureViolation::Calls(callee) => (
            format!("`{function}` is declared `pure`, so it cannot call `{callee}`"),
            format!("this calls `{callee}`, which is not declared `pure`"),
            format!(
                "declare `{callee}` as `pure` too, or do this work before calling `{function}`"
            ),
        ),
        PureViolation::ChangesOutside => (
            format!(
                "`{function}` is declared `pure`, so it cannot change a value from outside itself"
            ),
            "this changes a value declared outside this function".to_string(),
            "pass the new value back to the caller instead, or work on a local copy".to_string(),
        ),
    };

    Diagnostic::error("not-pure", message)
        .at(at, at_label)
        .also_at(declared_at, format!("`{function}` is declared `pure` here"))
        .with_help(help)
}

fn pure_effect_label(effect: &str) -> String {
    match effect {
        "call `print`" => "this calls `print`".to_string(),
        "call `read_file`" => "this calls `read_file`".to_string(),
        "call a function held in a value" => "this calls a function held in a value".to_string(),
        "start a task" => "this starts a task".to_string(),
        "wait for a task" => "this waits for a task".to_string(),
        "wait for tasks" => "this waits for tasks".to_string(),
        "send on a channel" => "this sends on a channel".to_string(),
        "close a channel" => "this closes a channel".to_string(),
        "wait for a value on a channel" => "this waits for a value on a channel".to_string(),
        "wait for something to become ready" => {
            "this waits for something to become ready".to_string()
        }
        "build a channel" => "this builds a channel".to_string(),
        "build a `shared` value" => "this builds a `shared` value".to_string(),
        "read a `shared` value" => "this reads a `shared` value".to_string(),
        "change a `shared` value" => "this changes a `shared` value".to_string(),
        other => format!("this {other}"),
    }
}

fn pure_effect_help(effect: &str) -> String {
    match effect {
        "call `print`" => "use `print` before or after the `pure` function, not inside it",
        "call `read_file`" => {
            "read the file before calling the `pure` function, and pass the text in"
        }
        "call a function held in a value" => {
            "call a function declared with `pure to` by name instead, since a function \
             held in a value carries no record of whether it is `pure`"
        }
        "build a channel" | "build a `shared` value" => {
            "build channels and `shared` values outside the `pure` function, and pass them in"
        }
        "read a `shared` value" | "change a `shared` value" => {
            "read or change a `shared` value outside the `pure` function, and pass the value in"
        }
        "start a task" | "wait for a task" | "wait for tasks" => {
            "start tasks and wait for them outside the `pure` function"
        }
        "send on a channel" | "close a channel" | "wait for a value on a channel" => {
            "use channels outside the `pure` function"
        }
        "wait for something to become ready" => {
            "waiting needs the clock or a channel, and neither belongs inside a `pure` function"
        }
        _ => "move this work outside the `pure` function",
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// Web server (phase 6)
// ---------------------------------------------------------------------------

pub fn serve_port_must_be_a_number(span: Span) -> Diagnostic {
    Diagnostic::error(
        "serve-port-must-be-a-number",
        "the port in `serve on port ...` must be a whole number",
    )
    .at(span, "this is not a plain whole number")
    .with_help("write something like `serve on port 8080 { ... }`")
}

pub fn unknown_http_method(method: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "unknown-http-method",
        format!("`{method}` is not an HTTP method Vaab knows"),
    )
    .at(span, format!("`{method}` was written here"))
    .with_help("use `get`, `post`, `put`, `patch` or `delete`")
}

pub fn route_param_type(found: &Type, span: Span) -> Diagnostic {
    Diagnostic::error(
        "route-param-type",
        format!("a route parameter may be `Int` or `Text`, not {found}"),
    )
    .at(span, format!("this parameter is declared as {found}"))
}

pub fn reply_outside_route(span: Span) -> Diagnostic {
    Diagnostic::error(
        "reply-outside-route",
        "`reply` may only appear inside a route body or error handler",
    )
    .at(span, "`reply` was written here")
}

pub fn cannot_json(span: Span) -> Diagnostic {
    Diagnostic::error(
        "cannot-json",
        "this value cannot be turned into JSON for the response body",
    )
    .at(span, "Vaab can only reply with JSON-serialisable values")
    .with_help("use text, numbers, lists, maps, records or choices whose fields can all be JSON")
}

// ---------------------------------------------------------------------------
// Sendability: what may cross between tasks
// ---------------------------------------------------------------------------

/// The specification fixes the wording of this one, and it must not drift:
///
/// > `` `total` can change, so a task cannot use it. Use `Shared` instead. ``
///
/// The first sentence is the message and the second is the help, which is where
/// the house style keeps the thing to type.
pub fn changing_captured_by_task(
    name: &str,
    declared: &Type,
    used_at: Span,
    declared_at: Span,
) -> Diagnostic {
    Diagnostic::error(
        "changing-captured-by-task",
        format!("`{name}` can change, so a task cannot use it."),
    )
    .at(used_at, format!("this task uses `{name}`, which lives outside it"))
    .also_at(declared_at, format!("`{name}` is declared `changing` here"))
    .with_help(instead_of_changing(name, declared))
    .with_note(why_changing_is_unsafe(name, declared))
}

/// The same rule broken one step away: the task uses a closure, and the closure is
/// the one holding on to the changing variable.
pub fn changing_captured_through(
    name: &str,
    holder: &str,
    declared: &Type,
    used_at: Span,
    read_at: Span,
    declared_at: Span,
) -> Diagnostic {
    Diagnostic::error(
        "changing-captured-by-task",
        format!("`{name}` can change, so a task cannot use it."),
    )
    .at(used_at, format!("this task uses `{holder}`, which holds on to `{name}`"))
    .also_at(read_at, format!("`{holder}` reads `{name}` here"))
    .also_at(declared_at, format!("`{name}` is declared `changing` here"))
    .with_help(instead_of_changing(name, declared))
    .with_note(format!(
        "a closure keeps the variable it was written beside rather than a copy of it, so \
         sending `{holder}` into a task would send `{name}` along with it"
    ))
}

/// What to do instead of letting a task have a changing variable. A name that
/// already holds a `shared` needs no `Shared.new`; it needs to stop being changing.
fn instead_of_changing(name: &str, declared: &Type) -> String {
    match declared {
        Type::Shared(_) => format!(
            "drop the `changing`: a `shared` is changed with `{name}.update(value -> ...)`, \
             never by assignment"
        ),
        _ => format!(
            "use `Shared` instead: `let {name} = Shared.new(...)`, and change it inside the \
             task with `{name}.update(value -> ...)`"
        ),
    }
}

fn why_changing_is_unsafe(name: &str, declared: &Type) -> String {
    match declared {
        // Changing what is *inside* a `shared` from several tasks is the whole
        // point of one. Changing which `shared` the name stands for is not.
        Type::Shared(_) => format!(
            "what a `shared` holds is safe to change from any number of tasks, but the name \
             is not: assigning to `{name}` would leave the tasks looking at different values"
        ),
        _ => format!(
            "two tasks could change `{name}` at the same moment, and then neither would see \
             what the other did; if the task only needs the value `{name}` holds right now, \
             copy it into a fixed name first and use that"
        ),
    }
}

/// A value a task reaches for whose *type* cannot cross, whoever is holding it.
pub fn unsendable_capture(
    name: &str,
    found: &Type,
    culprit: &Type,
    reached_by: Option<&str>,
    used_at: Span,
    declared_at: Span,
) -> Diagnostic {
    Diagnostic::error("unsendable-capture", format!("a task cannot be given `{name}`"))
        .at(used_at, format!("this task uses `{name}`, which is {}", spelled(found)))
        .also_at(declared_at, format!("`{name}` is declared here"))
        .with_help(pass_something_else(culprit, reached_by))
        .with_note(in_the_way(culprit, reached_by))
}

/// `self` inside a task, where the type `self` stands for cannot cross.
pub fn unsendable_self(
    found: &Type,
    culprit: &Type,
    reached_by: Option<&str>,
    used_at: Span,
) -> Diagnostic {
    Diagnostic::error("unsendable-capture", "a task cannot be given `self`")
        .at(used_at, format!("this task uses `self`, which is {}", spelled(found)))
        .with_help("take the parts the task needs out of `self` first, and let it use those")
        .with_note(in_the_way(culprit, reached_by))
}

/// A task's answer travels back out to whoever waits for it.
pub fn unsendable_task_result(
    found: &Type,
    culprit: &Type,
    reached_by: Option<&str>,
    span: Span,
) -> Diagnostic {
    Diagnostic::error(
        "unsendable-value",
        format!("a task cannot hand back {}", spelled(found)),
    )
    .at(
        span,
        format!(
            "this task ends with {}, which travels back to whoever waits for it",
            spelled(found)
        ),
    )
    .with_help(pass_something_else(culprit, reached_by))
    .with_note(in_the_way(culprit, reached_by))
}

pub fn unsendable_sent_value(
    found: &Type,
    culprit: &Type,
    reached_by: Option<&str>,
    span: Span,
) -> Diagnostic {
    Diagnostic::error("unsendable-value", format!("{} cannot be sent", spelled(found)))
        .at(span, format!("this is {}", spelled(found)))
        .with_help(pass_something_else(culprit, reached_by))
        .with_note(in_the_way(culprit, reached_by))
}

pub fn unsendable_shared_value(
    found: &Type,
    culprit: &Type,
    reached_by: Option<&str>,
    span: Span,
) -> Diagnostic {
    Diagnostic::error(
        "unsendable-value",
        format!("a `shared` cannot hold {}", spelled(found)),
    )
    .at(span, format!("this is {}, which every task sharing it would hold", spelled(found)))
    .with_help(pass_something_else(culprit, reached_by))
    .with_note(in_the_way(culprit, reached_by))
}

/// `Channel.new(of: T)` where a `T` could never safely cross.
pub fn unsendable_channel(
    carries: &Type,
    culprit: &Type,
    reached_by: Option<&str>,
    span: Span,
) -> Diagnostic {
    Diagnostic::error(
        "unsendable-channel",
        format!("a channel cannot carry {}", spelled(carries)),
    )
    .at(span, format!("this asks for a channel of {carries}"))
    .with_help(pass_something_else(culprit, reached_by))
    .with_note(in_the_way(culprit, reached_by))
}

/// A type as a noun phrase. A function type is quoted as the code it is, because
/// "a to(Int) returns Int" is not a thing anybody would say out loud.
fn spelled(declared: &Type) -> String {
    match declared {
        Type::Function(_) => format!("the function value `{declared}`"),
        other => a(other),
    }
}

/// What is standing in the way, as one sentence a `note` can hold.
///
/// A function value is the only thing that is ever really in the way: every other
/// type is judged by what it holds, so the walk ends at a function or at nothing.
fn in_the_way(culprit: &Type, reached_by: Option<&str>) -> String {
    let because = match culprit {
        Type::Function(_) => "a function value holds on to whatever was around it when it was \
             written, which Vaab has no way to look inside",
        _ => "that cannot be shared between tasks",
    };

    match reached_by {
        Some(path) => format!("{path} is `{culprit}`; {because}"),
        None => because.to_string(),
    }
}

fn pass_something_else(culprit: &Type, reached_by: Option<&str>) -> String {
    match (culprit, reached_by) {
        // The value itself is the function. A function *declared* with `to` is
        // never carried anywhere, so calling one by name always works.
        (Type::Function(_), None) => "work the answer out first and pass that instead, or \
             declare the work with `to` and call it by name, since a declared function is \
             never carried across"
            .to_string(),
        // The function is buried in a field, so the way out is to unpack it.
        (Type::Function(_), Some(_)) => {
            "pass the parts of it that are needed, rather than the whole value".to_string()
        }
        _ => "pass a value that two tasks could not disagree about".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// `what` says what the offending line does: "waits for a value to arrive".
pub fn update_cannot_wait(what: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "update-cannot-wait",
        "the change given to `.update` cannot wait for anything",
    )
    .at(span, format!("this {what}"))
    .with_help(
        "work the new value out before the `.update`, and hand the finished value in: \
         `let next = ...` and then `.update(value -> next)`",
    )
    .with_note(
        "`.update` holds the shared value while the change runs, so a wait inside it would \
         keep every other task away from the value until the wait ended — and it may be one \
         of those tasks the wait is for",
    )
}

// ---------------------------------------------------------------------------
// `together` and `select`
// ---------------------------------------------------------------------------

pub fn together_waits_for_nothing(span: Span) -> Diagnostic {
    Diagnostic::error("together-waits-for-nothing", "this `together` has nothing to wait for")
        .at(span, "nothing in here starts a task or calls anything that could")
        .with_help("start the work inside it, with `start { ... }`")
        .with_note(
            "`together` is how a group of tasks is waited for, and it cancels the rest if one \
             of them fails; with no task to watch it does nothing at all",
        )
}

pub fn select_with_no_arms(span: Span) -> Diagnostic {
    Diagnostic::error("select-with-no-arms", "this `select` has nothing to wait for")
        .at(span, "there are no `when` arms here")
        .with_help("add an arm, as in `when receive from inbox as message { ... }`")
        .with_note(
            "`select` waits for whichever of its arms is ready first, so with no arms there is \
             nothing that could ever become ready",
        )
}

/// `known` holds both spellings of every unit, so that a near miss on either is
/// offered; the fallback names only the plurals, because listing eight words helps
/// nobody.
pub fn unknown_time_unit(written: &str, span: Span, known: &[String]) -> Diagnostic {
    Diagnostic::error(
        "unknown-time-unit",
        format!("`{written}` is not a unit of time Vaab knows"),
    )
    .at(span, "Vaab expected a unit of time here")
    .with_help(did_you_mean(
        written,
        known,
        "write `milliseconds`, `seconds`, `minutes` or `hours` — or the singular of any of \
         them, as in `timeout after 1 second`"
            .to_string(),
    ))
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
    fn the_specifications_wording_for_a_captured_changing_value_is_word_for_word() {
        let captured = changing_captured_by_task(
            "total",
            &Type::Int,
            Span::new(0, 1),
            Span::new(2, 3),
        );
        assert_eq!(captured.message, "`total` can change, so a task cannot use it.");
        // The specification's second sentence, "Use `Shared` instead.", is the help.
        let help = captured.help.unwrap_or_default();
        assert!(help.contains("Shared"), "{help}");
    }

    #[test]
    fn a_function_type_is_named_as_the_code_it_is() {
        let function = Type::function(vec![Type::Int], Type::Int);
        assert_eq!(spelled(&function), "the function value `to(Int) returns Int`");
        // Everything else keeps the article it already reads best with.
        assert_eq!(spelled(&Type::Int), "an Int");
        assert_eq!(spelled(&Type::named("Job")), "a `Job`");
    }

    #[test]
    fn the_way_to_an_unsendable_value_is_spelled_out_when_there_is_one() {
        let function = Type::function(vec![Type::Int], Type::Int);
        let direct = in_the_way(&function, None);
        assert!(direct.starts_with("a function value holds on to"), "{direct}");

        let buried = in_the_way(&function, Some("`Job`'s field `work`"));
        assert!(buried.starts_with("`Job`'s field `work` is `to(Int) returns Int`;"), "{buried}");
    }

    #[test]
    fn a_name_already_holding_a_shared_is_told_to_drop_the_changing_instead() {
        let plain = instead_of_changing("total", &Type::Int);
        assert!(plain.contains("Shared.new"), "{plain}");

        let already_shared = instead_of_changing("counter", &Type::shared(Type::Int));
        assert!(already_shared.contains("drop the `changing`"), "{already_shared}");
        assert!(!already_shared.contains("Shared.new"), "{already_shared}");
    }

    #[test]
    fn counting_gets_its_plurals_right() {
        assert_eq!(counted(1, "argument"), "1 argument");
        assert_eq!(counted(2, "argument"), "2 arguments");
        assert_eq!(counted(0, "value"), "0 values");
    }
}

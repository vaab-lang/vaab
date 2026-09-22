//! The standard library this phase builds.
//!
//! There is one test per name in the checker's prelude that has an implementation
//! behind it, so a signature added to `vaab-types` with nothing to run would show
//! up here as a missing test rather than as a program that quietly does nothing.

mod support;

use support::{output, printed};

// ---------------------------------------------------------------------------
// print
// ---------------------------------------------------------------------------

#[test]
fn print_shows_a_value_of_any_type_at_all() {
    let source = "print(1)\nprint(\"two\")\nprint(yes)\nprint([1])\nprint((1, 2))\n";
    assert_eq!(output(source), ["1", "two", "yes", "[1]", "(1, 2)"]);
}

#[test]
fn print_shows_text_as_it_is_and_quotes_it_inside_a_list() {
    assert_eq!(output("print(\"a\")\nprint([\"a\"])\n"), ["a", "[\"a\"]"]);
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

#[test]
fn upper_and_lower_change_the_case() {
    assert_eq!(output("print(\"ada\".upper())\nprint(\"ADA\".lower())\n"), ["ADA", "ada"]);
}

#[test]
fn contains_says_whether_one_piece_of_text_is_inside_another() {
    let source = "print(\"ada@example.com\".contains(\"@\"))\nprint(\"nope\".contains(\"@\"))\n";
    assert_eq!(output(source), ["yes", "no"]);
}

#[test]
fn text_is_empty_when_it_has_no_characters() {
    assert_eq!(output("print(\"\".is_empty)\nprint(\"a\".is_empty)\n"), ["yes", "no"]);
}

#[test]
fn the_length_of_text_counts_characters_rather_than_bytes() {
    assert_eq!(printed("print(\"héllo\".length)\n"), "5");
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[test]
fn map_gives_back_a_list_the_same_length_as_the_one_it_walked() {
    assert_eq!(printed("print([1, 2, 3].map(n -> n * 2))\n"), "[2, 4, 6]");
}

#[test]
fn map_over_an_empty_list_gives_an_empty_list() {
    assert_eq!(printed("let items: list of Int = []\nprint(items.map(n -> n * 2))\n"), "[]");
}

#[test]
fn each_runs_its_closure_once_for_every_item() {
    assert_eq!(output("[1, 2].each(n -> print(n))\n"), ["1", "2"]);
}

#[test]
fn a_list_is_empty_when_it_holds_nothing() {
    let source = "let items: list of Int = []\nprint(items.is_empty)\nprint([1].is_empty)\n";
    assert_eq!(output(source), ["yes", "no"]);
}

#[test]
fn count_says_how_many_items_a_list_holds() {
    assert_eq!(printed("print([1, 2, 3].count)\n"), "3");
}

#[test]
fn the_first_of_a_list_is_a_maybe_because_the_list_may_be_empty() {
    let source = "let items: list of Int = []\nprint([1, 2].first)\nprint(items.first)\n";
    assert_eq!(output(source), ["found 1", "nothing"]);
}

#[test]
fn join_puts_a_separator_between_every_piece_of_text() {
    assert_eq!(printed("print([\"a\", \"b\", \"c\"].join(\", \"))\n"), "a, b, c");
}

#[test]
fn joining_one_piece_of_text_adds_no_separator() {
    assert_eq!(printed("print([\"only\"].join(\", \"))\n"), "only");
}

// ---------------------------------------------------------------------------
// Maps
// ---------------------------------------------------------------------------

#[test]
fn get_hands_back_a_maybe_because_the_key_may_not_be_there() {
    let source = "let ages = {\"Ada\": 36}\nprint(ages.get(\"Ada\"))\nprint(ages.get(\"Grace\"))\n";
    assert_eq!(output(source), ["found 36", "nothing"]);
}

#[test]
fn a_map_is_empty_when_it_holds_nothing() {
    let source = "let ages: map of Text to Int = {}\nprint(ages.is_empty)\n\
                  print({\"a\": 1}.is_empty)\n";
    assert_eq!(output(source), ["yes", "no"]);
}

#[test]
fn count_says_how_many_entries_a_map_holds() {
    assert_eq!(printed("print({\"a\": 1, \"b\": 2}.count)\n"), "2");
}

#[test]
fn keys_come_back_in_the_order_they_were_first_put_in() {
    assert_eq!(printed("print({\"b\": 1, \"a\": 2}.keys)\n"), "[\"b\", \"a\"]");
}

#[test]
fn a_map_can_be_keyed_by_something_other_than_text() {
    assert_eq!(printed("let names = {1: \"one\"}\nprint(names.get(1) otherwise \"?\")\n"), "one");
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

#[test]
fn abs_drops_the_sign_of_a_whole_number_and_of_a_decimal() {
    // A method binds tighter than a minus sign, so the brackets are the ones
    // anyone writing this would have to put in.
    assert_eq!(output("print((-7).abs())\nprint((-1.5).abs())\n"), ["7", "1.5"]);
}

#[test]
fn min_and_max_pick_between_two_whole_numbers() {
    assert_eq!(output("print(3.min(7))\nprint(3.max(7))\n"), ["3", "7"]);
}

#[test]
fn a_whole_number_becomes_a_decimal_only_when_asked() {
    assert_eq!(printed("print(3.to_float() / 2.0)\n"), "1.5");
}

#[test]
fn rounding_goes_to_the_nearest_whole_number() {
    assert_eq!(output("print(2.4.round())\nprint(2.6.round())\n"), ["2", "3"]);
}

#[test]
fn a_half_rounds_away_from_zero_the_way_it_is_taught_at_school() {
    assert_eq!(output("print(2.5.round())\nprint((-2.5).round())\n"), ["3", "-3"]);
}

// ---------------------------------------------------------------------------
// File I/O, time and JSON
// ---------------------------------------------------------------------------

#[test]
fn now_returns_seconds_since_1970() {
    let seconds = printed("print(now())\n").parse::<i64>().expect("a whole number");
    assert!(seconds > 1_600_000_000);
}

#[test]
fn to_json_turns_a_map_into_text() {
    assert_eq!(printed("print(to_json({\"a\": 1}))\n"), "{\"a\":1}");
}

#[test]
fn store_round_trips_text_by_key() {
    let path = std::env::temp_dir().join("vaab-store-test.kv");
    let _ = std::fs::remove_file(&path);
    let path = path.display();
    let source = format!(
        "match Store.open(\"{path}\") {{\n\
         when success store then {{\n\
             match store.set(\"greeting\", \"hello\") {{\n\
             when success _ then match store.get(\"greeting\") {{\n\
                 when found value then print(value)\n\
                 when nothing then print(\"missing\")\n\
             }}\n\
             when failure _ then print(\"set failed\")\n\
             }}\n\
         }}\n\
         when failure _ then print(\"open failed\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "hello");
}

#[test]
fn store_keys_lists_every_key_with_a_prefix() {
    let path = std::env::temp_dir().join("vaab-store-keys-test.kv");
    let _ = std::fs::remove_file(&path);
    let path = path.display();
    let source = format!(
        "match Store.open(\"{path}\") {{\n\
         when success store then {{\n\
             match store.set(\"a:1\", \"one\") {{\n\
             when success _ then match store.set(\"a:2\", \"two\") {{\n\
             when success _ then match store.set(\"b:1\", \"three\") {{\n\
             when success _ then match store.keys(\"a:\") {{\n\
                 when success keys then print(keys.join(\",\"))\n\
                 when failure _ then print(\"keys failed\")\n\
             }}\n\
             when failure _ then print(\"set failed\")\n\
             }}\n\
             when failure _ then print(\"set failed\")\n\
             }}\n\
             when failure _ then print(\"set failed\")\n\
             }}\n\
         }}\n\
         when failure _ then print(\"open failed\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "a:1,a:2");
}

#[test]
fn query_filters_sqlite_rows_with_sea_query() {
    let path = std::env::temp_dir().join("vaab-query-db-test.sqlite");
    let _ = std::fs::remove_file(&path);
    let path = path.display();
    let source = format!(
        "match Db.connect(\"sqlite:{path}\") {{\n\
         when success db then {{\n\
             let empty: list of Text = []\n\
             match db.execute(\"create table items (id text primary key, kind text not null)\", empty) {{\n\
             when success _ then match db.from(\"items\").insert({{\"id\": \"1\", \"kind\": \"a\"}}) {{\n\
             when success _ then match db.from(\"items\").insert({{\"id\": \"2\", \"kind\": \"b\"}}) {{\n\
             when success _ then match db.from(\"items\").where_eq(\"kind\", \"a\").count() {{\n\
                 when success n then print(n)\n\
                 when failure _ then print(\"count failed\")\n\
             }}\n\
             when failure _ then print(\"insert failed\")\n\
             }}\n\
             when failure _ then print(\"insert failed\")\n\
             }}\n\
             when failure _ then print(\"schema failed\")\n\
             }}\n\
         }}\n\
         when failure _ then print(\"connect failed\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "1");
}

#[test]
fn query_filters_store_rows_by_value() {
    let path = std::env::temp_dir().join("vaab-query-store-test.kv");
    let _ = std::fs::remove_file(&path);
    let path = path.display();
    let source = format!(
        "match Store.open(\"{path}\") {{\n\
         when success store then {{\n\
             match store.from(\"k:\").insert({{\"key\": \"k:1\", \"value\": \"yes\"}}) {{\n\
             when success _ then match store.from(\"k:\").insert({{\"key\": \"k:2\", \"value\": \"no\"}}) {{\n\
             when success _ then match store.from(\"k:\").where_eq(\"value\", \"yes\").count() {{\n\
                 when success n then print(n)\n\
                 when failure _ then print(\"count failed\")\n\
             }}\n\
             when failure _ then print(\"insert failed\")\n\
             }}\n\
             when failure _ then print(\"insert failed\")\n\
             }}\n\
         }}\n\
         when failure _ then print(\"open failed\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "1");
}

#[test]
fn store_delete_after_insert_does_not_deadlock() {
    // Regression: remove() used to hold the overlay mutex across a second lock
    // when the key was already cached, hanging Clear / delete forever.
    let path = std::env::temp_dir().join("vaab-store-delete-overlay-test.kv");
    let _ = std::fs::remove_file(&path);
    let path = path.display();
    let source = format!(
        "match Store.open(\"{path}\") {{\n\
         when success store then {{\n\
             match store.from(\"t:\").insert({{\"key\": \"t:1\", \"title\": \"a\", \"done\": \"no\"}}) {{\n\
             when success _ then match store.from(\"t:\").insert({{\"key\": \"t:2\", \"title\": \"b\", \"done\": \"no\"}}) {{\n\
             when success _ then match store.from(\"t:\").delete() {{\n\
                 when success n then print(n)\n\
                 when failure _ then print(\"delete failed\")\n\
             }}\n\
             when failure _ then print(\"insert failed\")\n\
             }}\n\
             when failure _ then print(\"insert failed\")\n\
             }}\n\
         }}\n\
         when failure _ then print(\"open failed\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "2");
}

#[test]
fn read_file_returns_the_contents_of_a_file() {
    let path = std::env::temp_dir().join("vaab-read-file-test.txt");
    std::fs::write(&path, "hello from disk").expect("write temp file");
    let path = path.display();
    let source = format!(
        "match read_file(\"{path}\") {{\n\
         when success text then print(text)\n\
         when failure _ then print(\"missing\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "hello from disk");
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

#[test]
fn a_memory_logger_keeps_info_and_above() {
    let source = "\
let log = Logger.memory()\n\
log.set_level(\"info\")\n\
log.set_format(\"text\")\n\
log.debug(\"hidden\")\n\
log.info(\"shown\")\n\
print(log.lines.count)\n\
";
    assert_eq!(printed(source), "1");
}

#[test]
fn a_logger_can_write_json_with_fields() {
    let source = "\
let log = Logger.memory()\n\
log.set_level(\"debug\")\n\
log.set_format(\"json\")\n\
log.write(\"warn\", \"slow\", {\"ms\": \"42\"})\n\
print(log.lines.first otherwise \"\")\n\
";
    let line = printed(source);
    assert!(line.contains("\"level\":\"warn\""), "{line}");
    assert!(line.contains("\"message\":\"slow\""), "{line}");
    assert!(line.contains("\"ms\":\"42\""), "{line}");
}

#[test]
fn a_file_logger_appends_and_multi_fans_out() {
    let path = std::env::temp_dir().join("vaab-logger-test.log");
    let _ = std::fs::remove_file(&path);
    let path = path.display();
    let source = format!(
        "match Logger.file(\"{path}\") {{\n\
         when success file then {{\n\
             let memory = Logger.memory()\n\
             memory.set_level(\"debug\")\n\
             file.set_level(\"debug\")\n\
             let both = Logger.multi([memory, file])\n\
             both.info(\"hello\")\n\
             print(memory.lines.count)\n\
         }}\n\
         when failure _ then print(\"open failed\")\n\
         }}\n"
    );
    assert_eq!(printed(&source), "1");
    let contents = std::fs::read_to_string(std::env::temp_dir().join("vaab-logger-test.log"))
        .expect("log file");
    assert!(contents.contains("hello"), "{contents}");
}


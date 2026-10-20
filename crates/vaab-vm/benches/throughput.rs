//! Replay a generated JSON-lines fixture through decode, a rolling mean, a
//! batched SQLite transaction, and a hub broadcast.
//!
//! Run with: `cargo bench -p vaab-vm --bench throughput --offline`

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use vaab_syntax::parse;
use vaab_types::check;
use vaab_vm::{prepare, run, Output};

const EVENTS: usize = 4_000;

fn main() {
    let fixture = write_fixture();
    let lines = fs::read_to_string(&fixture).expect("the fixture should be readable");
    let records: Vec<&str> = lines.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(records.len(), EVENTS);

    let db = fixture.with_file_name("throughput-events.sqlite");
    let _ = fs::remove_file(&db);
    let _ = fs::remove_file(path_with_suffix(&db, "-wal"));
    let _ = fs::remove_file(path_with_suffix(&db, "-shm"));

    let source = program(&records, &db);
    let parsed = parse(&source);
    if parsed.has_errors() {
        panic!("bench source does not parse: {:?}", parsed.diagnostics);
    }
    let checked = check(&parsed.module).expect("bench source should type-check");
    let mut world = prepare(&parsed.module, &checked, Output::collected());

    let started = Instant::now();
    run(&mut world).expect("the replay should finish");
    let elapsed = started.elapsed();

    let printed = world.output.lines();
    let expected_count = format!("count {EVENTS}");
    let count_line = printed.iter().find(|line| line.starts_with("count "));
    assert_eq!(
        count_line.map(String::as_str),
        Some(expected_count.as_str()),
        "the replay should update the rolling mean once per event, got {printed:?}"
    );
    let mut samples_ms: Vec<i64> = printed
        .iter()
        .filter_map(|line| line.parse::<i64>().ok())
        .collect();
    assert_eq!(
        samples_ms.len(),
        EVENTS,
        "expected one duration per event, got {} lines: {printed:?}",
        samples_ms.len()
    );
    samples_ms.sort_unstable();
    let p99_ms = percentile(&samples_ms, 0.99);
    let events_per_sec = EVENTS as f64 / elapsed.as_secs_f64();

    println!("events        {EVENTS}");
    println!("elapsed_ms    {:.2}", elapsed.as_secs_f64() * 1000.0);
    println!("events_per_s  {events_per_sec:.1}");
    println!("p99_ms        {p99_ms}");
}

fn write_fixture() -> PathBuf {
    let path = std::env::temp_dir().join("vaab-throughput-events.jsonl");
    let mut body = String::with_capacity(EVENTS * 32);
    for id in 1..=EVENTS {
        let amount = 10.0 + (id as f64) * 0.25;
        body.push_str(&format!("{{\"id\":{id},\"amount\":{amount}}}\n"));
    }
    fs::write(&path, body).expect("the fixture should be writable");
    path
}

fn program(records: &[&str], db: &std::path::Path) -> String {
    let listed: String = records
        .iter()
        .map(|line| format!("\"{}\"", escape_text(line)))
        .collect::<Vec<_>>()
        .join(", ");
    let db = db.display();
    format!(
        r#"
type Event {{
    id: Int
    amount: Float
}}

type Mean {{
    total: Float
    count: Int
}}

to replay() returns Int or fails DbError {{
    let stats = Shared.new(Mean.new(total: 0.0, count: 0))
    let hub = Hub.new(of: Text)
    let lines = [{listed}]
    let db = try Db.connect("sqlite:{db}")
    let empty: list of Text = []
    try db.execute("create table if not exists events (id integer primary key, amount real)", empty)
    try db.transaction {{
        for each line in lines {{
            let started = Clock.monotonic_millis()
            match from_json(line, as: Event) {{
                when success event then {{
                    stats.update(mean -> Mean.new(total: mean.total + event.amount, count: mean.count + 1))
                    match db.execute("insert into events (id, amount) values ({{event.id}}, {{event.amount}})", empty) {{
                        when success _ then {{}}
                        when failure _ then {{}}
                    }}
                    hub.broadcast(to_json(event))
                }}
                when failure _ then {{}}
            }}
            print(Clock.monotonic_millis() - started)
        }}
        0
    }}
    print("count {{stats.value.count}}")
    return 0
}}

match replay() {{
    when success _ then {{}}
    when failure _ then {{}}
}}
"#
    )
}

fn escape_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

fn percentile(sorted: &[i64], fraction: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() as f64 - 1.0) * fraction).ceil() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn path_with_suffix(path: &std::path::Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

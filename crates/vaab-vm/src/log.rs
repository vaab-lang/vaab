//! A standard logger for Vaab programs and the HTTP server.
//!
//! Destinations, levels and formats match what other languages call a logger:
//! write to stdout, stderr, a file, or several at once; filter by severity; and
//! choose text, JSON or a more readable layout.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;

/// How loud a logger is. Messages below the configured level are dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug = 10,
    Info = 20,
    Warn = 30,
    Error = 40,
}

impl Level {
    pub fn parse(name: &str) -> Option<Level> {
        match name.trim().to_ascii_lowercase().as_str() {
            "debug" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" => Some(Level::Error),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// How a line is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// One line: `2026-09-22T05:38:00Z INFO message key=value`
    Text,
    /// One JSON object per line.
    Json,
    /// A short level tag and indented fields, meant for terminals.
    Pretty,
}

impl Format {
    pub fn parse(name: &str) -> Option<Format> {
        match name.trim().to_ascii_lowercase().as_str() {
            "text" => Some(Format::Text),
            "json" => Some(Format::Json),
            "pretty" => Some(Format::Pretty),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Format::Text => "text",
            Format::Json => "json",
            Format::Pretty => "pretty",
        }
    }
}

/// Where lines go.
#[derive(Debug)]
enum Sink {
    Stdout,
    Stderr,
    File(Mutex<File>),
    Memory(Mutex<Vec<String>>),
    Multi(Vec<Arc<LoggerInner>>),
}

#[derive(Debug)]
struct LoggerInner {
    sink: Sink,
    level: Mutex<Level>,
    format: Mutex<Format>,
    /// Path kept only so `show` and errors can name a file logger.
    path: Option<PathBuf>,
}

/// A handle to a configured logger. Cloning shares the same destination and settings.
#[derive(Clone, Debug)]
pub struct Logger {
    inner: Arc<LoggerInner>,
}

impl Logger {
    pub fn stdout() -> Logger {
        Logger::new(Sink::Stdout, None)
    }

    pub fn stderr() -> Logger {
        Logger::new(Sink::Stderr, None)
    }

    pub fn memory() -> Logger {
        Logger::new(Sink::Memory(Mutex::new(Vec::new())), None)
    }

    pub fn file(path: impl AsRef<Path>) -> Result<Logger, String> {
        let path = path.as_ref();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("could not open log file {}: {error}", path.display()))?;
        Ok(Logger::new(Sink::File(Mutex::new(file)), Some(path.to_path_buf())))
    }

    /// Writes the same line to every logger in the list.
    pub fn multi(loggers: Vec<Logger>) -> Logger {
        let children = loggers.into_iter().map(|logger| logger.inner).collect();
        Logger::new(Sink::Multi(children), None)
    }

    fn new(sink: Sink, path: Option<PathBuf>) -> Logger {
        Logger {
            inner: Arc::new(LoggerInner {
                sink,
                level: Mutex::new(Level::Info),
                format: Mutex::new(Format::Text),
                path,
            }),
        }
    }

    pub fn set_level(&self, level: Level) {
        if let Sink::Multi(children) = &self.inner.sink {
            for child in children {
                if let Ok(mut held) = child.level.lock() {
                    *held = level;
                }
            }
        }
        if let Ok(mut held) = self.inner.level.lock() {
            *held = level;
        }
    }

    pub fn set_format(&self, format: Format) {
        if let Sink::Multi(children) = &self.inner.sink {
            for child in children {
                if let Ok(mut held) = child.format.lock() {
                    *held = format;
                }
            }
        }
        if let Ok(mut held) = self.inner.format.lock() {
            *held = format;
        }
    }

    pub fn level(&self) -> Level {
        match self.inner.level.lock() {
            Ok(held) => *held,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    pub fn format(&self) -> Format {
        match self.inner.format.lock() {
            Ok(held) => *held,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    pub fn debug(&self, message: &str) {
        self.log(Level::Debug, message, &[]);
    }

    pub fn info(&self, message: &str) {
        self.log(Level::Info, message, &[]);
    }

    pub fn warn(&self, message: &str) {
        self.log(Level::Warn, message, &[]);
    }

    pub fn error(&self, message: &str) {
        self.log(Level::Error, message, &[]);
    }

    pub fn log(&self, level: Level, message: &str, fields: &[(&str, &str)]) {
        if level < self.level() {
            return;
        }
        let line = render(self.format(), level, message, fields);
        let _ = self.inner.write_line(&line);
    }

    pub fn write(&self, level: Level, message: &str, fields: &[(&str, &str)]) {
        self.log(level, message, fields);
    }

    /// Lines captured by a memory logger, newest last. Other destinations give back nothing.
    pub fn lines(&self) -> Vec<String> {
        match &self.inner.sink {
            Sink::Memory(lines) => match lines.lock() {
                Ok(held) => held.clone(),
                Err(poisoned) => poisoned.into_inner().clone(),
            },
            Sink::Multi(children) => {
                let mut gathered = Vec::new();
                for child in children {
                    if let Sink::Memory(lines) = &child.sink {
                        match lines.lock() {
                            Ok(held) => gathered.extend(held.iter().cloned()),
                            Err(poisoned) => gathered.extend(poisoned.into_inner().iter().cloned()),
                        }
                    }
                }
                gathered
            }
            _ => Vec::new(),
        }
    }

    pub fn describe(&self) -> String {
        match &self.inner.sink {
            Sink::Stdout => "logger stdout".to_string(),
            Sink::Stderr => "logger stderr".to_string(),
            Sink::File(_) => match &self.inner.path {
                Some(path) => format!("logger file {}", path.display()),
                None => "logger file".to_string(),
            },
            Sink::Memory(_) => "logger memory".to_string(),
            Sink::Multi(children) => format!("logger multi({})", children.len()),
        }
    }
}

impl LoggerInner {
    fn write_line(&self, line: &str) -> io::Result<()> {
        match &self.sink {
            Sink::Stdout => {
                let mut out = io::stdout().lock();
                writeln!(out, "{line}")?;
                out.flush()
            }
            Sink::Stderr => {
                let mut out = io::stderr().lock();
                writeln!(out, "{line}")?;
                out.flush()
            }
            Sink::File(file) => {
                let mut file = match file.lock() {
                    Ok(held) => held,
                    Err(poisoned) => poisoned.into_inner(),
                };
                writeln!(file, "{line}")?;
                file.flush()
            }
            Sink::Memory(lines) => {
                let mut lines = match lines.lock() {
                    Ok(held) => held,
                    Err(poisoned) => poisoned.into_inner(),
                };
                lines.push(line.to_string());
                Ok(())
            }
            Sink::Multi(children) => {
                for child in children {
                    child.write_line(line)?;
                }
                Ok(())
            }
        }
    }
}

fn render(format: Format, level: Level, message: &str, fields: &[(&str, &str)]) -> String {
    let stamp = timestamp();
    match format {
        Format::Text => {
            let mut line = format!("{stamp} {} {message}", level.label());
            for (key, value) in fields {
                line.push(' ');
                line.push_str(key);
                line.push('=');
                line.push_str(&escape_field(value));
            }
            line
        }
        Format::Json => {
            let mut map = serde_json::Map::new();
            map.insert("time".into(), serde_json::Value::String(stamp));
            map.insert("level".into(), serde_json::Value::String(level.as_str().into()));
            map.insert("message".into(), serde_json::Value::String(message.into()));
            if !fields.is_empty() {
                let mut object = serde_json::Map::new();
                for (key, value) in fields {
                    object.insert((*key).into(), serde_json::Value::String((*value).into()));
                }
                map.insert("fields".into(), serde_json::Value::Object(object));
            }
            serde_json::Value::Object(map).to_string()
        }
        Format::Pretty => {
            let mut line = format!("{}  {message}", level.as_str());
            if !fields.is_empty() {
                let shown: Vec<String> = fields
                    .iter()
                    .map(|(key, value)| format!("{key}={}", escape_field(value)))
                    .collect();
                line.push('\n');
                line.push_str("       ");
                line.push_str(&shown.join("  "));
            }
            line
        }
    }
}

fn escape_field(value: &str) -> String {
    if value.is_empty()
        || value.chars().any(|letter| letter.is_whitespace() || matches!(letter, '"' | '=' | '\\'))
    {
        let mut out = String::with_capacity(value.len() + 2);
        out.push('"');
        for letter in value.chars() {
            match letter {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                other => out.push(other),
            }
        }
        out.push('"');
        out
    } else {
        value.to_string()
    }
}

fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Enough for logs without pulling in a calendar crate: UTC via a fixed epoch.
    let (year, month, day, hour, minute, second) = civil_from_days(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Days since Unix epoch → civil UTC date and time-of-day.
fn civil_from_days(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let second = (secs % 60) as u32;
    let minutes = secs / 60;
    let minute = (minutes % 60) as u32;
    let hours = minutes / 60;
    let hour = (hours % 24) as u32;
    let days = (hours / 24) as i64;

    // Algorithm from Howard Hinnant's date library (public domain).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32, hour, minute, second)
}

/// Process-wide registry of loggers opened from Vaab (`Logger.stdout`, `.file`, …).
#[derive(Default)]
pub struct Loggers {
    loggers: Mutex<Vec<Logger>>,
}

impl Loggers {
    pub fn shared() -> Arc<Loggers> {
        static LOGGERS: OnceLock<Arc<Loggers>> = OnceLock::new();
        Arc::clone(LOGGERS.get_or_init(|| Arc::new(Loggers::default())))
    }

    pub fn stdout(&self) -> u32 {
        self.push(Logger::stdout())
    }

    pub fn stderr(&self) -> u32 {
        self.push(Logger::stderr())
    }

    pub fn memory(&self) -> u32 {
        self.push(Logger::memory())
    }

    pub fn file(&self, path: &str) -> Result<u32, String> {
        Ok(self.push(Logger::file(path)?))
    }

    pub fn multi(&self, handles: &[u32]) -> Result<u32, String> {
        let mut children = Vec::with_capacity(handles.len());
        for handle in handles {
            children.push(self.get(*handle)?);
        }
        Ok(self.push(Logger::multi(children)))
    }

    pub fn get(&self, handle: u32) -> Result<Logger, String> {
        let loggers = self.loggers.lock().map_err(|_| "loggers lock poisoned".to_string())?;
        loggers
            .get(handle as usize)
            .cloned()
            .ok_or_else(|| format!("unknown logger {handle}"))
    }

    pub fn set_level(&self, handle: u32, level: &str) -> Result<(), String> {
        let level = Level::parse(level).ok_or_else(|| {
            format!("unknown log level `{level}` (want debug, info, warn or error)")
        })?;
        self.get(handle)?.set_level(level);
        Ok(())
    }

    pub fn set_format(&self, handle: u32, format: &str) -> Result<(), String> {
        let format = Format::parse(format).ok_or_else(|| {
            format!("unknown log format `{format}` (want text, json or pretty)")
        })?;
        self.get(handle)?.set_format(format);
        Ok(())
    }

    pub fn write(
        &self,
        handle: u32,
        level: Level,
        message: &str,
        fields: &IndexMap<String, String>,
    ) -> Result<(), String> {
        let logger = self.get(handle)?;
        let pairs: Vec<(&str, &str)> = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        logger.log(level, message, &pairs);
        Ok(())
    }

    pub fn lines(&self, handle: u32) -> Result<Vec<String>, String> {
        Ok(self.get(handle)?.lines())
    }

    fn push(&self, logger: Logger) -> u32 {
        let mut loggers = self.loggers.lock().expect("loggers lock");
        let handle = loggers.len() as u32;
        loggers.push(logger);
        handle
    }
}

/// Default logger for the HTTP server process (stderr, text, info).
pub fn server_logger() -> Logger {
    static SERVER: OnceLock<Logger> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            let logger = Logger::stderr();
            logger.set_level(Level::Info);
            logger.set_format(Format::Text);
            if let Ok(level) = std::env::var("VAAB_LOG_LEVEL") {
                if let Some(parsed) = Level::parse(&level) {
                    logger.set_level(parsed);
                }
            }
            if let Ok(format) = std::env::var("VAAB_LOG_FORMAT") {
                if let Some(parsed) = Format::parse(&format) {
                    logger.set_format(parsed);
                }
            }
            logger
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_logger_keeps_lines_at_or_above_its_level() {
        let log = Logger::memory();
        log.set_level(Level::Info);
        log.set_format(Format::Text);
        log.debug("hidden");
        log.info("shown");
        let lines = log.lines();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("INFO"));
        assert!(lines[0].contains("shown"));
    }

    #[test]
    fn json_format_is_one_object() {
        let log = Logger::memory();
        log.set_level(Level::Debug);
        log.set_format(Format::Json);
        log.warn("slow");
        let line = &log.lines()[0];
        let value: serde_json::Value = serde_json::from_str(line).expect("json");
        assert_eq!(value["level"], "warn");
        assert_eq!(value["message"], "slow");
    }

    #[test]
    fn multi_writes_to_every_child() {
        let left = Logger::memory();
        let right = Logger::memory();
        left.set_level(Level::Debug);
        right.set_level(Level::Debug);
        let both = Logger::multi(vec![left.clone(), right.clone()]);
        both.info("ping");
        assert_eq!(left.lines().len(), 1);
        assert_eq!(right.lines().len(), 1);
    }
}

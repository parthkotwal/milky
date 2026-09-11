//! Headless driver for the engine.
//!
//! Exists so the engine can be exercised, tested, and benchmarked without
//! building or launching the macOS app. That separation matters: if the only
//! way to run a search is through the UI, iterating on ranking is slow and
//! measuring latency is impossible.

use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::Instant;

use milky_core::{Config, Engine};

const USAGE: &str = "\
milky — dev driver for the Milky search engine

USAGE:
    milky health
    milky search <query> [--limit N]
    milky raw '<request json>'
    milky repl
    milky bench <query> [--iterations N]
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let engine = match Engine::new(Config::default()) {
        Ok(engine) => engine,
        Err(err) => {
            eprintln!("failed to start engine: {err}");
            return ExitCode::FAILURE;
        }
    };

    match args.first().map(String::as_str) {
        Some("health") => print(&engine.handle_json(r#"{"op":"health"}"#)),
        Some("search") => match search_request(&args[1..]) {
            Ok(request) => print(&engine.handle_json(&request)),
            Err(err) => return fail(&err),
        },
        Some("raw") => match args.get(1) {
            Some(request) => print(&engine.handle_json(request)),
            None => return fail("raw needs a JSON request"),
        },
        Some("repl") => return repl(&engine),
        Some("bench") => match bench(&engine, &args[1..]) {
            Ok(()) => {}
            Err(err) => return fail(&err),
        },
        Some("--help") | Some("-h") | None => print!("{USAGE}"),
        Some(other) => return fail(&format!("unknown command '{other}'\n\n{USAGE}")),
    }

    ExitCode::SUCCESS
}

fn print(response: &str) {
    println!("{response}");
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}

/// Build a search request from `<query> [--limit N]`.
fn search_request(args: &[String]) -> Result<String, String> {
    let mut query: Option<&str> = None;
    let mut limit: Option<usize> = None;
    let mut rest = args.iter();

    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--limit" => {
                let value = rest.next().ok_or("--limit needs a number")?;
                limit = Some(value.parse().map_err(|_| format!("bad limit '{value}'"))?);
            }
            other if query.is_none() => query = Some(other),
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }

    let query = query.ok_or("search needs a query")?;
    Ok(match limit {
        Some(limit) => format!(
            r#"{{"op":"search","query":{},"limit":{limit}}}"#,
            quote(query)
        ),
        None => format!(r#"{{"op":"search","query":{}}}"#, quote(query)),
    })
}

/// Read requests from stdin, one JSON object per line, and echo responses.
///
/// This is the same line-delimited shape a socket transport would use, so it
/// doubles as a rehearsal for moving the engine out of process later.
fn repl(engine: &Engine) -> ExitCode {
    let mut input = String::new();
    if let Err(err) = io::stdin().read_to_string(&mut input) {
        return fail(&format!("failed to read stdin: {err}"));
    }
    let mut out = io::stdout().lock();
    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        let _ = writeln!(out, "{}", engine.handle_json(line));
    }
    ExitCode::SUCCESS
}

/// Time repeated queries against one warm engine.
///
/// Reports p50 and p99 rather than a mean: for a launcher, the tail is what the
/// user actually notices.
fn bench(engine: &Engine, args: &[String]) -> Result<(), String> {
    let mut iterations = 1_000usize;
    let mut query: Option<&str> = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--iterations" => {
                let value = rest.next().ok_or("--iterations needs a number")?;
                iterations = value.parse().map_err(|_| format!("bad count '{value}'"))?;
            }
            // Reject extra positionals rather than silently keeping the last
            // one: an unquoted multi-word query used to benchmark only its
            // final word, which is a quietly wrong measurement.
            other if query.is_none() => query = Some(other),
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    let query = query.unwrap_or("terminal");
    if iterations == 0 {
        return Err("--iterations must be greater than 0".into());
    }

    let request = format!(r#"{{"op":"search","query":{}}}"#, quote(query));
    // Warm caches and code paths so the first sample is not an outlier.
    for _ in 0..iterations.min(50) {
        let _ = engine.handle_json(&request);
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let response = engine.handle_json(&request);
        samples.push(started.elapsed().as_nanos() as u64);
        debug_assert!(!response.is_empty());
    }
    samples.sort_unstable();

    let micros = |nanos: u64| nanos as f64 / 1000.0;
    println!("query        {query:?}");
    println!("iterations   {iterations}");
    println!("p50          {:.1} us", micros(samples[samples.len() / 2]));
    println!(
        "p99          {:.1} us",
        micros(samples[samples.len() * 99 / 100])
    );
    println!("max          {:.1} us", micros(*samples.last().unwrap()));
    Ok(())
}

/// Minimal JSON string escaping, enough for shell-supplied queries.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

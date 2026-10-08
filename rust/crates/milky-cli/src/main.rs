//! Headless driver for the Milky engine.
//!
//! Exists so ranking can be judged by typing real queries at the real index,
//! without a UI. If the only way to run a search were the launcher, iterating on
//! ranking would be slow and measuring latency would be awkward.
//!
//! No argument-parsing dependency on purpose: the surface is small enough to
//! hand-roll, and the dependency tree stays empty.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::time::Instant;

use milky_core::engine::Engine;
use milky_core::search::Hit;

const USAGE: &str = "\
milky — dev driver for the Milky search engine

USAGE:
    milky <query>...          search, print ranked results
    milky                     interactive: one query per line, one warm engine
    milky --count             how many apps and settings are indexed
    milky --settings          list System Settings panes and their sections

OPTIONS:
    --limit N                 maximum results (default 10)
    -h, --help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let options = match Options::parse(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    if options.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    if options.settings {
        let started = Instant::now();
        let panes = milky_core::settings::discover_settings();
        let items: usize = panes.iter().map(|pane| pane.items.len()).sum();
        eprintln!("read {} panes, {items} items in {:.1?}", panes.len(), started.elapsed());
        for pane in &panes {
            let hidden = if pane.in_sidebar { "" } else { "  (not in sidebar)" };
            println!("{:<34} {:>3} items  {}{hidden}", pane.name, pane.items.len(), pane.id);
        }
        return ExitCode::SUCCESS;
    }

    // Built once and reused for every query below, which is the whole point of
    // the engine holding its index: the scan cost is paid at startup, not per
    // keystroke.
    let started = Instant::now();
    let engine = Engine::new();
    let scan = started.elapsed();
    eprintln!(
        "indexed {} apps and {} settings in {:.1?}",
        engine.app_count(),
        engine.settings_count(),
        scan
    );

    if options.count {
        return ExitCode::SUCCESS;
    }

    match options.query {
        Some(query) => {
            run_query(&engine, &query, options.limit);
            ExitCode::SUCCESS
        }
        None => interactive(&engine, options.limit),
    }
}

struct Options {
    query: Option<String>,
    limit: usize,
    count: bool,
    settings: bool,
    help: bool,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut limit = 10usize;
        let mut count = false;
        let mut help = false;
        let mut settings = false;
        let mut words: Vec<&str> = Vec::new();

        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            match arg.as_str() {
                "--limit" => {
                    let value = rest.next().ok_or("--limit needs a number")?;
                    limit = value
                        .parse()
                        .map_err(|_| format!("bad --limit value '{value}'"))?;
                    if limit == 0 {
                        return Err("--limit must be greater than 0".to_string());
                    }
                }
                "--count" => count = true,
                "--settings" => settings = true,
                "-h" | "--help" => help = true,
                other if other.starts_with('-') => {
                    return Err(format!("unknown option '{other}'"));
                }
                // Remaining words are the query, so an unquoted multi-word query
                // still works: `milky visual studio`.
                other => words.push(other),
            }
        }

        let query = if words.is_empty() {
            None
        } else {
            Some(words.join(" "))
        };

        Ok(Options {
            query,
            limit,
            count,
            settings,
            help,
        })
    }
}

fn run_query(engine: &Engine, query: &str, limit: usize) {
    let started = Instant::now();
    let results = engine.search(query, limit);
    let elapsed = started.elapsed();

    print_results(&results, elapsed);
}

fn print_results(results: &[Hit<'_>], elapsed: std::time::Duration) {
    if results.is_empty() {
        println!("  no matches  ({elapsed:.1?})");
        return;
    }

    // Pad titles to a common width so the columns line up and the ranking is
    // readable at a glance.
    let widest = results
        .iter()
        .map(|hit| hit.title.chars().count())
        .max()
        .unwrap_or(0);

    for (index, hit) in results.iter().enumerate() {
        println!(
            "  {rank:>2}. {title:<width$}  {kind:<11}  {subtitle}",
            rank = index + 1,
            title = hit.title,
            width = widest,
            kind = format!("{:?}", hit.kind),
            subtitle = hit.candidate.subtitle,
        );
    }
    println!("  {} results in {elapsed:.1?}", results.len());
}

/// Read one query per line against a single warm engine.
///
/// This is the useful mode for judging ranking: type, look, adjust, repeat,
/// without paying the scan again.
fn interactive(engine: &Engine, limit: usize) -> ExitCode {
    let stdin = io::stdin();
    let mut out = io::stdout();

    loop {
        print!("> ");
        if out.flush().is_err() {
            return ExitCode::FAILURE;
        }

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            // End of input: ^D, or a piped file running out.
            Ok(0) => {
                println!();
                return ExitCode::SUCCESS;
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!("failed to read input: {err}");
                return ExitCode::FAILURE;
            }
        }

        let query = line.trim();
        if query.is_empty() {
            continue;
        }
        if query == ":quit" || query == ":q" {
            return ExitCode::SUCCESS;
        }

        run_query(engine, query, limit);
    }
}

//! Headless driver for the Milky engine.
//!
//! Exists so ranking can be judged by typing real queries at the real index,
//! without a UI. If the only way to run a search were the launcher, iterating on
//! ranking would be slow and measuring latency would be awkward.
//!
//! No argument-parsing dependency on purpose: the surface is small enough to
//! hand-roll, and the dependency tree stays empty.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use milky_core::engine::Engine;
use milky_core::files::{self, Exclusions};
use milky_core::search::Hit;

const USAGE: &str = "\
milky — dev driver for the Milky search engine

USAGE:
    milky <query>...          search, print ranked results
    milky                     interactive: one query per line, one warm engine
    milky --count             how many apps and settings are indexed
    milky --settings          list System Settings panes and their sections
    milky --files             walk the home folder and iCloud Drive: counts,
                              timing, unreadable folders, largest folders
    milky --watch             keep the engine running and print each index
                              update as files change (Ctrl-C to stop)
    milky --log               what the event store holds, and recent sessions
    milky --extract           extract and word-split every indexable document
                              (fills the text cache); report counts and timing
    milky --pause-history     stop recording use (searches still work)
    milky --resume-history    start recording again
    milky --export PATH       write a consistent copy of the event store
    milky --forget-history all|DAYS
                              delete recorded history: all of it (usage
                              scores too), or the last DAYS days

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

    if options.files {
        return list_files();
    }

    if options.extract {
        return extract_all();
    }

    if options.settings {
        let started = Instant::now();
        let panes = milky_core::settings::discover_settings();
        let items: usize = panes.iter().map(|pane| pane.items.len()).sum();
        eprintln!(
            "read {} panes, {items} items in {:.1?}",
            panes.len(),
            started.elapsed()
        );
        for pane in &panes {
            let hidden = if pane.in_sidebar {
                ""
            } else {
                "  (not in sidebar)"
            };
            println!(
                "{:<34} {:>3} items  {}{hidden}",
                pane.name,
                pane.items.len(),
                pane.id
            );
        }
        return ExitCode::SUCCESS;
    }

    // Built once and reused for every query below, which is the whole point of
    // the engine holding its index: the scan cost is paid at startup, not per
    // keystroke.
    let started = Instant::now();
    let engine = Engine::new();
    let ready = started.elapsed();
    // Files arrive from a background thread; a headless one-shot query wants
    // them, so wait (the launcher does not).
    let files_ready = engine.wait_for_files(Duration::from_secs(60));
    let with_files = started.elapsed();
    eprintln!(
        "indexed {} apps, {} places, {} settings in {:.1?}; {} files and folders {} {:.1?}",
        engine.app_count(),
        engine.place_count(),
        engine.settings_count(),
        ready,
        engine.file_count(),
        if files_ready { "after" } else { "still walking after" },
        with_files
    );

    if options.count {
        return ExitCode::SUCCESS;
    }

    if options.watch {
        watch(&engine);
    }

    if options.log {
        return show_log(&engine);
    }

    if let Some(history) = options.history {
        return change_history(&engine, history);
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
    files: bool,
    extract: bool,
    watch: bool,
    log: bool,
    history: Option<History>,
    help: bool,
}

/// A change to the recorded history, from the command line.
enum History {
    Pause,
    Resume,
    Export(std::path::PathBuf),
    Forget(Option<u64>),
}

impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut limit = 10usize;
        let mut count = false;
        let mut help = false;
        let mut settings = false;
        let mut files = false;
        let mut extract = false;
        let mut watch = false;
        let mut log = false;
        let mut history = None;
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
                "--files" => files = true,
                "--extract" => extract = true,
                "--watch" => watch = true,
                "--log" => log = true,
                "--pause-history" => history = Some(History::Pause),
                "--resume-history" => history = Some(History::Resume),
                "--export" => {
                    let path = rest.next().ok_or("--export needs a path")?;
                    history = Some(History::Export(path.into()));
                }
                "--forget-history" => {
                    let what = rest.next().ok_or("--forget-history needs `all` or a number of days")?;
                    history = Some(History::Forget(match what.as_str() {
                        "all" => None,
                        days => Some(days.parse().map_err(|_| format!("bad --forget-history value '{days}'"))?),
                    }));
                }
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
            files,
            extract,
            watch,
            log,
            history,
            help,
        })
    }
}

fn run_query(engine: &Engine, query: &str, limit: usize) {
    let started = Instant::now();
    let index = engine.index();
    let results = engine.search(&index, query, limit);
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
            "  {rank:>2}. {title:<width$}  {kind:<15}  {subtitle}",
            rank = index + 1,
            title = hit.title,
            width = widest,
            kind = format!("{:?}", hit.tier),
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

/// Walk the file roots the way the engine will, and show what was found and
/// what it cost. Creates `exclusions.txt` with the defaults if it is missing.
fn list_files() -> ExitCode {
    let Some(home) = std::env::home_dir() else {
        eprintln!("no home directory");
        return ExitCode::FAILURE;
    };
    let exclusions = load_exclusions(&home);

    let started = Instant::now();
    let walk = files::discover_files(&home, &exclusions);
    let elapsed = started.elapsed();

    let folders = walk.entries.iter().filter(|entry| entry.is_folder).count();
    println!(
        "{} entries ({folders} folders, {} files) in {elapsed:.1?}",
        walk.entries.len(),
        walk.entries.len() - folders,
    );
    for root in files::roots(&home) {
        println!("  root: {}", tilde(&root, &home));
    }
    for dir in &walk.unreadable {
        println!("  unreadable: {}", tilde(dir, &home));
    }

    // Two levels below the home folder is where one project or one tool's
    // output shows up as a single line, which is what deciding on an
    // exclusion needs.
    let mut sizes: HashMap<PathBuf, usize> = HashMap::new();
    for entry in &walk.entries {
        let relative = entry.path.strip_prefix(&home).unwrap_or(&entry.path);
        *sizes
            .entry(relative.components().take(2).collect())
            .or_default() += 1;
    }
    let mut largest: Vec<(PathBuf, usize)> = sizes.into_iter().collect();
    largest.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    println!("largest folders:");
    for (folder, count) in largest.iter().take(10) {
        println!("  {count:>7}  ~/{}", folder.display());
    }
    ExitCode::SUCCESS
}

/// The user's exclusions, reported, falling back to the defaults on any error
/// so a broken file never means indexing everything.
fn load_exclusions(home: &Path) -> Exclusions {
    let path = match files::exclusions_path() {
        Ok(path) => path,
        Err(err) => {
            println!("exclusions: {err}; using the defaults");
            return Exclusions::defaults(home);
        }
    };
    let existed = path.exists();
    match files::load_exclusions(&path, home) {
        Ok((rules, ignored)) => {
            let created = if existed {
                ""
            } else {
                ", just created with the defaults"
            };
            println!(
                "exclusions: {} ({} rules{created})",
                tilde(&path, home),
                rules.len()
            );
            for line in &ignored {
                println!("  ignored line, not a name or a path: {line}");
            }
            rules
        }
        Err(err) => {
            println!(
                "exclusions: {}: {err}; using the defaults",
                tilde(&path, home)
            );
            Exclusions::defaults(home)
        }
    }
}

/// `path` with the home folder written as `~`.
fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(relative) => format!("~/{}", relative.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Print every index update as it is published: how many files and folders,
/// and which appeared or disappeared. Runs until interrupted.
fn watch(engine: &Engine) -> ! {
    eprintln!("watching for changes; Ctrl-C to stop");
    let mut last = engine.index();
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let current = engine.index();
        if std::sync::Arc::ptr_eq(&current, &last) {
            continue;
        }
        let ids = |index: &milky_core::engine::Index| -> std::collections::BTreeSet<String> {
            index.candidates().iter().map(|candidate| candidate.id.clone()).collect()
        };
        let (before, after) = (ids(&last), ids(&current));
        let added: Vec<&String> = after.difference(&before).collect();
        let removed: Vec<&String> = before.difference(&after).collect();
        println!(
            "{} files and folders  (+{} -{})",
            current.count(milky_core::candidate::Kind::File) + current.count(milky_core::candidate::Kind::Folder),
            added.len(),
            removed.len()
        );
        for id in added.iter().take(5) {
            println!("  + {id}");
        }
        for id in removed.iter().take(5) {
            println!("  - {id}");
        }
        last = current;
    }
}

/// Print what the event store holds and the most recent sessions.
fn show_log(engine: &Engine) -> ExitCode {
    let Some((counts, sessions)) = engine.log_summary(10) else {
        eprintln!("the event store could not be opened");
        return ExitCode::FAILURE;
    };
    println!(
        "sessions {}  events {}  searches {}  impressions {}  selections {}  focus intervals {}  folder activity {}",
        counts.sessions,
        counts.events,
        counts.searches,
        counts.impressions,
        counts.selections,
        counts.focus_intervals,
        counts.folder_activity
    );
    for session in sessions {
        println!(
            "  {}  {:<10} {:>3} events  last query {:?}  acted on {}",
            session.started_at_ms,
            session.end_reason,
            session.events,
            session.last_query.unwrap_or_default(),
            session.acted_on.unwrap_or_else(|| "-".to_string()),
        );
    }
    ExitCode::SUCCESS
}

fn change_history(engine: &Engine, history: History) -> ExitCode {
    let result = match &history {
        History::Pause => engine.set_paused(true).map(|()| "history paused".to_string()),
        History::Resume => engine.set_paused(false).map(|()| "history resumed".to_string()),
        History::Export(path) => engine.export_history(path).map(|()| format!("exported to {}", path.display())),
        History::Forget(None) => engine.forget_history(None).map(|()| "all history forgotten".to_string()),
        History::Forget(Some(days)) => {
            let since = std::time::SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
            engine.forget_history(Some(since)).map(|()| format!("the last {days} days forgotten"))
        }
    };
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// Extract and word-split every indexable document under the file roots, the
/// way document search will at startup, and report what it cost.
fn extract_all() -> ExitCode {
    use milky_core::extract::{TextKind, text_kind};
    use milky_core::textcache::{self, TextCache};

    let Some(home) = std::env::home_dir() else {
        eprintln!("no home directory");
        return ExitCode::FAILURE;
    };
    let cache_path = milky_core::engine::data_dir().join(textcache::FILE_NAME);
    let mut cache = match TextCache::open(&cache_path) {
        Ok(cache) => cache,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let walk = files::discover_files(&home, &milky_core::engine::user_exclusions(&home));
    let started = Instant::now();
    let (mut documents, mut empty, mut chars, mut occurrences) = (0usize, 0usize, 0usize, 0usize);
    let mut by_kind: HashMap<&str, (usize, Duration)> = HashMap::new();
    let mut tokenizing = Duration::ZERO;
    for entry in walk.entries.iter().filter(|entry| !entry.is_folder) {
        let Some(kind) = text_kind(&entry.path) else { continue };
        let one = Instant::now();
        let text = textcache::document_text(&entry.path, &mut cache);
        let slot = by_kind.entry(match kind {
            TextKind::Plain => "plain",
            TextKind::Pdf => "pdf",
            TextKind::Rich => "rich",
        }).or_default();
        slot.0 += 1;
        slot.1 += one.elapsed();
        let Some(text) = text else {
            empty += 1;
            continue;
        };
        documents += 1;
        chars += text.chars().count();
        let split = Instant::now();
        occurrences += milky_core::terms::occurrences(&text).len();
        tokenizing += split.elapsed();
    }
    println!(
        "{documents} documents with text ({empty} without), {:.1} M characters, {:.2} M word occurrences, in {:.1?}",
        chars as f64 / 1e6,
        occurrences as f64 / 1e6,
        started.elapsed()
    );
    for (kind, (count, time)) in &by_kind {
        println!("  {kind:<6} {count:>5} files  {time:.1?}");
    }
    println!("  splitting into words: {tokenizing:.1?}");
    if let Ok((entries, bytes)) = cache.size() {
        println!("text cache: {entries} entries, {:.1} MB of text  ({})", bytes as f64 / 1e6, cache_path.display());
    }
    ExitCode::SUCCESS
}

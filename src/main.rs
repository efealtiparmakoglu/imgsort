//! imgsort — organize photos into folders by capture date, with dry-run,
//! conflict policies and a summary report.

mod datetime;
mod exif;
mod plan;

use datetime::DateTime;
use exif::read_exif;
use plan::{photo_extension, plan_file, Action, ConflictPolicy, PlanItem, Source, Summary};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::UNIX_EPOCH;

const VERSION: &str = "1.0.0";
const DEFAULT_TEMPLATE: &str = "{Y}/{Y}-{M}-{D}/{name}.{ext}";

const USAGE_TEMPLATE: &str = "\
imgsort {version} — sort photos into folders by capture date

usage:
  imgsort <photos-dir> --dest <target> [options]
  imgsort --help

options:
  --dest DIR          where sorted files go (required)
  --template STR      path template, default \"{Y}/{Y}-{M}-{D}/{name}.{ext}\"
                      placeholders: {Y} {M} {D} {H} {m} {S} {name} {ext} {n}
  --dry-run           print the plan, change nothing
  --on-conflict MODE  rename (default) | skip | overwrite | fail
  --no-recursive      do not descend into subdirectories
  --undated DIR       where files without a date go (default: undated)
  --json              machine-readable report
  -h, --help          this text

exit codes: 0 ok, 1 runtime error, 2 usage error
";

struct Options {
    source: PathBuf,
    dest: PathBuf,
    template: String,
    dry_run: bool,
    policy: ConflictPolicy,
    recursive: bool,
    undated: String,
    json: bool,
}

fn parse_args(args: &[String]) -> Result<Option<Options>, String> {
    let mut source: Option<PathBuf> = None;
    let mut dest: Option<PathBuf> = None;
    let mut template = DEFAULT_TEMPLATE.to_string();
    let mut dry_run = false;
    let mut policy = ConflictPolicy::Rename;
    let mut recursive = true;
    let mut undated = "undated".to_string();
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(None),
            "--dry-run" => dry_run = true,
            "--no-recursive" => recursive = false,
            "--json" => json = true,
            "--dest" => {
                i += 1;
                dest = Some(PathBuf::from(args.get(i).ok_or("--dest needs a directory")?));
            }
            "--template" => {
                i += 1;
                template = args.get(i).ok_or("--template needs a pattern")?.clone();
            }
            "--on-conflict" => {
                i += 1;
                let name = args.get(i).ok_or("--on-conflict needs a mode")?;
                policy = ConflictPolicy::parse(name)
                    .ok_or(format!("unknown conflict mode '{name}'"))?;
            }
            "--undated" => {
                i += 1;
                undated = args.get(i).ok_or("--undated needs a directory")?.clone();
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option '{other}'"));
            }
            other => {
                if source.is_some() {
                    return Err(format!("unexpected argument '{other}'"));
                }
                source = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }

    let source = source.ok_or("a photos directory is required")?;
    let dest = dest.ok_or("--dest is required")?;
    Ok(Some(Options {
        source,
        dest,
        template,
        dry_run,
        policy,
        recursive,
        undated,
        json,
    }))
}

fn collect_photos(dir: &Path, recursive: bool) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                found.extend(collect_photos(&path, recursive)?);
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if photo_extension(&name).is_some() {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

fn file_time(path: &Path) -> Option<DateTime> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let seconds = modified.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    Some(DateTime::from_unix(seconds))
}

/// Work out the capture date: DateTimeOriginal first, then DateTime, then
/// the file's modification time.
fn capture_date(path: &Path) -> (Option<DateTime>, Source) {
    if let Ok(bytes) = fs::read(path) {
        if let Ok(exif) = read_exif(&bytes) {
            if let Some(when) = exif.date_original.or(exif.date_time) {
                return (Some(when), Source::Exif);
            }
        }
    }
    match file_time(path) {
        Some(when) => (Some(when), Source::FileTime),
        None => (None, Source::Unknown),
    }
}

fn describe(item: &PlanItem) -> String {
    let verb = match item.action {
        Action::Move => {
            if item.origin == Source::FileTime {
                "move (file time)"
            } else {
                "move"
            }
        }
        Action::SkipAlreadySorted => "keep",
        Action::ConflictSkip => "skip (exists)",
        Action::ConflictFail => "fail (exists)",
        Action::NoDate => "skip (no date)",
    };
    format!(
        "{:<18} {} -> {}",
        verb,
        item.source.display(),
        item.destination.display()
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{}", USAGE_TEMPLATE.replace("{version}", VERSION));
            return ExitCode::from(2);
        }
        Err(message) => {
            eprintln!("imgsort: {message}");
            eprint!("{}", USAGE_TEMPLATE.replace("{version}", VERSION));
            return ExitCode::from(2);
        }
    };

    match run(&options) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("imgsort: {message}");
            ExitCode::from(1)
        }
    }
}

fn run(options: &Options) -> Result<ExitCode, String> {
    let photos = collect_photos(&options.source, options.recursive)?;
    if !options.dest.exists() && !options.dry_run {
        fs::create_dir_all(&options.dest).map_err(|e| format!("create dest: {e}"))?;
    }

    let mut taken: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let mut summary = Summary::default();
    let mut lines: Vec<String> = Vec::new();

    for path in &photos {
        let (when, origin) = capture_date(path);
        let item = plan_file(
            path,
            &options.dest,
            &options.template,
            when,
            origin,
            options.policy,
            &mut taken,
        );
        if item.action == Action::NoDate {
            // park undated files in their own folder
            let mut parked = plan_file(
                path,
                &options.dest,
                &options.undated,
                Some(DateTime {
                    year: 1970,
                    month: 1,
                    day: 1,
                    hour: 0,
                    minute: 0,
                    second: 0,
                }),
                Source::Unknown,
                options.policy,
                &mut taken,
            );
            parked.action = Action::NoDate;
            lines.push(describe(&parked));
            summary.record(&parked);
            continue;
        }
        lines.push(describe(&item));
        summary.record(&item);

        if !options.dry_run && item.action == Action::Move {
            if let Some(parent) = item.destination.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
            }
            fs::rename(&item.source, &item.destination).map_err(|e| {
                format!("move {} -> {}: {e}", item.source.display(), item.destination.display())
            })?;
        }
    }

    if options.json {
        println!("{{");
        println!("  \"dry_run\": {},", options.dry_run);
        println!("  \"total\": {},", summary.total);
        println!("  \"moved\": {},", summary.moved);
        println!("  \"already_sorted\": {},", summary.already_sorted);
        println!("  \"skipped_conflict\": {},", summary.skipped_conflict);
        println!("  \"failed_conflict\": {},", summary.failed_conflict);
        println!("  \"without_date\": {},", summary.without_date);
        print!("  \"by_year\": {{");
        let years: Vec<String> = summary
            .by_year
            .iter()
            .map(|(year, count)| format!("\"{year}\": {count}"))
            .collect();
        println!("{}", years.join(", "));
        println!("  }}");
        println!("}}");
    } else {
        for line in &lines {
            println!("{line}");
        }
        println!();
        let mode = if options.dry_run { "dry run" } else { "done" };
        println!(
            "{mode}: {} file(s) — {} moved, {} already sorted, {} conflict(s) skipped, \
             {} undated",
            summary.total,
            summary.moved,
            summary.already_sorted,
            summary.skipped_conflict + summary.failed_conflict,
            summary.without_date
        );
        if !summary.by_year.is_empty() {
            let years: Vec<String> = summary
                .by_year
                .iter()
                .map(|(year, count)| format!("{year}: {count}"))
                .collect();
            println!("by year: {}", years.join("  "));
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_basic_options() {
        let options = parse_args(&args(&["/src", "--dest", "/dst", "--dry-run"]))
            .unwrap()
            .unwrap();
        assert_eq!(options.source, PathBuf::from("/src"));
        assert_eq!(options.dest, PathBuf::from("/dst"));
        assert!(options.dry_run);
        assert!(options.recursive);
        assert_eq!(options.policy, ConflictPolicy::Rename);
    }

    #[test]
    fn parses_every_flag() {
        let options = parse_args(&args(&[
            "/src", "--dest", "/dst", "--template", "{Y}/{name}.{ext}", "--on-conflict",
            "overwrite", "--no-recursive", "--undated", "nodate", "--json",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(options.template, "{Y}/{name}.{ext}");
        assert_eq!(options.policy, ConflictPolicy::Overwrite);
        assert!(!options.recursive);
        assert_eq!(options.undated, "nodate");
        assert!(options.json);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_args(&args(&["/src"])).is_err(), "missing --dest");
        assert!(parse_args(&args(&["--dest", "/dst"])).is_err(), "missing source");
        assert!(parse_args(&args(&["/src", "--dest"])).is_err(), "dangling --dest");
        assert!(parse_args(&args(&["/src", "--dest", "/d", "--on-conflict", "explode"])).is_err());
        assert!(parse_args(&args(&["/a", "/b", "--dest", "/d"])).is_err(), "two sources");
        assert!(parse_args(&args(&["/src", "--dest", "/d", "--wat"])).is_err());
    }

    #[test]
    fn help_returns_none() {
        assert!(parse_args(&args(&["--help"])).unwrap().is_none());
    }

    #[test]
    fn action_descriptions() {
        let base = PlanItem {
            source: PathBuf::from("/a.jpg"),
            destination: PathBuf::from("/b.jpg"),
            when: None,
            origin: Source::Exif,
            action: Action::Move,
        };
        assert!(describe(&base).contains("move"));
        let mut skip = base.clone();
        skip.action = Action::NoDate;
        assert!(describe(&skip).contains("no date"));
    }
}

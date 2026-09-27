//! Move planning: turn a photo's capture date into a destination path,
//! resolve conflicts, and summarize what a run would do.

use crate::datetime::DateTime;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// keep both files, add -1, -2, ... to the name
    Rename,
    /// leave the existing file alone and skip this one
    Skip,
    /// replace the existing file
    Overwrite,
    /// report the conflict and do nothing
    Fail,
}

impl ConflictPolicy {
    pub fn parse(name: &str) -> Option<ConflictPolicy> {
        match name {
            "rename" => Some(ConflictPolicy::Rename),
            "skip" => Some(ConflictPolicy::Skip),
            "overwrite" => Some(ConflictPolicy::Overwrite),
            "fail" => Some(ConflictPolicy::Fail),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Exif,
    FileTime,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub when: Option<DateTime>,
    pub origin: Source,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Move,
    SkipAlreadySorted,
    ConflictSkip,
    ConflictFail,
    NoDate,
}

#[derive(Debug, Default, Clone)]
pub struct Summary {
    pub total: usize,
    pub moved: usize,
    pub already_sorted: usize,
    pub skipped_conflict: usize,
    pub failed_conflict: usize,
    pub without_date: usize,
    pub by_year: BTreeMap<i32, usize>,
}

impl Summary {
    pub fn record(&mut self, item: &PlanItem) {
        self.total += 1;
        match item.action {
            Action::Move => self.moved += 1,
            Action::SkipAlreadySorted => self.already_sorted += 1,
            Action::ConflictSkip => self.skipped_conflict += 1,
            Action::ConflictFail => self.failed_conflict += 1,
            Action::NoDate => self.without_date += 1,
        }
        if let Some(when) = item.when {
            *self.by_year.entry(when.year).or_insert(0) += 1;
        }
    }

}

/// Render a path template. Supported placeholders:
/// `{Y}` `{M}` `{D}` `{H}` `{m}` `{S}` `{name}` `{ext}` `{n}` (counter).
pub fn render_template(template: &str, when: &DateTime, stem: &str, ext: &str, counter: usize) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '{' {
            out.push(ch);
            continue;
        }
        let mut key = String::new();
        for next in chars.by_ref() {
            if next == '}' {
                break;
            }
            key.push(next);
        }
        match key.as_str() {
            "Y" => out.push_str(&format!("{:04}", when.year)),
            "M" => out.push_str(&format!("{:02}", when.month)),
            "D" => out.push_str(&format!("{:02}", when.day)),
            "H" => out.push_str(&format!("{:02}", when.hour)),
            "m" => out.push_str(&format!("{:02}", when.minute)),
            "S" => out.push_str(&format!("{:02}", when.second)),
            "name" => out.push_str(stem),
            "ext" => out.push_str(ext),
            "n" => out.push_str(&counter.to_string()),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
    }
    out
}

/// Build the destination path for one file, relative to `root`.
pub fn destination_for(
    root: &Path,
    template: &str,
    when: &DateTime,
    stem: &str,
    ext: &str,
) -> PathBuf {
    let rendered = render_template(template, when, stem, ext, 0);
    let mut path = root.to_path_buf();
    for part in rendered.split('/') {
        if !part.is_empty() {
            path.push(part);
        }
    }
    path
}

fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name.to_string(), String::new()),
    }
}

/// Plan a single file's move given which destinations are already taken.
pub fn plan_file(
    source: &Path,
    root: &Path,
    template: &str,
    when: Option<DateTime>,
    origin: Source,
    policy: ConflictPolicy,
    taken: &mut BTreeMap<PathBuf, usize>,
) -> PlanItem {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = split_name(&name);

    let when = match when {
        Some(when) => when,
        None => {
            return PlanItem {
                source: source.to_path_buf(),
                destination: source.to_path_buf(),
                when: None,
                origin,
                action: Action::NoDate,
            }
        }
    };

    let base = destination_for(root, template, &when, &stem, &ext);
    if base == source {
        return PlanItem {
            source: source.to_path_buf(),
            destination: base,
            when: Some(when),
            origin,
            action: Action::SkipAlreadySorted,
        };
    }

    let exists = base.exists();
    let destination = match (exists, policy) {
        (false, _) => base,
        (true, ConflictPolicy::Overwrite) => base,
        (true, ConflictPolicy::Skip) => {
            return PlanItem {
                source: source.to_path_buf(),
                destination: base,
                when: Some(when),
                origin,
                action: Action::ConflictSkip,
            }
        }
        (true, ConflictPolicy::Fail) => {
            return PlanItem {
                source: source.to_path_buf(),
                destination: base,
                when: Some(when),
                origin,
                action: Action::ConflictFail,
            }
        }
        (true, ConflictPolicy::Rename) => {
            let counter = {
                let slot = taken.entry(base.clone()).or_insert(0);
                *slot += 1;
                *slot
            };
            numbered_name(&base, counter)
        }
    };
    if !taken.contains_key(&destination) {
        taken.insert(destination.clone(), 0);
    }

    PlanItem {
        source: source.to_path_buf(),
        destination,
        when: Some(when),
        origin,
        action: Action::Move,
    }
}

fn numbered_name(path: &Path, counter: usize) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (stem, ext) = split_name(&name);
    let new_name = if ext.is_empty() {
        format!("{stem}-{counter}")
    } else {
        format!("{stem}-{counter}.{ext}")
    };
    path.with_file_name(new_name)
}

/// File extension of a photo, lowercased and without the dot.
pub fn photo_extension(name: &str) -> Option<String> {
    let (_, ext) = split_name(name);
    let lower = ext.to_ascii_lowercase();
    const KNOWN: [&str; 8] = ["jpg", "jpeg", "png", "heic", "tif", "tiff", "dng", "webp"];
    if KNOWN.contains(&lower.as_str()) {
        Some(lower)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date() -> DateTime {
        DateTime {
            year: 2026,
            month: 9,
            day: 27,
            hour: 14,
            minute: 5,
            second: 9,
        }
    }

    #[test]
    fn template_rendering() {
        let out = render_template(
            "{Y}/{Y}-{M}-{D}/{Y}{M}{D}_{H}{m}{S}_{name}.{ext}",
            &date(),
            "DSC1234",
            "jpg",
            0,
        );
        assert_eq!(out, "2026/2026-09-27/20260927_140509_DSC1234.jpg");
    }

    #[test]
    fn template_unknown_placeholder_is_literal() {
        let out = render_template("{Y}-{nope}", &date(), "a", "jpg", 3);
        assert_eq!(out, "2026-{nope}");
    }

    #[test]
    fn destination_for_builds_nested_paths() {
        let path = destination_for(
            Path::new("/photos"),
            "{Y}/{M}/{name}.{ext}",
            &date(),
            "img",
            "jpeg",
        );
        assert_eq!(path, Path::new("/photos/2026/09/img.jpeg"));
    }

    #[test]
    fn extensions_are_recognized() {
        assert_eq!(photo_extension("a.JPG"), Some("jpg".to_string()));
        assert_eq!(photo_extension("noext"), None);
        assert_eq!(photo_extension("notes.txt"), None);
    }

    #[test]
    fn name_splitting() {
        assert_eq!(split_name("photo.jpg"), ("photo".to_string(), "jpg".to_string()));
        assert_eq!(split_name("archive.tar.gz"), ("archive.tar".to_string(), "gz".to_string()));
        assert_eq!(split_name("noext"), ("noext".to_string(), String::new()));
    }

    #[test]
    fn policy_parsing() {
        assert_eq!(ConflictPolicy::parse("rename"), Some(ConflictPolicy::Rename));
        assert_eq!(ConflictPolicy::parse("skip"), Some(ConflictPolicy::Skip));
        assert_eq!(ConflictPolicy::parse("overwrite"), Some(ConflictPolicy::Overwrite));
        assert_eq!(ConflictPolicy::parse("fail"), Some(ConflictPolicy::Fail));
        assert_eq!(ConflictPolicy::parse("nonsense"), None);
    }

    #[test]
    fn summary_tracks_actions_and_years() {
        let mut summary = Summary::default();
        let item = PlanItem {
            source: PathBuf::from("/a.jpg"),
            destination: PathBuf::from("/b.jpg"),
            when: Some(date()),
            origin: Source::Exif,
            action: Action::Move,
        };
        summary.record(&item);
        assert_eq!(summary.total, 1);
        assert_eq!(summary.moved, 1);
        assert_eq!(summary.by_year.get(&2026), Some(&1));

        let undated = PlanItem {
            source: PathBuf::from("/c.jpg"),
            destination: PathBuf::from("/c.jpg"),
            when: None,
            origin: Source::Unknown,
            action: Action::NoDate,
        };
        summary.record(&undated);
        assert_eq!(summary.total, 2);
        assert_eq!(summary.without_date, 1)
    }
}

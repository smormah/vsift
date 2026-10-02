//! Marks one trial workspace as trusted in Claude Code's trial home.
//!
//! Claude Code applies a project's `.claude/settings.json` permission
//! *allow* rules only in a workspace whose trust dialog was accepted; in an
//! untrusted workspace it ignores them and says so on stderr (the first dry
//! trial, 2026-09-28). A trial workspace is new every time, so the harness
//! accepts the trust for exactly that workspace before it starts the client,
//! by setting `projects[<workspace>].hasTrustDialogAccepted = true` in
//! `<CLAUDE_CONFIG_DIR>/.claude.json`.
//!
//! That file belongs to the operator's signed-in trial home and holds
//! account details, so the merge is deliberately minimal:
//!
//! - every other top-level member, every other project and every other
//!   field of this project is copied back byte for byte (members are kept
//!   as raw JSON text, in their original order);
//! - the new file is written beside the old one and renamed over it, so a
//!   failure leaves the old file whole;
//! - nothing read from the file is ever logged or put into an error: a
//!   malformed file is reported by line and column only.

use std::{
    fmt,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor},
};
use serde_json::value::RawValue;

use crate::error::TrialError;

/// The file in the client home that holds per-project state.
pub const CLAUDE_STATE_FILE: &str = ".claude.json";

/// The member of a project entry that records the accepted trust dialog.
const TRUST_FIELD: &str = "hasTrustDialogAccepted";

/// The member that maps project paths to their entries.
const PROJECTS: &str = "projects";

/// What [`trust_workspace`] did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustOutcome {
    /// The workspace was already trusted; the file was not rewritten.
    AlreadyTrusted,
    /// The trust flag was set and the file replaced.
    Marked,
}

/// A JSON object whose members stay raw text in their original order.
struct OrderedObject(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for OrderedObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;

        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = OrderedObject;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry::<String, Box<RawValue>>()? {
                    members.push(member);
                }
                Ok(OrderedObject(members))
            }
        }

        deserializer.deserialize_map(ObjectVisitor)
    }
}

impl OrderedObject {
    fn parse(text: &str, what: &str) -> Result<Self, TrialError> {
        serde_json::from_str(text).map_err(|error| {
            // The parser's message can quote the offending value, which may
            // be account data; only its position leaves this function.
            TrialError::Invalid(format!(
                "{what} in the Claude Code home's {CLAUDE_STATE_FILE} is not a JSON object \
                 (line {}, column {})",
                error.line(),
                error.column()
            ))
        })
    }

    /// The single member named `name`; two members of that name are refused
    /// because a reader could honour either.
    fn position(&self, name: &str, what: &str) -> Result<Option<usize>, TrialError> {
        let mut found = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, (key, _))| key == name)
            .map(|(index, _)| index);
        let first = found.next();
        if found.next().is_some() {
            return Err(TrialError::Invalid(format!(
                "{what} in the Claude Code home's {CLAUDE_STATE_FILE} names a member twice"
            )));
        }
        Ok(first)
    }

    /// Writes the object with one member per line at `depth` (two spaces per
    /// level), each value as its original text.
    fn render(&self, depth: usize) -> Result<String, TrialError> {
        if self.0.is_empty() {
            return Ok("{}".to_owned());
        }
        let inner = "  ".repeat(depth + 1);
        let mut members = Vec::with_capacity(self.0.len());
        for (key, value) in &self.0 {
            let key = serde_json::to_string(key)
                .map_err(|error| TrialError::json("a .claude.json member name", error))?;
            members.push(format!("{inner}{key}: {}", value.get()));
        }
        Ok(format!(
            "{{\n{}\n{}}}",
            members.join(",\n"),
            "  ".repeat(depth)
        ))
    }

    fn set(&mut self, index: Option<usize>, name: &str, value: Box<RawValue>) {
        match index {
            Some(index) => {
                if let Some(member) = self.0.get_mut(index) {
                    member.1 = value;
                }
            }
            None => self.0.push((name.to_owned(), value)),
        }
    }
}

fn raw(text: String) -> Result<Box<RawValue>, TrialError> {
    RawValue::from_string(text).map_err(|error| TrialError::json("a .claude.json member", error))
}

/// The key Claude Code uses for a project: the absolute workspace path, with
/// forward slashes on Windows (`C:/vsift-trials/<trial>/workspace`, as its
/// own trust warning prints it).
#[must_use]
pub fn project_key(workspace: &Path) -> String {
    let text = workspace.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

/// Sets `projects[<key>].hasTrustDialogAccepted = true` in `text`, leaving
/// every other member as it was. `None` when it was already `true`.
///
/// # Errors
///
/// [`TrialError::Invalid`] when the text, `projects` or the project entry
/// is not a JSON object, or a member name repeats.
pub fn merge_trust(text: &str, key: &str) -> Result<Option<String>, TrialError> {
    let mut root = OrderedObject::parse(text, "the file")?;
    let projects_index = root.position(PROJECTS, "the file")?;
    let mut projects = match projects_index.and_then(|index| root.0.get(index)) {
        Some((_, value)) => OrderedObject::parse(value.get(), "`projects`")?,
        None => OrderedObject(Vec::new()),
    };
    let entry_index = projects.position(key, "`projects`")?;
    let mut entry = match entry_index.and_then(|index| projects.0.get(index)) {
        Some((_, value)) => OrderedObject::parse(value.get(), "the trial's project entry")?,
        None => OrderedObject(Vec::new()),
    };
    let trust_index = entry.position(TRUST_FIELD, "the trial's project entry")?;
    if trust_index
        .and_then(|index| entry.0.get(index))
        .is_some_and(|(_, value)| value.get().trim() == "true")
    {
        return Ok(None);
    }
    entry.set(trust_index, TRUST_FIELD, raw("true".to_owned())?);
    projects.set(entry_index, key, raw(entry.render(2)?)?);
    root.set(projects_index, PROJECTS, raw(projects.render(1)?)?);
    Ok(Some(format!("{}\n", root.render(0)?)))
}

/// Marks `workspace` as trusted in `<client_home>/.claude.json`.
///
/// A missing file is created with only the trust entry (Claude Code adds
/// the rest on its first start). The client home itself must exist: it is
/// the operator's signed-in trial home.
///
/// # Errors
///
/// [`TrialError::Refused`] when the client home is not a directory;
/// [`TrialError::Invalid`] when the file is not the JSON object Claude Code
/// writes; [`TrialError::Io`] when it cannot be read or replaced.
pub fn trust_workspace(client_home: &Path, workspace: &Path) -> Result<TrustOutcome, TrialError> {
    if !client_home.is_dir() {
        return Err(TrialError::Refused(
            "the Claude Code client home is not a directory; sign the client in first".to_owned(),
        ));
    }
    let path = client_home.join(CLAUDE_STATE_FILE);
    let text = match fs::read(&path) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| {
            TrialError::Invalid(format!(
                "the Claude Code home's {CLAUDE_STATE_FILE} is not UTF-8"
            ))
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_owned(),
        Err(error) => return Err(TrialError::io_step("reading", &path, error)),
    };
    // A byte-order mark is not JSON; Claude Code does not write one.
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match merge_trust(text, &project_key(workspace))? {
        None => Ok(TrustOutcome::AlreadyTrusted),
        Some(merged) => {
            replace(&path, merged.as_bytes())?;
            Ok(TrustOutcome::Marked)
        }
    }
}

/// Writes `bytes` beside `path` and renames the copy over it.
fn replace(path: &Path, bytes: &[u8]) -> Result<(), TrialError> {
    let staged = staged_path(path);
    let written = File::create(&staged)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .map_err(|error| TrialError::io_step("writing the staged copy", &staged, error))
        .and_then(|()| {
            fs::rename(&staged, path)
                .map_err(|error| TrialError::io_step("renaming the staged copy over", path, error))
        });
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    written
}

fn staged_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(format!(".vsift-trials-{}.tmp", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "C:/vsift-trials/a-08-f05-local-asr-0000/workspace";

    fn parse(text: &str) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_str(text)
    }

    #[test]
    fn the_flag_is_added_and_every_other_member_is_kept_in_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = "{\n  \"userID\": \"abc\",\n  \"oauthAccount\": {\"emailAddress\": \"x@example.invalid\", \"n\": 1.50},\n  \"projects\": {\n    \"D:/other\": {\"allowedTools\": [], \"hasTrustDialogAccepted\": false}\n  },\n  \"tail\": [1, 2]\n}\n";
        let merged = merge_trust(original, KEY)?.ok_or("nothing merged")?;
        let value = parse(&merged)?;
        assert_eq!(value["projects"][KEY]["hasTrustDialogAccepted"], true);
        assert_eq!(
            value["projects"]["D:/other"]["hasTrustDialogAccepted"],
            false
        );
        assert_eq!(value["userID"], "abc");
        // Raw members come back byte for byte (the float keeps its text).
        assert!(
            merged.contains(
                "\"oauthAccount\": {\"emailAddress\": \"x@example.invalid\", \"n\": 1.50}"
            )
        );
        let order: Vec<usize> = ["\"userID\"", "\"oauthAccount\"", "\"projects\"", "\"tail\""]
            .iter()
            .filter_map(|name| merged.find(name))
            .collect();
        assert_eq!(order.len(), 4);
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
        Ok(())
    }

    #[test]
    fn an_existing_entry_keeps_its_fields_and_a_trusted_one_is_left_alone()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = format!(
            "{{\"projects\": {{\"{KEY}\": {{\"lastCost\": 0.25, \"hasTrustDialogAccepted\": false}}}}}}"
        );
        let merged = merge_trust(&original, KEY)?.ok_or("nothing merged")?;
        let value = parse(&merged)?;
        assert_eq!(value["projects"][KEY]["lastCost"], 0.25);
        assert_eq!(value["projects"][KEY]["hasTrustDialogAccepted"], true);
        assert!(merge_trust(&merged, KEY)?.is_none());
        let fresh = merge_trust("{}", KEY)?.ok_or("nothing merged")?;
        assert_eq!(
            parse(&fresh)?["projects"][KEY]["hasTrustDialogAccepted"],
            true
        );
        Ok(())
    }

    #[test]
    fn a_malformed_file_is_refused_without_quoting_it() {
        for text in [
            "[\"secret-value\"]",
            "{\"projects\": \"secret-value\"}",
            "{\"projects\": {}, \"projects\": {}}",
            "{\"oauthAccount\": \"secret-value\"",
        ] {
            let outcome = merge_trust(text, KEY);
            let message = match &outcome {
                Err(TrialError::Invalid(message)) => message.clone(),
                other => format!("unexpected outcome: {other:?}"),
            };
            assert!(
                matches!(outcome, Err(TrialError::Invalid(_))),
                "{text}: {message}"
            );
            assert!(!message.contains("secret-value"), "{message}");
        }
    }

    #[test]
    fn trust_is_written_atomically_into_the_client_home() -> Result<(), Box<dyn std::error::Error>>
    {
        // The process id, a counter and the clock: macOS reports time in
        // microseconds, so the clock alone can repeat (issue #205).
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let home = std::env::temp_dir().join(format!(
            "vsift-trials-trust-{}-{sequence}-{stamp}",
            std::process::id()
        ));
        let workspace = home.join("trial").join("workspace");
        assert!(matches!(
            trust_workspace(&home, &workspace),
            Err(TrialError::Refused(_))
        ));
        fs::create_dir_all(&home)?;
        fs::write(
            home.join(CLAUDE_STATE_FILE),
            "\u{feff}{\"userID\": \"abc\"}",
        )?;
        assert_eq!(trust_workspace(&home, &workspace)?, TrustOutcome::Marked);
        assert_eq!(
            trust_workspace(&home, &workspace)?,
            TrustOutcome::AlreadyTrusted
        );
        let value = parse(&fs::read_to_string(home.join(CLAUDE_STATE_FILE))?)?;
        assert_eq!(value["userID"], "abc");
        assert_eq!(
            value["projects"][project_key(&workspace)]["hasTrustDialogAccepted"],
            true
        );
        let leftovers = fs::read_dir(&home)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        fs::remove_dir_all(&home)?;
        Ok(())
    }
}

//! The guard on committed trial records (P12's neutral-path rules, kept for
//! P14): a record or a summary never names a drive, a home or profile
//! folder, the trial root, a client home, an e-mail address or the
//! operating-system user. `record` replaces them with tokens; this test
//! reads every committed file and fails if one slipped through.

mod common;

use std::{error::Error, fs, path::Path};

use common::repository;

type TestResult = Result<(), Box<dyn Error>>;

/// Where each kind of committed trial record lives.
const FOLDERS: [&str; 2] = [
    "docs/planning/p12-agent-trials",
    "docs/planning/p14-agent-trials",
];

fn is_word(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// A drive path (`C:\`, `C:\\`, `C:/`) that does not continue a word.
fn has_drive_path(text: &str) -> bool {
    let characters: Vec<char> = text.chars().collect();
    characters.windows(3).enumerate().any(|(index, window)| {
        window[0].is_ascii_alphabetic()
            && window[1] == ':'
            && matches!(window[2], '\\' | '/')
            && (index == 0 || !is_word(characters[index - 1]))
    })
}

/// Something shaped like an address: word characters, `@`, a dotted domain.
fn has_email(text: &str) -> bool {
    let characters: Vec<char> = text.chars().collect();
    characters.iter().enumerate().any(|(index, character)| {
        *character == '@'
            && index > 0
            && (is_word(characters[index - 1]) || characters[index - 1] == '.')
            && characters[index + 1..]
                .iter()
                .take_while(|next| is_word(**next) || matches!(**next, '.' | '-'))
                .collect::<String>()
                .contains('.')
    })
}

/// Every private-looking thing in `text`, for the user names given.
fn problems(text: &str, user_names: &[String]) -> Vec<String> {
    let lowered = text.to_lowercase();
    let mut found = Vec::new();
    if has_drive_path(text) {
        found.push("a drive path".to_owned());
    }
    if has_email(text) {
        found.push("an e-mail address".to_owned());
    }
    for marker in [
        "/home/",
        "/users/",
        "/root/",
        "\\users\\",
        "\\\\users\\\\",
        "vsift-trials",
        ".clients",
    ] {
        if lowered.contains(marker) {
            found.push((*marker).to_owned());
        }
    }
    for name in user_names.iter().filter(|name| name.chars().count() >= 3) {
        if lowered.contains(&name.to_lowercase()) {
            found.push("the operating-system user name".to_owned());
        }
    }
    found
}

fn user_names() -> Vec<String> {
    ["USERNAME", "USER", "LOGNAME"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .filter(|name| !name.trim().is_empty())
        .collect()
}

fn committed_files(folder: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn Error>> {
    let mut found = Vec::new();
    if !folder.is_dir() {
        return Ok(found);
    }
    let mut pending = vec![folder.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json" || extension == "md")
            {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

#[test]
fn the_scan_finds_what_it_is_meant_to_find_and_nothing_else() {
    for private in [
        "C:\\\\vsift-trials\\\\a",
        "c:/work/x",
        "at /home/someone/file",
        "see /Users/someone",
        "mail me at someone@example.com",
        "the .clients folder",
    ] {
        assert!(!problems(private, &[]).is_empty(), "{private}");
    }
    assert!(!problems("hello Jordan", &["Jordan".to_owned()]).is_empty());
    for fine in [
        "https://registry.npmjs.org/vsift-cli/-/vsift-cli-0.1.0.tgz",
        "<workspace>\\walkthrough.mp4 and <session-root>/x",
        "the package @vsift/win32-x64 and @next",
        "vsift 0.1.0 (0123456789ab)",
        "ses_0123456789abcdef: 1:2 a:b",
    ] {
        assert!(
            problems(fine, &["Jordan".to_owned()]).is_empty(),
            "{fine}: {:?}",
            problems(fine, &["Jordan".to_owned()])
        );
    }
}

#[test]
fn committed_trial_records_name_nothing_private() -> TestResult {
    let names = user_names();
    let mut checked = 0;
    for folder in FOLDERS {
        for file in committed_files(&repository().join(folder))? {
            let text = fs::read_to_string(&file)?;
            let found = problems(&text, &names);
            assert!(found.is_empty(), "{} holds {found:?}", file.display());
            checked += 1;
        }
    }
    assert!(checked > 0, "no committed trial file was found to check");
    Ok(())
}

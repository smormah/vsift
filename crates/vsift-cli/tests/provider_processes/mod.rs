//! The provider processes a `vsift` command started, told apart from every
//! other process by more than their number (the P10 and P11 checkpoints'
//! "no provider left running" checks, #404).
//!
//! **Why a number is not enough on Windows.** A process id is reused as soon
//! as the process that had it is gone, and a process keeps the id of the
//! process that created it as its parent id after that process has gone. So
//! a list of the processes whose parent id is the command's can hold
//! processes the command never started: at boot, Windows starts `csrss.exe`
//! and `wininit.exe` (or `winlogon.exe`) from an instance of `smss.exe` that
//! then exits, and a command that later gets that instance's id appears to
//! have two children that run for as long as the machine does. That is what
//! the hosted Windows journeys on `0.2.1` reported as "providers still
//! running after the batch ended: [760, 856]" (inferred from the pair, the
//! ten-second wait it outlived and the Job Object that kills a command's
//! providers when the command ends; the report named only the numbers). A
//! dead provider's number can also be taken by a new process within the
//! wait, and a check by number then sees it still running.
//!
//! So a process is identified here by its id and, on Windows, its creation
//! time: a child of the command was created no earlier than the command, and
//! a provider still runs only while a process with its id *and* its creation
//! time does. On Unix an orphan is re-parented and an id is not reused within
//! a check's seconds, so the id alone stays the identity there, as before.
//! The diagnostics name each process, so the next failure says what ran.
//!
//! Only the listing differs by platform ([`ProcessTable::now`]); the rules are
//! plain functions over a listing, tested on every platform below.

#![allow(
    dead_code,
    reason = "each checkpoint crate includes this module and uses only part of it"
)]

use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    process::{Command, Stdio},
};

/// The console host Windows starts for a console of its own: never a
/// provider, so it is never counted as one.
const CONSOLE_HOST: &str = "conhost.exe";

/// How many listings [`ProcessTable::now`] takes before it gives up on one
/// that names this test's own process.
const LISTING_ATTEMPTS: u32 = 5;

/// One running process as the operating system lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListedProcess {
    /// Its process id.
    pub id: u32,
    /// The id it names as its parent, which on Windows may belong to a
    /// process that has gone (or to a later one that reuses the id).
    pub parent: u32,
    /// Its creation time as a Windows file time (100 ns units since 1601),
    /// or `None` where the listing gives none (Unix).
    pub created: Option<u64>,
    /// Its image name.
    pub name: String,
}

/// A process told apart from a later one that reuses its number.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProcessIdentity {
    /// Its process id.
    pub id: u32,
    /// Its creation time, where the listing gives one.
    pub created: Option<u64>,
    /// Its image name, for diagnostics.
    pub name: String,
}

impl ProcessIdentity {
    fn of(process: &ListedProcess) -> Self {
        Self {
            id: process.id,
            created: process.created,
            name: process.name.clone(),
        }
    }

    /// Whether `process` is this one: the same id, and the same creation
    /// time where both are known.
    fn is(&self, process: &ListedProcess) -> bool {
        self.id == process.id
            && match (self.created, process.created) {
                (Some(mine), Some(theirs)) => mine == theirs,
                _ => true,
            }
    }
}

impl fmt::Display for ProcessIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.id, self.name)
    }
}

/// The processes listed together, at one moment.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessTable(Vec<ListedProcess>);

impl ProcessTable {
    /// A table of the given rows (the tests' listings).
    pub fn of(rows: Vec<ListedProcess>) -> Self {
        Self(rows)
    }

    /// The process with `id`, if it is running.
    pub fn identify(&self, id: u32) -> Option<ProcessIdentity> {
        self.0
            .iter()
            .find(|process| process.id == id)
            .map(ProcessIdentity::of)
    }

    /// The processes `parent` started that are running: they name it as
    /// their parent and, where creation times are known, were created no
    /// earlier than it. The console host is left out.
    pub fn children_of(&self, parent: &ProcessIdentity) -> BTreeSet<ProcessIdentity> {
        self.0
            .iter()
            .filter(|process| process.parent == parent.id && process.name != CONSOLE_HOST)
            .filter(|process| match (parent.created, process.created) {
                (Some(parent_created), Some(created)) => created >= parent_created,
                _ => true,
            })
            .map(ProcessIdentity::of)
            .collect()
    }

    /// Those of `wanted` that are still running: the same id and, where
    /// known, the same creation time.
    pub fn still_running<'a>(
        &self,
        wanted: impl IntoIterator<Item = &'a ProcessIdentity>,
    ) -> Vec<ProcessIdentity> {
        wanted
            .into_iter()
            .filter(|identity| self.0.iter().any(|process| identity.is(process)))
            .cloned()
            .collect()
    }

    /// Those of `wanted` that are still running now. The whole listing is
    /// read first; before it is taken to say that none of them runs, each is
    /// looked up again by its own id, so a listing that happened to miss a
    /// process cannot pass a check.
    ///
    /// # Errors
    ///
    /// A listing could not run, printed a line it does not explain, or stayed
    /// incomplete.
    pub fn still_running_now(
        wanted: &BTreeSet<ProcessIdentity>,
    ) -> Result<Vec<ProcessIdentity>, ProcessListError> {
        let running = Self::now()?.still_running(wanted);
        if !running.is_empty() {
            return Ok(running);
        }
        let mut confirmed = Vec::new();
        for identity in wanted {
            confirmed.extend(Self::listed_id(identity.id)?.still_running([identity]));
        }
        Ok(confirmed)
    }

    /// Every process running now, from a listing that names this test's
    /// own process (one that does not is incomplete and is taken again, at
    /// most [`LISTING_ATTEMPTS`] times).
    ///
    /// # Errors
    ///
    /// The listing could not run, printed a line it does not explain, or
    /// stayed incomplete.
    pub fn now() -> Result<Self, ProcessListError> {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let table = Self::listed()?;
            if table.identify(std::process::id()).is_some() {
                return Ok(table);
            }
            if attempts >= LISTING_ATTEMPTS {
                return Err(ProcessListError::Incomplete);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// One listing (Windows: a CIM query through PowerShell, with creation
    /// times; a process whose creation time cannot be read is listed with
    /// none, written `0`, rather than left out).
    #[cfg(windows)]
    fn listed() -> Result<Self, ProcessListError> {
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process | ForEach-Object { $created = if ($_.CreationDate) { $_.CreationDate.ToFileTimeUtc() } else { 0 }; '{0} {1} {2} {3}' -f $_.ProcessId, $_.ParentProcessId, $created, $_.Name }",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| ProcessListError::NotListed)?;
        if !output.status.success() {
            return Err(ProcessListError::NotListed);
        }
        parse_listing(&String::from_utf8_lossy(&output.stdout), true)
    }

    /// The process with `id` alone, if it runs (Windows: the CIM query
    /// filtered by id).
    #[cfg(windows)]
    fn listed_id(id: u32) -> Result<Self, ProcessListError> {
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "Get-CimInstance Win32_Process -Filter 'ProcessId={id}' | ForEach-Object {{ $created = if ($_.CreationDate) {{ $_.CreationDate.ToFileTimeUtc() }} else {{ 0 }}; '{{0}} {{1}} {{2}} {{3}}' -f $_.ProcessId, $_.ParentProcessId, $created, $_.Name }}"
                ),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| ProcessListError::NotListed)?;
        if !output.status.success() {
            return Err(ProcessListError::NotListed);
        }
        parse_listing(&String::from_utf8_lossy(&output.stdout), true)
    }

    /// The process with `id` alone, if it runs (Unix: `ps -p`, which exits
    /// unsuccessfully and prints nothing when there is none).
    #[cfg(unix)]
    fn listed_id(id: u32) -> Result<Self, ProcessListError> {
        let ps = ["/bin/ps", "/usr/bin/ps"]
            .into_iter()
            .find(|path| std::path::Path::new(path).is_file())
            .ok_or(ProcessListError::NotListed)?;
        let output = Command::new(ps)
            .args([
                "-p",
                &id.to_string(),
                "-o",
                "pid=",
                "-o",
                "ppid=",
                "-o",
                "comm=",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| ProcessListError::NotListed)?;
        parse_listing(&String::from_utf8_lossy(&output.stdout), false)
    }

    /// One listing (Unix: `ps`, without creation times).
    #[cfg(unix)]
    fn listed() -> Result<Self, ProcessListError> {
        let ps = ["/bin/ps", "/usr/bin/ps"]
            .into_iter()
            .find(|path| std::path::Path::new(path).is_file())
            .ok_or(ProcessListError::NotListed)?;
        let output = Command::new(ps)
            .args(["-A", "-o", "pid=", "-o", "ppid=", "-o", "comm="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| ProcessListError::NotListed)?;
        if !output.status.success() {
            return Err(ProcessListError::NotListed);
        }
        parse_listing(&String::from_utf8_lossy(&output.stdout), false)
    }
}

/// Why the processes could not be listed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessListError {
    /// The listing command did not run or did not succeed.
    NotListed,
    /// A line of the listing is not `<id> <parent> [<created>] <name>`.
    Unreadable(String),
    /// Every listing taken left out this test's own process.
    Incomplete,
}

impl fmt::Display for ProcessListError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotListed => formatter.write_str("the running processes could not be listed"),
            Self::Unreadable(line) => {
                write!(formatter, "an unreadable process listing line: {line}")
            }
            Self::Incomplete => formatter.write_str("every process listing taken was incomplete"),
        }
    }
}

impl Error for ProcessListError {}

/// Reads `<id> <parent> <created> <name>` lines (`with_created`) or
/// `<id> <parent> <name>` lines; a name may hold spaces.
pub fn parse_listing(text: &str, with_created: bool) -> Result<ProcessTable, ProcessListError> {
    let mut rows = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let unreadable = || ProcessListError::Unreadable(line.chars().take(120).collect());
        let (id, rest) = number_field(line).ok_or_else(unreadable)?;
        let (parent, rest) = number_field(rest).ok_or_else(unreadable)?;
        let (created, rest) = if with_created {
            let (created, rest) = number_field(rest).ok_or_else(unreadable)?;
            // `0`: the listing could not read this process's creation time.
            (Some(created).filter(|created| *created != 0), rest)
        } else {
            (None, rest)
        };
        rows.push(ListedProcess {
            id: u32::try_from(id).map_err(|_| unreadable())?,
            parent: u32::try_from(parent).map_err(|_| unreadable())?,
            created,
            name: rest.trim().to_owned(),
        });
    }
    Ok(ProcessTable(rows))
}

/// The leading number of `text` (after any spaces) and what follows it.
fn number_field(text: &str) -> Option<(u64, &str)> {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let (field, rest) = text.split_at(end);
    field.parse().ok().map(|number| (number, rest))
}

/// The identities as `id name` pairs, for a diagnostic.
pub fn named(processes: &[ProcessIdentity]) -> String {
    processes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::{ListedProcess, ProcessIdentity, ProcessTable, named, parse_listing};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn listed(id: u32, parent: u32, created: Option<u64>, name: &str) -> ListedProcess {
        ListedProcess {
            id,
            parent,
            created,
            name: name.to_owned(),
        }
    }

    /// The hosted `0.2.1` failure (#404), as a listing: the batch got the id
    /// of a long-gone `smss.exe`, whose two children name it as their
    /// parent. They were created before the batch, so they are not its
    /// children; its provider is.
    #[test]
    fn a_process_created_before_the_command_is_not_its_child() -> TestResult {
        let table = ProcessTable::of(vec![
            listed(760, 548, Some(100), "csrss.exe"),
            listed(856, 548, Some(110), "winlogon.exe"),
            listed(548, 412, Some(9_000), "vsift.exe"),
            listed(3_120, 548, Some(9_500), "whisper-cli.exe"),
            listed(3_124, 548, Some(9_400), "conhost.exe"),
        ]);
        let batch = table.identify(548).ok_or("the batch is not listed")?;
        let children = table.children_of(&batch);
        assert_eq!(
            children
                .into_iter()
                .map(|child| child.id)
                .collect::<Vec<_>>(),
            [3_120]
        );
        Ok(())
    }

    /// A provider that ended and whose id a new process took is not still
    /// running; one with the same id and creation time is.
    #[test]
    fn a_reused_id_is_not_the_provider_that_had_it() -> TestResult {
        let before = ProcessTable::of(vec![
            listed(548, 412, Some(9_000), "vsift.exe"),
            listed(760, 548, Some(9_500), "whisper-cli.exe"),
        ]);
        let batch = before.identify(548).ok_or("the batch is not listed")?;
        let providers = before.children_of(&batch);
        let reused = ProcessTable::of(vec![listed(760, 4, Some(12_000), "powershell.exe")]);
        assert_eq!(
            reused.still_running(&providers),
            [] as [crate::provider_processes::ProcessIdentity; 0]
        );
        let unchanged = ProcessTable::of(vec![listed(760, 1, Some(9_500), "whisper-cli.exe")]);
        let running = unchanged.still_running(&providers);
        assert_eq!(named(&running), "760 whisper-cli.exe");
        Ok(())
    }

    /// Without creation times (Unix) the ids decide, as the checks always did.
    #[test]
    fn without_creation_times_the_ids_decide() -> TestResult {
        let table = parse_listing("  1 0 launchd\n 548 1 vsift\n 760 548 whisper-cli\n", false)?;
        let batch = table.identify(548).ok_or("the batch is not listed")?;
        let providers = table.children_of(&batch);
        assert_eq!(
            providers,
            [ProcessIdentity {
                id: 760,
                created: None,
                name: "whisper-cli".to_owned(),
            }]
            .into_iter()
            .collect()
        );
        assert_eq!(table.still_running(&providers).len(), 1);
        assert_eq!(
            ProcessTable::of(Vec::new()).still_running(&providers),
            [] as [crate::provider_processes::ProcessIdentity; 0]
        );
        Ok(())
    }

    /// The Windows listing's lines, a name with spaces included; a line that
    /// is not one is refused, not skipped.
    #[test]
    fn the_listing_is_read_strictly() -> TestResult {
        let table = parse_listing(
            "760 548 134052000000000000 csrss.exe\r\n4242 760 134052000001000000 Some Tool.exe\r\n",
            true,
        )?;
        let tool = table.identify(4242).ok_or("the tool is not listed")?;
        assert_eq!(tool.name, "Some Tool.exe");
        assert_eq!(tool.created, Some(134_052_000_001_000_000));
        let unknown = parse_listing("4300 760 0 vsift.exe\n", true)?;
        assert_eq!(
            unknown.identify(4300).and_then(|process| process.created),
            None
        );
        assert!(parse_listing("not a process\n", true).is_err());
        assert!(parse_listing("760 548 csrss.exe\n", true).is_err());
        Ok(())
    }
}

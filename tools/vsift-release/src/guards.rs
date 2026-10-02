//! The guards a publish plan states and, when it is enforced, obeys (P14 PR
//! 8, ADR 0024 decisions A and B).
//!
//! A plan for a **stable** version moves npm's `latest` on all four packages,
//! the one thing the workflow can never take back (it never runs `npm
//! dist-tag`; a bad version is deprecated and replaced, `release.md` section
//! 6.5). So the plan lists, before anything runs:
//!
//! 1. that the version is stable (no pre-release suffix);
//! 2. the **accepted candidate**: the stable commit differs from the highest
//!    `v<X.Y.Z>-rc.<N>` tag only in version strings and the launcher's README
//!    (see the `candidate` module);
//! 3. that this candidate is **published** on npm, for all four packages;
//! 4. that **`latest` moves forward**: on every package it is now a stable
//!    version below this one (or already this one, with the same bytes, which
//!    is a re-run completing a partial publish), and that this version is not
//!    on npm under another dist-tag or with other bytes;
//! 5. that the registry could be read at all;
//! 6. what is not checked here: the evidence ledger's completeness.
//!
//! A plan for a pre-release states that `latest` is not touched, and checks
//! the little the registry can say about it (the version is not already
//! `latest`, and is not on npm with other bytes).
//!
//! A failed guard stops a run only when the plan is *enforced*: a publish, or
//! a dispatch on the release tag even with `dry_run` set, which is the
//! rehearsal of one. Every other run (a pull request, a push, a dispatch on a
//! branch) reports the same findings and carries on, so a working tree whose
//! version has no candidate yet does not fail every pull request.

use crate::{
    candidate::CandidateObservation,
    publish::{Channel, NpmPublication, ReleaseVersion, publication_order},
    registry::{PackageRecord, PackageState, RegistryObservation},
};

/// What a guard found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GuardOutcome {
    /// The condition holds.
    Passed,
    /// The condition does not hold; an enforced plan is refused.
    Failed,
    /// Nothing in this workflow checks it; the plan says so.
    NotEnforced,
}

impl GuardOutcome {
    /// The word the plan's table shows.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "**FAILED**",
            Self::NotEnforced => "not enforced",
        }
    }

    /// The word the plan's JSON holds.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotEnforced => "not_enforced",
        }
    }
}

/// One condition, its result and what was seen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Guard {
    /// What it checks.
    pub name: &'static str,
    /// What it found.
    pub outcome: GuardOutcome,
    /// What was seen, in a sentence.
    pub detail: String,
}

impl Guard {
    fn new(name: &'static str, outcome: GuardOutcome, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome,
            detail: detail.into(),
        }
    }

    fn passed(name: &'static str, detail: impl Into<String>) -> Self {
        Self::new(name, GuardOutcome::Passed, detail)
    }

    fn failed(name: &'static str, detail: impl Into<String>) -> Self {
        Self::new(name, GuardOutcome::Failed, detail)
    }
}

/// What the plan job observed outside the archives: the registry and the
/// candidate tag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Observations {
    /// The four packages' public metadata.
    pub registry: RegistryObservation,
    /// The comparison with the accepted candidate (stable versions only).
    pub candidate: CandidateObservation,
}

impl Observations {
    /// No observation at all, for a plan made without a registry or git.
    #[cfg(test)]
    pub(crate) const fn none() -> Self {
        Self {
            registry: RegistryObservation::NotRead,
            candidate: CandidateObservation::NotApplicable,
        }
    }
}

/// One dist-tag of one package, and what a publication does to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DistTagMove {
    /// The package.
    pub package: &'static str,
    /// The tag the publication sets: `latest` for a stable version, `next`
    /// for a pre-release.
    pub tag: &'static str,
    /// What it names now.
    pub from: String,
    /// What it names after: this version.
    pub to: String,
    /// The tag the publication leaves alone.
    pub untouched_tag: &'static str,
    /// What that one names now (and after).
    pub untouched: String,
}

/// The guards and the dist-tag moves of one plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Evaluation {
    /// Every guard, in the order the plan shows them.
    pub guards: Vec<Guard>,
    /// One move per package, in publication order.
    pub moves: Vec<DistTagMove>,
}

/// Evaluates every guard of the version's channel.
pub(crate) fn evaluate(
    version: &ReleaseVersion,
    npm: &[NpmPublication],
    observations: &Observations,
) -> Evaluation {
    let channel = version.kind().channel();
    let guards = match channel {
        Channel::Stable => stable_guards(version, npm, observations),
        Channel::PreRelease => prerelease_guards(version, npm, observations),
    };
    Evaluation {
        guards,
        moves: moves(version, channel, npm, &observations.registry),
    }
}

fn stable_guards(
    version: &ReleaseVersion,
    npm: &[NpmPublication],
    observations: &Observations,
) -> Vec<Guard> {
    let candidate_version = match &observations.candidate {
        CandidateObservation::Checked(report) => Some(report.candidate_version.as_str()),
        CandidateObservation::NotApplicable | CandidateObservation::Failed(_) => None,
    };
    vec![
        Guard::passed(
            "Stable version",
            format!(
                "`{}` has no pre-release suffix, so it is published under `latest`, which is \
                 moved on all four packages",
                version.as_str()
            ),
        ),
        candidate_guard(&observations.candidate),
        registry_read_guard(&observations.registry),
        candidate_published_guard(&observations.registry, candidate_version),
        latest_forward_guard(version, npm, &observations.registry),
        Guard::new(
            "Evidence ledger",
            GuardOutcome::NotEnforced,
            "this workflow does not check the evidence ledger's completeness for the candidate \
             (known limit L-106); run that check and read its result before dispatching \
             (release.md section 6.7)",
        ),
    ]
}

fn candidate_guard(candidate: &CandidateObservation) -> Guard {
    const NAME: &str = "Accepted candidate";
    match candidate {
        CandidateObservation::Checked(report) => {
            let violations = report.violations();
            if violations.is_empty() {
                Guard::passed(NAME, report.summary())
            } else {
                Guard::failed(NAME, violations.join("; "))
            }
        }
        CandidateObservation::Failed(reason) => Guard::failed(NAME, reason.clone()),
        CandidateObservation::NotApplicable => {
            Guard::failed(NAME, "no comparison with a release candidate was made")
        }
    }
}

/// The records of the packages that could be read, and the problems with the
/// others, in publication order.
fn read_records(
    registry: &RegistryObservation,
) -> Result<Vec<(&'static str, &PackageRecord)>, String> {
    let RegistryObservation::Read(packages) = registry else {
        return Err(String::from(
            "the registry was not read: the plan job reads it before `publish-plan` runs",
        ));
    };
    let mut records = Vec::new();
    let mut problems = Vec::new();
    for package in packages {
        match &package.state {
            PackageState::Found(record) => records.push((package.package, record)),
            PackageState::NotFound => problems.push((
                package.package,
                String::from(
                    "is not on npm (a trusted publisher can be set only on an existing package)",
                ),
            )),
            PackageState::Unreadable(reason) => {
                problems.push((package.package, reason.clone()));
            }
        }
    }
    if problems.is_empty() {
        Ok(records)
    } else {
        Err(describe(&problems))
    }
}

/// Problems per package, one phrase per distinct reason: `reason (on all four
/// packages)` or `reason (on `a`, `b`)`.
fn describe(problems: &[(&str, String)]) -> String {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for (package, reason) in problems {
        match groups
            .iter_mut()
            .find(|(known, _)| *known == reason.as_str())
        {
            Some((_, packages)) => packages.push(package),
            None => groups.push((reason.as_str(), vec![package])),
        }
    }
    groups
        .iter()
        .map(|(reason, packages)| {
            let on = if packages.len() == publication_order().len() {
                String::from("all four packages")
            } else {
                join_code(packages)
            };
            format!("{reason} (on {on})")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn registry_read_guard(registry: &RegistryObservation) -> Guard {
    const NAME: &str = "Registry read";
    match read_records(registry) {
        Ok(records) => Guard::passed(
            NAME,
            format!(
                "npm's public metadata of all {} packages was read",
                records.len()
            ),
        ),
        Err(reason) => Guard::failed(NAME, reason),
    }
}

fn candidate_published_guard(
    registry: &RegistryObservation,
    candidate_version: Option<&str>,
) -> Guard {
    const NAME: &str = "Candidate published";
    let Some(candidate) = candidate_version else {
        return Guard::failed(NAME, "there is no accepted candidate to look for on npm");
    };
    let records = match read_records(registry) {
        Ok(records) => records,
        Err(reason) => return Guard::failed(NAME, reason),
    };
    let missing: Vec<&str> = records
        .iter()
        .filter(|(_, record)| !record.has_version(candidate))
        .map(|(package, _)| *package)
        .collect();
    if missing.is_empty() {
        Guard::passed(
            NAME,
            format!("`{candidate}` is published on all four packages"),
        )
    } else {
        Guard::failed(
            NAME,
            format!("`{candidate}` is not published on {}", join_code(&missing)),
        )
    }
}

fn join_code(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How a package stands towards a stable publication of `version`.
#[derive(Debug, Eq, PartialEq)]
enum Standing {
    /// `latest` is a stable version below this one and this version is not
    /// on npm: the publication moves it from the given version.
    Fresh(String),
    /// This version is on npm with the planned bytes and is `latest`: a
    /// re-run that completes a partial publish.
    AlreadyLatest,
}

fn standing(
    version: &ReleaseVersion,
    planned_integrity: &str,
    record: &PackageRecord,
) -> Result<Standing, String> {
    let text = version.as_str();
    if record.has_version(text) {
        return match (record.integrity_of(text), record.latest.as_deref()) {
            (Some(integrity), Some(latest)) if integrity == planned_integrity && latest == text => {
                Ok(Standing::AlreadyLatest)
            }
            (Some(integrity), latest) if integrity == planned_integrity => Err(format!(
                "`{text}` is already on npm with these bytes but `latest` is `{}`: it was \
                 published under another dist-tag, which this workflow never changes",
                latest.unwrap_or("not set")
            )),
            _ => Err(format!(
                "`{text}` is already on npm, and not with these bytes"
            )),
        };
    }
    let Some(latest) = record.latest.as_deref() else {
        return Err(String::from(
            "it has no `latest` dist-tag to move forward from",
        ));
    };
    match ReleaseVersion::parse(latest) {
        Ok(current) if current.kind().channel() == Channel::Stable && current.precedes(version) => {
            Ok(Standing::Fresh(latest.to_owned()))
        }
        Ok(_) | Err(_) => Err(format!(
            "`latest` is `{latest}`, which is not a stable version below `{text}`"
        )),
    }
}

fn latest_forward_guard(
    version: &ReleaseVersion,
    npm: &[NpmPublication],
    registry: &RegistryObservation,
) -> Guard {
    const NAME: &str = "`latest` moves forward";
    let records = match read_records(registry) {
        Ok(records) => records,
        Err(reason) => return Guard::failed(NAME, reason),
    };
    let mut problems = Vec::new();
    let mut fresh: Vec<(&str, String)> = Vec::new();
    let mut already = 0_usize;
    for (package, record) in records {
        let Some(publication) = npm
            .iter()
            .find(|publication| publication.package == package)
        else {
            problems.push((package, String::from("has no planned tarball")));
            continue;
        };
        match standing(version, &publication.integrity, record) {
            Ok(Standing::Fresh(from)) => fresh.push((package, from)),
            Ok(Standing::AlreadyLatest) => already += 1,
            Err(reason) => problems.push((package, reason)),
        }
    }
    if !problems.is_empty() {
        return Guard::failed(NAME, describe(&problems));
    }
    let same = fresh
        .split_first()
        .is_some_and(|((_, first), rest)| rest.iter().all(|(_, from)| from == first));
    let detail = match (fresh.first(), same) {
        (Some((_, from)), true) if already == 0 => format!(
            "`latest` is `{from}` on all four packages, a stable version below `{}`, and this \
             version is on none of them",
            version.as_str()
        ),
        (None, _) => String::from(
            "this version is already `latest` with these bytes on all four packages: nothing \
             is left to publish",
        ),
        _ => format!(
            "{already} package(s) already hold this version as `latest` with these bytes (a \
             re-run completing a partial publish); the other {} move forward from a stable \
             version below it",
            fresh.len()
        ),
    };
    Guard::passed(NAME, detail)
}

fn prerelease_guards(
    version: &ReleaseVersion,
    npm: &[NpmPublication],
    observations: &Observations,
) -> Vec<Guard> {
    let tag = Channel::PreRelease.dist_tag();
    vec![
        Guard::passed(
            "Pre-release version",
            format!(
                "`{}` has a pre-release suffix, so it is published under `{tag}` and `latest` \
                 is not touched",
                version.as_str()
            ),
        ),
        prerelease_registry_guard(version, npm, &observations.registry),
    ]
}

fn prerelease_registry_guard(
    version: &ReleaseVersion,
    npm: &[NpmPublication],
    registry: &RegistryObservation,
) -> Guard {
    const NAME: &str = "Registry";
    let RegistryObservation::Read(packages) = registry else {
        return Guard::new(
            NAME,
            GuardOutcome::NotEnforced,
            "the registry was not read; the publish job reads the dist-tags before and after \
             publishing and stops if `latest` changed",
        );
    };
    let text = version.as_str();
    let mut problems = Vec::new();
    let mut unread = Vec::new();
    for package in packages {
        match &package.state {
            PackageState::Found(record) => {
                let planned = npm
                    .iter()
                    .find(|publication| publication.package == package.package)
                    .map(|publication| publication.integrity.as_str());
                if record.latest.as_deref() == Some(text) {
                    problems.push((
                        package.package,
                        String::from("this pre-release is already `latest`"),
                    ));
                }
                if record.has_version(text) && record.integrity_of(text) != planned {
                    problems.push((
                        package.package,
                        format!("`{text}` is already on npm, and not with these bytes"),
                    ));
                }
            }
            PackageState::NotFound | PackageState::Unreadable(_) => unread.push(package.package),
        }
    }
    if !problems.is_empty() {
        Guard::failed(NAME, describe(&problems))
    } else if !unread.is_empty() {
        Guard::new(
            NAME,
            GuardOutcome::NotEnforced,
            format!("the registry could not be read for {}", join_code(&unread)),
        )
    } else {
        Guard::passed(
            NAME,
            "this version is not `latest` and is not on npm with other bytes on any package",
        )
    }
}

fn moves(
    version: &ReleaseVersion,
    channel: Channel,
    npm: &[NpmPublication],
    registry: &RegistryObservation,
) -> Vec<DistTagMove> {
    let tag = channel.dist_tag();
    let untouched_tag = channel.untouched_dist_tag();
    npm.iter()
        .map(|publication| DistTagMove {
            package: publication.package,
            tag,
            from: tag_text(registry, publication.package, tag),
            to: version.as_str().to_owned(),
            untouched_tag,
            untouched: tag_text(registry, publication.package, untouched_tag),
        })
        .collect()
}

/// What a dist-tag names now, as the plan shows it.
fn tag_text(registry: &RegistryObservation, package: &str, tag: &str) -> String {
    let RegistryObservation::Read(packages) = registry else {
        return String::from("not read");
    };
    match packages.iter().find(|observed| observed.package == package) {
        Some(observed) => match &observed.state {
            PackageState::Found(record) => {
                let named = if tag == Channel::Stable.dist_tag() {
                    record.latest.as_deref()
                } else {
                    record.next.as_deref()
                };
                named.unwrap_or("not set").to_owned()
            }
            PackageState::NotFound => String::from("no such package"),
            PackageState::Unreadable(_) => String::from("unreadable"),
        },
        None => String::from("not read"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Guard, GuardOutcome, Observations, Standing, evaluate, standing, tag_text};
    use crate::{
        candidate::{CandidateObservation, CandidateReport, ChangeClass, ChangedPath},
        publish::{Digested, NpmPublication, ReleaseVersion, publication_order},
        registry::{PackageObservation, PackageRecord, PackageState, RegistryObservation},
    };

    pub(super) const PLANNED: &str = "sha512-planned";

    fn publications() -> Vec<NpmPublication> {
        publication_order()
            .into_iter()
            .map(|package| NpmPublication {
                package,
                tarball: Digested {
                    name: format!("{package}.tgz"),
                    sha256: String::from("00"),
                },
                integrity: String::from(PLANNED),
            })
            .collect()
    }

    fn record(
        latest: &str,
        next: Option<&str>,
        versions: &[(&str, Option<&str>)],
    ) -> PackageRecord {
        PackageRecord {
            latest: Some(latest.to_owned()),
            next: next.map(str::to_owned),
            versions: versions
                .iter()
                .map(|(version, integrity)| ((*version).to_owned(), integrity.map(str::to_owned)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn registry(per_package: &[PackageState]) -> RegistryObservation {
        RegistryObservation::Read(
            publication_order()
                .into_iter()
                .zip(per_package.iter().cloned())
                .map(|(package, state)| PackageObservation { package, state })
                .collect(),
        )
    }

    fn all_found(record: &PackageRecord) -> RegistryObservation {
        registry(&vec![PackageState::Found(record.clone()); 4])
    }

    fn report(violating: bool) -> CandidateObservation {
        CandidateObservation::Checked(Box::new(CandidateReport {
            candidate_tag: String::from("v0.2.0-rc.1"),
            candidate_version: String::from("0.2.0-rc.1"),
            candidate_commit: "a".repeat(40),
            stable_commit: "b".repeat(40),
            ancestor: true,
            changes: vec![
                ChangedPath {
                    path: String::from("Cargo.toml"),
                    verdict: Ok(ChangeClass::VersionString),
                },
                ChangedPath {
                    path: String::from("crates/x.rs"),
                    verdict: if violating {
                        Err(String::from("may not differ"))
                    } else {
                        Ok(ChangeClass::VersionString)
                    },
                },
            ],
        }))
    }

    fn guard<'a>(guards: &'a [Guard], name: &str) -> Option<&'a Guard> {
        guards.iter().find(|guard| guard.name == name)
    }

    fn healthy_registry() -> RegistryObservation {
        all_found(&record(
            "0.0.0",
            Some("0.2.0-rc.1"),
            &[("0.0.0", None), ("0.2.0-rc.1", Some("sha512-rc"))],
        ))
    }

    #[test]
    fn a_healthy_stable_plan_passes_every_guard_but_the_evidence_ledger()
    -> Result<(), Box<dyn std::error::Error>> {
        let version = ReleaseVersion::parse("0.2.0")?;
        let observations = Observations {
            registry: healthy_registry(),
            candidate: report(false),
        };
        let evaluation = evaluate(&version, &publications(), &observations);
        let outcomes: Vec<(&str, GuardOutcome)> = evaluation
            .guards
            .iter()
            .map(|guard| (guard.name, guard.outcome))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("Stable version", GuardOutcome::Passed),
                ("Accepted candidate", GuardOutcome::Passed),
                ("Registry read", GuardOutcome::Passed),
                ("Candidate published", GuardOutcome::Passed),
                ("`latest` moves forward", GuardOutcome::Passed),
                ("Evidence ledger", GuardOutcome::NotEnforced),
            ]
        );
        assert_eq!(evaluation.moves.len(), 4);
        for moved in &evaluation.moves {
            assert_eq!(moved.tag, "latest");
            assert_eq!(moved.from, "0.0.0");
            assert_eq!(moved.to, "0.2.0");
            assert_eq!(moved.untouched_tag, "next");
            assert_eq!(moved.untouched, "0.2.0-rc.1");
        }
        Ok(())
    }

    #[test]
    fn each_stable_guard_fails_on_its_own_violation() -> Result<(), Box<dyn std::error::Error>> {
        let version = ReleaseVersion::parse("0.2.0")?;
        let healthy = healthy_registry();
        let failing = |observations: &Observations, name: &str| -> bool {
            let evaluation = evaluate(&version, &publications(), observations);
            guard(&evaluation.guards, name)
                .is_some_and(|guard| guard.outcome == GuardOutcome::Failed)
        };
        // A candidate that differs by more than versions.
        assert!(failing(
            &Observations {
                registry: healthy.clone(),
                candidate: report(true)
            },
            "Accepted candidate"
        ));
        // No candidate found, or the comparison could not be made.
        for candidate in [
            CandidateObservation::Failed(String::from("no release candidate tag")),
            CandidateObservation::NotApplicable,
        ] {
            assert!(failing(
                &Observations {
                    registry: healthy.clone(),
                    candidate
                },
                "Accepted candidate"
            ));
        }
        // The registry was not read, or a package is missing or unreadable.
        for unread in [
            RegistryObservation::NotRead,
            registry(&[
                PackageState::NotFound,
                PackageState::Unreadable(String::from("the registry answered HTTP 503")),
                PackageState::Found(PackageRecord::default()),
                PackageState::Found(PackageRecord::default()),
            ]),
        ] {
            let observations = Observations {
                registry: unread,
                candidate: report(false),
            };
            for name in [
                "Registry read",
                "Candidate published",
                "`latest` moves forward",
            ] {
                assert!(failing(&observations, name), "{name}");
            }
        }
        // The candidate is not on one package.
        let mut records =
            vec![PackageState::Found(record("0.0.0", None, &[("0.2.0-rc.1", None)],)); 4];
        records[2] = PackageState::Found(record("0.0.0", None, &[("0.0.0", None)]));
        let observations = Observations {
            registry: registry(&records),
            candidate: report(false),
        };
        assert!(failing(&observations, "Candidate published"));
        Ok(())
    }

    #[test]
    fn latest_never_moves_backwards_sideways_or_off_a_pre_release()
    -> Result<(), Box<dyn std::error::Error>> {
        let version = ReleaseVersion::parse("0.2.0")?;
        for bad in [
            "0.2.0",
            "0.3.0",
            "1.0.0",
            "0.2.0-rc.1",
            "0.1.0-rc.1",
            "not-a-version",
        ] {
            // `bad` is `latest` while this version is not on npm.
            let result = standing(&version, PLANNED, &record(bad, None, &[(bad, None)]));
            assert!(result.is_err(), "{bad}: {result:?}");
        }
        for good in ["0.0.0", "0.1.0", "0.1.9"] {
            assert_eq!(
                standing(&version, PLANNED, &record(good, None, &[])),
                Ok(Standing::Fresh(good.to_owned())),
                "{good}"
            );
        }
        // No latest at all.
        let none = PackageRecord::default();
        assert!(standing(&version, PLANNED, &none).is_err());
        // Already published: with the planned bytes and as latest is a re-run; any
        // other state is refused.
        let same = record("0.2.0", Some("0.2.0-rc.1"), &[("0.2.0", Some(PLANNED))]);
        assert_eq!(
            standing(&version, PLANNED, &same),
            Ok(Standing::AlreadyLatest)
        );
        let other_tag = record("0.0.0", Some("0.2.0"), &[("0.2.0", Some(PLANNED))]);
        assert!(
            matches!(standing(&version, PLANNED, &other_tag), Err(reason) if reason.contains("another dist-tag"))
        );
        let other_bytes = record("0.2.0", None, &[("0.2.0", Some("sha512-other"))]);
        assert!(
            matches!(standing(&version, PLANNED, &other_bytes), Err(reason) if reason.contains("not with these bytes"))
        );
        let unknown_bytes = record("0.2.0", None, &[("0.2.0", None)]);
        assert!(standing(&version, PLANNED, &unknown_bytes).is_err());
        Ok(())
    }

    #[test]
    fn a_partly_published_stable_is_completed_not_refused() -> Result<(), Box<dyn std::error::Error>>
    {
        let version = ReleaseVersion::parse("0.2.0")?;
        let published = PackageState::Found(record(
            "0.2.0",
            Some("0.2.0-rc.1"),
            &[("0.2.0", Some(PLANNED)), ("0.2.0-rc.1", None)],
        ));
        let waiting = PackageState::Found(record(
            "0.0.0",
            Some("0.2.0-rc.1"),
            &[("0.0.0", None), ("0.2.0-rc.1", None)],
        ));
        let observations = Observations {
            registry: registry(&[published.clone(), published, waiting.clone(), waiting]),
            candidate: report(false),
        };
        let evaluation = evaluate(&version, &publications(), &observations);
        let forward = guard(&evaluation.guards, "`latest` moves forward");
        assert!(
            forward.is_some_and(|guard| guard.outcome == GuardOutcome::Passed
                && guard.detail.contains("2 package(s) already hold")),
            "{forward:?}"
        );
        Ok(())
    }

    #[test]
    fn the_pre_release_plan_leaves_latest_alone_and_checks_what_it_can()
    -> Result<(), Box<dyn std::error::Error>> {
        let version = ReleaseVersion::parse("0.2.0-rc.1")?;
        let healthy = all_found(&record(
            "0.0.0",
            Some("0.1.0"),
            &[("0.0.0", None), ("0.1.0", None)],
        ));
        let evaluation = evaluate(
            &version,
            &publications(),
            &Observations {
                registry: healthy,
                candidate: CandidateObservation::NotApplicable,
            },
        );
        assert!(
            evaluation
                .guards
                .iter()
                .all(|guard| guard.outcome == GuardOutcome::Passed),
            "{:#?}",
            evaluation.guards
        );
        for moved in &evaluation.moves {
            assert_eq!(moved.tag, "next");
            assert_eq!(moved.from, "0.1.0");
            assert_eq!(moved.to, "0.2.0-rc.1");
            assert_eq!(moved.untouched_tag, "latest");
            assert_eq!(moved.untouched, "0.0.0");
        }
        // Not read: advisory only.
        let unread = evaluate(&version, &publications(), &Observations::none());
        assert!(
            guard(&unread.guards, "Registry")
                .is_some_and(|guard| guard.outcome == GuardOutcome::NotEnforced)
        );
        assert!(unread.moves.iter().all(|moved| moved.from == "not read"));
        // A pre-release that is already `latest`, or on npm with other bytes.
        for bad in [
            record("0.2.0-rc.1", None, &[]),
            record("0.0.0", None, &[("0.2.0-rc.1", Some("sha512-other"))]),
        ] {
            let evaluation = evaluate(
                &version,
                &publications(),
                &Observations {
                    registry: all_found(&bad),
                    candidate: CandidateObservation::NotApplicable,
                },
            );
            assert!(
                guard(&evaluation.guards, "Registry")
                    .is_some_and(|guard| guard.outcome == GuardOutcome::Failed),
                "{bad:?}"
            );
        }
        // An unreadable package does not fail a pre-release.
        let partly = evaluate(
            &version,
            &publications(),
            &Observations {
                registry: registry(&[
                    PackageState::Unreadable(String::from("x")),
                    PackageState::NotFound,
                    PackageState::Found(record("0.0.0", None, &[])),
                    PackageState::Found(record("0.0.0", None, &[])),
                ]),
                candidate: CandidateObservation::NotApplicable,
            },
        );
        assert!(
            guard(&partly.guards, "Registry")
                .is_some_and(|guard| guard.outcome == GuardOutcome::NotEnforced)
        );
        Ok(())
    }

    #[test]
    fn tags_are_described_for_every_state() {
        let registry = registry(&[
            PackageState::NotFound,
            PackageState::Unreadable(String::from("x")),
            PackageState::Found(record("0.0.0", None, &[])),
            PackageState::Found(record("0.0.0", Some("0.1.0"), &[])),
        ]);
        let order = publication_order();
        let text = |index: usize, tag: &str| {
            tag_text(
                &registry,
                order.get(index).copied().unwrap_or_default(),
                tag,
            )
        };
        assert_eq!(text(0, "latest"), "no such package");
        assert_eq!(text(1, "latest"), "unreadable");
        assert_eq!(text(2, "next"), "not set");
        assert_eq!(text(3, "next"), "0.1.0");
        assert_eq!(tag_text(&registry, "other", "latest"), "not read");
        assert_eq!(
            tag_text(&RegistryObservation::NotRead, "vsift-cli", "latest"),
            "not read"
        );
    }
}

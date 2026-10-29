use super::{
    AgentClient, ReplacePolicy, SkillBundle, SkillBundleError, SkillEntryName, SkillFile,
    SkillFolderState, SkillInstallRefusal, SkillInstallStep, SkillLayout, install_step,
    names_the_vsift_skill,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn claude_code_reads_its_own_folder_and_every_other_client_the_shared_one() {
    for client in AgentClient::ALL {
        let expected = if client == AgentClient::ClaudeCode {
            SkillLayout::Claude
        } else {
            SkillLayout::Agents
        };
        assert_eq!(client.layout(), expected, "{client}");
    }
    assert_eq!(
        SkillLayout::Claude.components(),
        [".claude", "skills", "vsift"]
    );
    assert_eq!(
        SkillLayout::Agents.components(),
        [".agents", "skills", "vsift"]
    );
}

#[test]
fn clients_that_share_a_folder_have_it_written_once() {
    assert_eq!(
        SkillLayout::of_clients(&[AgentClient::Codex, AgentClient::Cursor, AgentClient::Amp]),
        [SkillLayout::Agents]
    );
    assert_eq!(
        SkillLayout::of_clients(&[AgentClient::Goose, AgentClient::ClaudeCode]),
        [SkillLayout::Claude, SkillLayout::Agents]
    );
}

#[test]
fn an_install_never_overwrites_without_replace_and_never_touches_another_folder() {
    use ReplacePolicy::{Keep, Replace};
    use SkillFolderState::{Absent, Current, Different, Link, NotVsift, ParentNotFolder};
    assert_eq!(install_step(Absent, Keep), SkillInstallStep::Create);
    assert_eq!(
        install_step(Current, Replace),
        SkillInstallStep::KeepCurrent
    );
    assert_eq!(
        install_step(Different, Keep),
        SkillInstallStep::Refuse(SkillInstallRefusal::DifferentWithoutReplace)
    );
    assert_eq!(install_step(Different, Replace), SkillInstallStep::Swap);
    assert_eq!(
        install_step(Link, Keep),
        SkillInstallStep::Refuse(SkillInstallRefusal::LinkWithoutReplace)
    );
    assert_eq!(install_step(Link, Replace), SkillInstallStep::Swap);
    for policy in [Keep, Replace] {
        assert_eq!(
            install_step(NotVsift, policy),
            SkillInstallStep::Refuse(SkillInstallRefusal::NotVsift)
        );
        assert_eq!(
            install_step(ParentNotFolder, policy),
            SkillInstallStep::Refuse(SkillInstallRefusal::ParentNotFolder)
        );
    }
}

#[test]
fn an_entry_name_cannot_leave_the_skill_folder() {
    for good in [
        "SKILL.md",
        "references/commands.md",
        "workflows/bug-report.md",
    ] {
        assert!(SkillEntryName::parse(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "../SKILL.md",
        "references/../../x",
        "/etc/passwd",
        "C:/Windows/x",
        r"C:\x",
        "a//b",
        ".hidden",
        "references/.x",
        "a/b/c/d",
        r"references\commands.md",
        "white space.md",
    ] {
        assert!(SkillEntryName::parse(bad).is_err(), "{bad}");
    }
}

fn file(name: &str, bytes: &[u8]) -> Result<SkillFile, Box<dyn std::error::Error>> {
    Ok(SkillFile::new(SkillEntryName::parse(name)?, bytes.to_vec()))
}

#[test]
fn a_bundle_has_its_main_file_and_no_name_twice() -> TestResult {
    let bundle = SkillBundle::new(vec![
        file("references/a.md", b"a")?,
        file("SKILL.md", b"s")?,
    ])?;
    assert_eq!(bundle.files()[0].name().as_str(), "SKILL.md");
    assert!(bundle.file("references/a.md").is_some());
    assert_eq!(
        SkillBundle::new(vec![file("a.md", b"a")?]),
        Err(SkillBundleError::MissingSkillFile)
    );
    assert!(matches!(
        SkillBundle::new(vec![file("SKILL.md", b"s")?, file("SKILL.md", b"t")?]),
        Err(SkillBundleError::DuplicateEntry(_))
    ));
    assert!(matches!(
        SkillBundle::new(vec![
            file("SKILL.md", b"s")?,
            file("a", b"")?,
            file("a/b", b"")?
        ]),
        Err(SkillBundleError::EntryIsFolder(_))
    ));
    Ok(())
}

#[test]
fn only_a_skill_named_vsift_in_its_front_matter_counts_as_one() {
    assert!(names_the_vsift_skill(
        b"---\nname: vsift\ndescription: x\n---\n# x\n"
    ));
    assert!(names_the_vsift_skill(b"---\r\nname: vsift\r\n---\r\n"));
    assert!(!names_the_vsift_skill(
        b"---\nname: other\n---\nname: vsift\n"
    ));
    assert!(!names_the_vsift_skill(b"name: vsift\n"));
    assert!(!names_the_vsift_skill(b"---\nname: vsift-fork\n---\n"));
    assert!(!names_the_vsift_skill(&[0xff, 0xfe]));
}

//! Exercises production persistence in an isolated temporary profile, without
//! opening GPUI, connecting to a server or accessing another client's files.

use papo_desktop::persistence::{Change, PersistenceError, Profile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    if let Some(mode) = arguments.next() {
        let root = arguments.next().ok_or("missing probe child root")?;
        check(arguments.next().is_none())?;
        if mode == "--assert-locked" {
            return check(Profile::open(root.into()).err() == Some(PersistenceError::Locked));
        }
        check(mode == "--assert-restart")?;
        let profile = Profile::open(root.into())?;
        check(profile.snapshot().servers().len() == 2)?;
        check(profile.snapshot().servers()[0].url() == "https://second.example.invalid")?;
        return check(
            profile.snapshot().selected_server() == Some(profile.snapshot().servers()[1].key()),
        );
    }
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("papo-desktop");
    let mut profile = Profile::open(root.clone())?;
    check(profile.snapshot().servers().is_empty())?;
    profile.apply(Change::Add {
        url: "https://first.example.invalid".into(),
    })?;
    profile.apply(Change::Add {
        url: "https://second.example.invalid".into(),
    })?;
    let first = profile.snapshot().servers()[0].key().to_owned();
    let second = profile.snapshot().servers()[1].key().to_owned();
    profile.apply(Change::Select {
        server: Some(first.clone()),
    })?;
    profile.apply(Change::Reorder {
        servers: vec![second.clone(), first.clone()],
    })?;
    check(Profile::open(root.clone()).err() == Some(PersistenceError::Locked))?;
    let executable = std::env::current_exe()?;
    check(
        std::process::Command::new(&executable)
            .arg("--assert-locked")
            .arg(&root)
            .status()?
            .success(),
    )?;
    drop(profile);
    check(
        std::process::Command::new(executable)
            .arg("--assert-restart")
            .arg(&root)
            .status()?
            .success(),
    )?;

    let mut profile = Profile::open(root.clone())?;
    check(profile.snapshot().servers()[0].key() == second)?;
    check(profile.snapshot().selected_server() == Some(first.as_str()))?;
    profile.apply(Change::Remove { server: first })?;
    drop(profile);

    let profile = Profile::open(root)?;
    check(profile.snapshot().servers().len() == 1)?;
    check(profile.snapshot().selected_server().is_none())?;
    drop(profile);
    directory.close()?;
    report_pass();
    Ok(())
}

fn check(condition: bool) -> Result<(), Box<dyn std::error::Error>> {
    if condition {
        Ok(())
    } else {
        Err("persistence probe invariant failed".into())
    }
}

#[expect(
    clippy::print_stdout,
    reason = "the explicit probe reports only a sanitized result"
)]
fn report_pass() {
    println!(
        "persistence_probe: PASS; restart, order, selection, removal, exclusive ownership and cleanup verified; no display or GPU used"
    );
}

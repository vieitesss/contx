use super::create;
use crate::{
    config::{Command, Multiplexer, ResolvedConfig, SessionCandidate},
    utils::test_utils::TempDir,
};
use std::{fs, path::Path, process::Command as Git};

fn git(root: &Path, args: &[&str]) {
    let output = Git::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn setup(d: &TempDir) -> (std::path::PathBuf, ResolvedConfig) {
    let projects = d.child("projects");
    let repo = d.child("projects/repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    git(&repo, &["branch", "existing"]);
    let file = d.file(
        "config.toml",
        &format!("paths = [\"{}\"]\n", projects.display()),
    );
    (
        repo.clone(),
        ResolvedConfig {
            candidates: vec![SessionCandidate::new(
                repo.display().to_string(),
                projects.display().to_string(),
            )],
            multiplexer: Multiplexer::Auto,
            command: Command::Picker,
            json: true,
            permanent_delete: false,
            clone: crate::config::CloneSettings::default(),
            config_path: file.display().to_string(),
            paths: vec![projects.display().to_string()],
            git_from_home: false,
            config_existed: true,
        },
    )
}

#[test]
fn creates_existing_local_branch_at_covered_destination() {
    let d = TempDir::new();
    let (repo, cfg) = setup(&d);
    let destination = d.path().join("projects/linked");
    let result = create(
        &cfg,
        repo.to_str().unwrap(),
        "existing",
        destination.to_str().unwrap(),
        false,
        false,
    )
    .unwrap();
    assert!(destination.join(".git").is_file());
    assert_eq!(result.branch, "existing");
    assert!(result.discoverable);
    assert!(!result.config_updated);
    assert!(
        String::from_utf8(
            Git::new("git")
                .arg("-C")
                .arg(&destination)
                .args(["branch", "--show-current"])
                .output()
                .unwrap()
                .stdout
        )
        .unwrap()
        .contains("existing")
    );
}

#[test]
fn new_branch_adds_uncovered_parent_to_config_only_after_success() {
    let d = TempDir::new();
    let (repo, cfg) = setup(&d);
    let destination = d.child("other").join("linked");
    let result = create(
        &cfg,
        repo.to_str().unwrap(),
        "feature",
        destination.to_str().unwrap(),
        true,
        true,
    )
    .unwrap();
    assert!(result.config_updated);
    assert!(result.discoverable);
    assert!(destination.join(".git").is_file());
    assert!(
        fs::read_to_string(&cfg.config_path)
            .unwrap()
            .contains(&d.child("other").display().to_string())
    );
}

#[test]
fn refuses_bad_branch_or_existing_destination_without_mutation() {
    let d = TempDir::new();
    let (repo, cfg) = setup(&d);
    let destination = d.path().join("projects/linked");
    let repo = repo.to_str().unwrap();
    let dest = destination.to_str().unwrap();
    assert!(
        create(&cfg, repo, "main", dest, true, true)
            .unwrap_err()
            .contains("already exists")
    );
    assert!(
        create(&cfg, repo, "missing", dest, false, true)
            .unwrap_err()
            .contains("--new-branch")
    );
    assert!(
        create(&cfg, repo, "bad name", dest, true, true)
            .unwrap_err()
            .contains("invalid branch")
    );
    assert!(!destination.exists());
    assert!(
        !fs::read_to_string(&cfg.config_path)
            .unwrap()
            .contains("linked")
    );
    let occupied = d.child("projects/occupied");
    assert!(
        create(&cfg, repo, "fresh", occupied.to_str().unwrap(), true, false)
            .unwrap_err()
            .contains("destination already exists")
    );
    assert!(
        Git::new("git")
            .arg("-C")
            .arg(repo)
            .args(["show-ref", "--verify", "--quiet", "refs/heads/fresh"])
            .status()
            .unwrap()
            .code()
            == Some(1)
    );
}

#[test]
fn refuses_directory_inside_repo_or_non_candidate_as_repo() {
    let d = TempDir::new();
    let (repo, cfg) = setup(&d);
    let nested = d.child("projects/repo/subdir");
    let dest = d.path().join("projects/linked");
    assert!(
        create(
            &cfg,
            nested.to_str().unwrap(),
            "feature",
            dest.to_str().unwrap(),
            true,
            false
        )
        .unwrap_err()
        .contains("not a session candidate")
    );
    let mut nested_cfg = cfg.clone();
    nested_cfg.candidates.push(SessionCandidate::new(
        nested.display().to_string(),
        repo.display().to_string(),
    ));
    assert!(
        create(
            &nested_cfg,
            nested.to_str().unwrap(),
            "feature",
            dest.to_str().unwrap(),
            true,
            false
        )
        .unwrap_err()
        .contains("not a Git repository root")
    );
    assert!(!dest.exists());
}

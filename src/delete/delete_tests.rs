use super::{
    DeleteClass, DeleteError, DeleteRequest, DeleteStrategy, classify, plan,
    resolve_path, strategy,
};
use crate::{config::SessionCandidate, utils::test_utils::TempDir};
use std::{
    ffi::OsString,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
};

fn env_from<'a>(
    vars: &'a [(&str, String)],
) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        vars.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| OsString::from(v))
    }
}

fn cand(path: &Path) -> SessionCandidate {
    SessionCandidate::new(
        path.display().to_string(),
        path.parent().unwrap_or(path).display().to_string(),
    )
}

fn request(path: &str, permanent: bool, force: bool) -> DeleteRequest {
    DeleteRequest {
        path: path.to_string(),
        dry_run: false,
        permanent,
        force,
    }
}

fn git_cmd(dir: Option<&Path>, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git subprocess failed to spawn");
    assert!(
        out.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("git output is not UTF-8")
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    git_cmd(Some(dir), args)
}

fn commit(dir: &Path, name: &str, content: &str, msg: &str) {
    fs::write(dir.join(name), content).unwrap();
    git_in(dir, &["add", name]);
    git_in(
        dir,
        &[
            "-c",
            "user.email=contx-test@example.com",
            "-c",
            "user.name=contx-test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            msg,
        ],
    );
}

fn init_repo(dir: &Path) {
    git_in(dir, &["init", "-b", "topic"]);
    commit(dir, "file.txt", "one\n", "first");
}

fn add_linked_worktree(main: &Path, linked: &Path) -> PathBuf {
    git_in(main, &["branch", "side"]);
    git_in(
        main,
        &[
            "worktree",
            "add",
            linked.to_str().expect("temp path is UTF-8"),
            "side",
        ],
    );
    linked.to_path_buf()
}

#[test]
fn resolve_expands_tilde_env_and_cli_relative_from_injected_cwd() {
    let d = TempDir::new();
    let home = d.child("home");
    let cwd = d.child("cwd");
    let vars = [
        ("HOME", home.display().to_string()),
        ("WORK", d.child("work").display().to_string()),
    ];
    let env = env_from(&vars);

    let tilde = resolve_path("~/proj", &cwd, &env).unwrap();
    assert_eq!(tilde, format!("{}/proj", home.display()));

    let from_env = resolve_path("$WORK/proj", &cwd, &env).unwrap();
    assert_eq!(from_env, format!("{}/proj", d.child("work").display()));

    let relative = resolve_path("repo", &cwd, &env).unwrap();
    assert_eq!(relative, format!("{}/repo", cwd.display()));
}

#[test]
fn missing_candidate_is_not_a_session_candidate() {
    let d = TempDir::new();
    let path = d.child("nope");
    let err = plan(
        path.to_str().unwrap(),
        &[],
        d.path(),
        &|_| None,
        &request(path.to_str().unwrap(), false, false),
        false,
    )
    .unwrap_err();
    assert!(matches!(err, DeleteError::NotCandidate(_)));
}

#[test]
fn nested_candidate_stays_exact_and_is_not_widened_to_git_root() {
    let d = TempDir::new();
    let repo = d.child("group/repo");
    init_repo(&repo);
    let nested = d.child("group/repo/src");
    let candidates = [cand(&repo), cand(&nested)];
    let target = plan(
        nested.to_str().unwrap(),
        &candidates,
        d.path(),
        &|_| None,
        &request(nested.to_str().unwrap(), false, false),
        false,
    )
    .unwrap();
    assert_eq!(target.path, nested.display().to_string());
    assert_eq!(target.class, DeleteClass::OrdinaryDirectory);
    assert_eq!(target.strategy, DeleteStrategy::Trash);
    assert_ne!(target.path, repo.display().to_string());
}

#[test]
fn symlink_candidate_is_the_link_not_the_target() {
    let d = TempDir::new();
    let repo = d.child("real");
    init_repo(&repo);
    let link = d.path().join("link");
    symlink(&repo, &link).unwrap();
    let candidates = [cand(&link)];

    let by_link = plan(
        link.to_str().unwrap(),
        &candidates,
        d.path(),
        &|_| None,
        &request(link.to_str().unwrap(), false, false),
        false,
    )
    .unwrap();
    assert_eq!(by_link.path, link.display().to_string());
    assert_eq!(by_link.class, DeleteClass::Symlink);
    assert_eq!(by_link.strategy, DeleteStrategy::Trash);

    let by_target = plan(
        repo.to_str().unwrap(),
        &candidates,
        d.path(),
        &|_| None,
        &request(repo.to_str().unwrap(), true, false),
        false,
    )
    .unwrap();
    assert_eq!(by_target.path, link.display().to_string());
    assert_eq!(by_target.class, DeleteClass::Symlink);
    assert_eq!(by_target.strategy, DeleteStrategy::Permanent);
}

#[test]
fn classify_standalone_repo_versus_linked_worktree() {
    let d = TempDir::new();
    let main = d.child("main");
    init_repo(&main);
    let linked = add_linked_worktree(&main, &d.path().join("linked"));

    assert_eq!(
        classify(main.to_str().unwrap()).unwrap(),
        DeleteClass::StandaloneRepo
    );
    assert_eq!(
        classify(linked.to_str().unwrap()).unwrap(),
        DeleteClass::LinkedWorktree
    );
    assert!(main.join(".git").is_dir());
    assert!(linked.join(".git").is_file());
}

#[test]
fn linked_worktree_always_uses_git_worktree_deletion() {
    let d = TempDir::new();
    let main = d.child("main");
    init_repo(&main);
    let linked = add_linked_worktree(&main, &d.path().join("linked"));
    let candidates = [cand(&linked)];
    for (permanent, force, config_permanent) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (true, true, true),
    ] {
        let target = plan(
            linked.to_str().unwrap(),
            &candidates,
            d.path(),
            &|_| None,
            &request(linked.to_str().unwrap(), permanent, force),
            config_permanent,
        )
        .unwrap();
        assert_eq!(target.class, DeleteClass::LinkedWorktree);
        assert_eq!(target.strategy, DeleteStrategy::GitWorktree);
    }
}

#[test]
fn flag_and_config_select_permanent_trash_ignores_force() {
    assert_eq!(
        strategy(DeleteClass::OrdinaryDirectory, false, false),
        DeleteStrategy::Trash
    );
    assert_eq!(
        strategy(DeleteClass::OrdinaryDirectory, true, false),
        DeleteStrategy::Permanent
    );
    assert_eq!(
        strategy(DeleteClass::OrdinaryDirectory, false, true),
        DeleteStrategy::Permanent
    );
    assert_eq!(
        strategy(DeleteClass::StandaloneRepo, true, false),
        DeleteStrategy::Permanent
    );
    assert_eq!(
        strategy(DeleteClass::Symlink, false, false),
        DeleteStrategy::Trash
    );
    assert_eq!(
        strategy(DeleteClass::LinkedWorktree, true, true),
        DeleteStrategy::GitWorktree
    );

    let d = TempDir::new();
    let dir = d.child("plain");
    let candidates = [cand(&dir)];
    let forced = plan(
        dir.to_str().unwrap(),
        &candidates,
        d.path(),
        &|_| None,
        &request(dir.to_str().unwrap(), false, true),
        false,
    )
    .unwrap();
    assert_eq!(forced.class, DeleteClass::OrdinaryDirectory);
    assert_eq!(forced.strategy, DeleteStrategy::Trash);

    let via_config = plan(
        dir.to_str().unwrap(),
        &candidates,
        d.path(),
        &|_| None,
        &request(dir.to_str().unwrap(), false, false),
        true,
    )
    .unwrap();
    assert_eq!(via_config.strategy, DeleteStrategy::Permanent);

    let via_flag = plan(
        dir.to_str().unwrap(),
        &candidates,
        d.path(),
        &|_| None,
        &request(dir.to_str().unwrap(), true, true),
        false,
    )
    .unwrap();
    assert_eq!(via_flag.strategy, DeleteStrategy::Permanent);
}

#[test]
fn relative_path_matches_candidate_from_injected_cwd() {
    let d = TempDir::new();
    let dir = d.child("cwd/repo");
    let cwd = d.child("cwd");
    let candidates = [cand(&dir)];
    let target = plan(
        "repo",
        &candidates,
        &cwd,
        &|_| None,
        &request("repo", false, false),
        false,
    )
    .unwrap();
    assert_eq!(target.path, dir.display().to_string());
    assert_eq!(target.class, DeleteClass::OrdinaryDirectory);
}

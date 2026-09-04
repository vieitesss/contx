use super::{
    Config, ConfigError, discover_git_repos, git_repos_from_home,
    load_config, merge_paths, set_config_file,
};
use crate::utils::test_utils::{TempDir, with_home, without_home};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn discovery_disabled_when_key_omitted_or_false() {
    assert_eq!(Config::default().git_from_home, None);

    let d = TempDir::new();
    let home = d.child("home");
    let repo = d.child("home/repo");
    fs::create_dir(repo.join(".git")).unwrap();

    for content in ["paths = []\n", "git-from-home = false\n"] {
        let cfg = d.file("config.toml", content);
        with_home(&home, || {
            let c = load_config(cfg.to_str().unwrap(), true).unwrap();
            assert_eq!(c.paths, Some(vec![]));
        });
    }
}

#[test]
fn discovers_git_dirs_and_worktree_files() {
    let home = TempDir::new();
    let repo = home.child("repo");
    fs::create_dir(repo.join(".git")).unwrap();
    let worktree = home.child("worktree");
    fs::write(worktree.join(".git"), "gitdir: /elsewhere\n").unwrap();
    home.child("plain");
    let nested = home.child("repo/nested");
    fs::create_dir(nested.join(".git")).unwrap();

    let repos = discover_git_repos(home.path()).unwrap();

    assert_eq!(
        repos,
        vec![
            format!("{}/repo", home.path().display()),
            format!("{}/worktree", home.path().display()),
        ]
    );
}

#[test]
fn skips_symlinked_children() {
    let home = TempDir::new();
    let repo = home.child("repo");
    fs::create_dir(repo.join(".git")).unwrap();
    symlink(&repo, home.path().join("link")).unwrap();

    let repos = discover_git_repos(home.path()).unwrap();

    assert_eq!(repos, vec![format!("{}/repo", home.path().display())]);
}

#[test]
fn skips_unreadable_children() {
    let home = TempDir::new();
    let repo = home.child("repo");
    fs::create_dir(repo.join(".git")).unwrap();
    fs::set_permissions(&repo, fs::Permissions::from_mode(0o000)).unwrap();

    let repos = discover_git_repos(home.path()).unwrap();

    fs::set_permissions(&repo, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(repos.is_empty());
}

#[test]
fn missing_home_errors_when_discovery_enabled() {
    without_home(|| {
        let res = git_repos_from_home();
        assert!(matches!(res, Err(ConfigError::HomeIsNotSet)));
    });
}

#[test]
fn disabled_discovery_ignores_missing_home() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "git-from-home = false\n");

    without_home(|| {
        let c = load_config(cfg.to_str().unwrap(), true).unwrap();
        assert_eq!(c.paths, Some(vec![]));
    });
}

#[test]
fn configured_paths_come_first_then_discovered_sorted() {
    let d = TempDir::new();
    let parent = d.child("parent");
    let child = d.child("parent/child");
    let home = d.child("home");
    let zeta = d.child("home/zeta");
    fs::create_dir(zeta.join(".git")).unwrap();
    let alpha = d.child("home/alpha");
    fs::create_dir(alpha.join(".git")).unwrap();
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}\"]\ngit-from-home = true\n", parent.display()),
    );

    with_home(&home, || {
        let c = load_config(cfg.to_str().unwrap(), true).unwrap();
        assert_eq!(
            c.paths,
            Some(vec![
                format!("{}", child.display()),
                format!("{}/alpha", home.display()),
                format!("{}/zeta", home.display()),
            ])
        );
    });
}

#[test]
fn merge_dedups_by_canonical_identity_keeping_first_spelling() {
    let d = TempDir::new();
    let real = d.child("real");
    let link = d.path().join("link");
    symlink(&real, &link).unwrap();

    let merged = merge_paths(
        vec![format!("{}", real.display())],
        vec![
            format!("{}", link.display()),
            format!("{}/./real", d.path().display()),
        ],
    );

    assert_eq!(merged, vec![format!("{}", real.display())]);
}

#[test]
fn merge_keeps_uncanonicalizable_paths_and_dedups_textually() {
    let missing = String::from("/definitely/not/a/real/path");

    let merged = merge_paths(vec![missing.clone(), missing.clone()], vec![]);

    assert_eq!(merged, vec![missing]);
}

#[test]
fn missing_implicit_config_file_runs_with_defaults() {
    let d = TempDir::new();

    with_home(d.path(), || {
        let c = load_config("~/.config/contx/config.toml", false).unwrap();
        assert_eq!(c.paths, None);
        assert_eq!(c.git_from_home, None);
        assert_eq!(
            c.config_file,
            Some(String::from("~/.config/contx/config.toml"))
        );
    });
}

#[test]
fn invalid_implicit_config_file_errors() {
    let d = TempDir::new();
    d.file(".config/contx/config.toml", "= invalid\n");

    with_home(d.path(), || {
        let res = load_config("~/.config/contx/config.toml", false);
        assert!(matches!(res, Err(ConfigError::IncorrectStructure(_))));
    });
}

#[test]
fn explicit_missing_or_invalid_config_file_errors() {
    let d = TempDir::new();
    let missing = d.path().join("missing.toml");

    let mut config = Config::default();
    let res = set_config_file(&mut config, missing.to_str().unwrap());
    assert!(matches!(res, Err(ConfigError::PathIsNotValid(_))));

    let res = load_config(missing.to_str().unwrap(), true);
    assert!(matches!(res, Err(ConfigError::IoError(_, _))));

    let bad = d.file("bad.toml", "= invalid\n");
    let res = load_config(bad.to_str().unwrap(), true);
    assert!(matches!(res, Err(ConfigError::IncorrectStructure(_))));
}

#[test]
fn config_file_arg_expands_tilde_and_env_vars() {
    let d = TempDir::new();
    d.file("config.toml", "paths = []\n");
    let expected = Some(format!("{}/config.toml", d.path().display()));

    with_home(d.path(), || {
        let mut config = Config::default();
        set_config_file(&mut config, "~/config.toml").unwrap();
        assert_eq!(config.config_file, expected);

        // SAFETY: serialized by ENV_LOCK via with_home; test-only.
        unsafe { std::env::set_var("CONTX_TEST_CONFIG_DIR", d.path()) };
        let mut config = Config::default();
        set_config_file(&mut config, "$CONTX_TEST_CONFIG_DIR/config.toml")
            .unwrap();
        assert_eq!(config.config_file, expected);
        unsafe { std::env::remove_var("CONTX_TEST_CONFIG_DIR") };

        let mut config = Config::default();
        let res = set_config_file(&mut config, "~/missing.toml");
        assert!(matches!(res, Err(ConfigError::PathIsNotValid(_))));
    });
}

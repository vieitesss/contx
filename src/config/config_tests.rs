use super::{
    ConfigError, Multiplexer, SessionCandidate, Startup, merge_paths,
    resolve_with,
};
use crate::utils::test_utils::TempDir;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

/// Drive the grouped resolution seam with explicit arguments and
/// environment, without touching process-global state.
fn resolve_startup(
    args: &[&str],
    vars: &[(&str, String)],
) -> super::Result<Startup> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    resolve_with(&args, &|name| {
        vars.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| OsString::from(v))
    })
}

fn resolve_ready(
    args: &[&str],
    vars: &[(&str, String)],
) -> super::Result<super::ResolvedConfig> {
    match resolve_startup(args, vars)? {
        Startup::Ready(resolved) => Ok(resolved),
        Startup::Help => panic!("expected ready startup, got help"),
    }
}

fn resolve_grouped(
    args: &[&str],
    vars: &[(&str, String)],
) -> super::Result<Vec<SessionCandidate>> {
    Ok(resolve_ready(args, vars)?.candidates)
}

/// Drive the resolution seam with explicit arguments and environment,
/// without touching process-global state. Maps grouped candidates to
/// plain paths; group keys are covered by the grouping tests below.
fn resolve(
    args: &[&str],
    vars: &[(&str, String)],
) -> super::Result<Vec<String>> {
    Ok(SessionCandidate::paths(&resolve_grouped(args, vars)?))
}

fn home_env(home: &Path) -> [(&str, String); 1] {
    [("HOME", home.display().to_string())]
}

fn no_candidates() -> Vec<String> {
    vec![]
}

#[test]
fn missing_implicit_config_file_starts_with_no_candidates() {
    let d = TempDir::new();

    let candidates = resolve(&[], &home_env(d.path())).unwrap();

    assert_eq!(candidates, no_candidates());
}

#[test]
fn implicit_config_file_is_loaded_when_present() {
    let d = TempDir::new();
    d.child("opt/project");
    d.file(
        ".config/contx/config.toml",
        &format!("paths = [\"{}\"]\n", d.child("opt").display()),
    );

    let candidates = resolve(&[], &home_env(d.path())).unwrap();

    assert_eq!(
        candidates,
        vec![format!("{}", d.child("opt/project").display())]
    );
}

#[test]
fn malformed_implicit_config_file_errors() {
    let d = TempDir::new();
    d.file(".config/contx/config.toml", "= invalid\n");

    let res = resolve(&[], &home_env(d.path()));

    assert!(matches!(res, Err(ConfigError::IncorrectStructure(_))));
}

#[test]
fn explicit_config_file_short_and_long_flags() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "paths = []\n");

    for flag in ["-c", "--config-file"] {
        let candidates = resolve(&[flag, cfg.to_str().unwrap()], &[]).unwrap();
        assert_eq!(candidates, no_candidates());
    }
}

#[test]
fn explicit_missing_config_file_errors() {
    let d = TempDir::new();
    let missing = d.path().join("missing.toml");

    let res = resolve(&["-c", missing.to_str().unwrap()], &[]);

    assert!(matches!(res, Err(ConfigError::PathIsNotValid(_))));
}

#[test]
fn explicit_malformed_config_file_errors() {
    let d = TempDir::new();
    let bad = d.file("bad.toml", "= invalid\n");

    let res = resolve(&["-c", bad.to_str().unwrap()], &[]);

    assert!(matches!(res, Err(ConfigError::IncorrectStructure(_))));
}

#[test]
fn config_file_arg_expands_tilde_and_env_vars() {
    let d = TempDir::new();
    d.file("config.toml", "paths = []\n");

    let candidates = resolve(&["-c", "~/config.toml"], &home_env(d.path()));
    assert_eq!(candidates.unwrap(), no_candidates());

    let vars = [
        ("HOME", d.path().display().to_string()),
        ("CONTX_CFG_DIR", d.path().display().to_string()),
    ];
    let candidates = resolve(&["-c", "$CONTX_CFG_DIR/config.toml"], &vars);
    assert_eq!(candidates.unwrap(), no_candidates());

    let res = resolve(&["-c", "~/missing.toml"], &home_env(d.path()));
    assert!(matches!(res, Err(ConfigError::PathIsNotValid(_))));
}

#[test]
fn invalid_arguments_error() {
    let res = resolve(&["--bogus"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgIsNotValid(_))));

    let res = resolve(&["-c"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgNotFound)));

    let res = resolve(&["--config-file"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgNotFound)));

    let res = resolve(&["--multiplexer"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgNotFound)));
}

#[test]
fn configured_directory_expands_to_immediate_children() {
    let d = TempDir::new();
    let parent = d.child("parent");
    d.child("parent/alpha");
    d.child("parent/beta");
    d.child("parent/alpha/nested");
    d.file("parent/plain.txt", "x");
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}\"]\n", parent.display()),
    );

    let mut candidates = resolve(&["-c", cfg.to_str().unwrap()], &[]).unwrap();

    candidates.sort();
    assert_eq!(
        candidates,
        vec![
            format!("{}/alpha", parent.display()),
            format!("{}/beta", parent.display()),
        ]
    );
}

#[test]
fn configured_wildcard_keeps_its_depth() {
    let d = TempDir::new();
    let parent = d.child("parent");
    d.child("parent/a/x");
    d.child("parent/a/y");
    d.child("parent/b/z");
    d.child("parent/a/x/too-deep");
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}/*\"]\n", parent.display()),
    );

    let mut candidates = resolve(&["-c", cfg.to_str().unwrap()], &[]).unwrap();

    candidates.sort();
    assert_eq!(
        candidates,
        vec![
            format!("{}/a/x", parent.display()),
            format!("{}/a/y", parent.display()),
            format!("{}/b/z", parent.display()),
        ]
    );
}

#[test]
fn wildcard_on_missing_directory_errors() {
    let d = TempDir::new();
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}/nope/*\"]\n", d.path().display()),
    );

    let res = resolve(&["-c", cfg.to_str().unwrap()], &[]);

    assert!(matches!(res, Err(ConfigError::PathIsNotDirectory(_))));
}

#[test]
fn invalid_configured_paths_error() {
    let d = TempDir::new();

    let cfg = d.file("relative.toml", "paths = [\"relative/dir\"]\n");
    let res = resolve(&["-c", cfg.to_str().unwrap()], &[]);
    assert!(matches!(res, Err(ConfigError::PathIsNotAbsolute(_))));

    let plain = d.file("plain.txt", "x");
    let cfg =
        d.file("file.toml", &format!("paths = [\"{}\"]\n", plain.display()));
    let res = resolve(&["-c", cfg.to_str().unwrap()], &[]);
    assert!(matches!(res, Err(ConfigError::PathIsNotValid(_))));

    let cfg = d.file("env.toml", "paths = [\"$CONTX_UNSET_VAR/dir\"]\n");
    let res = resolve(&["-c", cfg.to_str().unwrap()], &[]);
    assert!(matches!(res, Err(ConfigError::PathHasInvalidEnv(_, _, _))));
}

#[test]
fn configured_paths_expand_tilde_and_env_vars() {
    let d = TempDir::new();
    d.child("home/work/project");
    let home = d.child("home");
    let expected = vec![format!("{}", d.child("home/work/project").display())];

    let cfg = d.file("tilde.toml", "paths = [\"~/work\"]\n");
    let candidates = resolve(&["-c", cfg.to_str().unwrap()], &home_env(&home));
    assert_eq!(candidates.unwrap(), expected);

    let cfg = d.file("env.toml", "paths = [\"$WORKROOT\"]\n");
    let vars = [
        ("HOME", home.display().to_string()),
        ("WORKROOT", format!("{}", d.child("home/work").display())),
    ];
    let candidates = resolve(&["-c", cfg.to_str().unwrap()], &vars);
    assert_eq!(candidates.unwrap(), expected);
}

#[test]
fn unreadable_configured_directory_errors_instead_of_panicking() {
    let d = TempDir::new();
    let parent = d.child("parent");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o000)).unwrap();
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}\"]\n", parent.display()),
    );

    let res = resolve(&["-c", cfg.to_str().unwrap()], &[]);

    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(res, Err(ConfigError::IoError(_, _))));
}

#[test]
fn unreadable_wildcard_child_errors_instead_of_panicking() {
    let d = TempDir::new();
    let parent = d.child("parent");
    let child = d.child("parent/child");
    fs::set_permissions(&child, fs::Permissions::from_mode(0o000)).unwrap();
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}/*\"]\n", parent.display()),
    );

    let res = resolve(&["-c", cfg.to_str().unwrap()], &[]);

    fs::set_permissions(&child, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(res, Err(ConfigError::IoError(_, _))));
}

#[test]
fn discovery_finds_git_dirs_and_worktree_files_at_home_top_level() {
    let d = TempDir::new();
    let home = d.child("home");
    let repo = d.child("home/repo");
    fs::create_dir(repo.join(".git")).unwrap();
    let worktree = d.child("home/worktree");
    fs::write(worktree.join(".git"), "gitdir: /elsewhere\n").unwrap();
    d.child("home/plain");
    let nested = d.child("home/repo/nested");
    fs::create_dir(nested.join(".git")).unwrap();
    let cfg = d.file("config.toml", "git-from-home = true\n");

    let candidates =
        resolve(&["-c", cfg.to_str().unwrap()], &home_env(&home)).unwrap();

    assert_eq!(
        candidates,
        vec![
            format!("{}/repo", home.display()),
            format!("{}/worktree", home.display()),
        ]
    );
}

#[test]
fn discovery_is_off_unless_enabled() {
    let d = TempDir::new();
    let home = d.child("home");
    let repo = d.child("home/repo");
    fs::create_dir(repo.join(".git")).unwrap();

    for content in ["paths = []\n", "git-from-home = false\n"] {
        let cfg = d.file("config.toml", content);
        let candidates =
            resolve(&["-c", cfg.to_str().unwrap()], &home_env(&home)).unwrap();
        assert_eq!(candidates, no_candidates());
    }
}

#[test]
fn discovery_skips_symlinked_children() {
    let d = TempDir::new();
    let home = d.child("home");
    let repo = d.child("home/repo");
    fs::create_dir(repo.join(".git")).unwrap();
    symlink(&repo, home.join("link")).unwrap();
    let cfg = d.file("config.toml", "git-from-home = true\n");

    let candidates =
        resolve(&["-c", cfg.to_str().unwrap()], &home_env(&home)).unwrap();

    assert_eq!(candidates, vec![format!("{}/repo", home.display())]);
}

#[test]
fn discovery_skips_unreadable_children() {
    let d = TempDir::new();
    let home = d.child("home");
    let repo = d.child("home/repo");
    fs::create_dir(repo.join(".git")).unwrap();
    fs::set_permissions(&repo, fs::Permissions::from_mode(0o000)).unwrap();
    let cfg = d.file("config.toml", "git-from-home = true\n");

    let candidates =
        resolve(&["-c", cfg.to_str().unwrap()], &home_env(&home)).unwrap();

    fs::set_permissions(&repo, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(candidates, no_candidates());
}

#[test]
fn missing_home_errors_only_when_discovery_is_enabled() {
    let d = TempDir::new();

    let enabled = d.file("enabled.toml", "git-from-home = true\n");
    let res = resolve(&["-c", enabled.to_str().unwrap()], &[]);
    assert!(matches!(res, Err(ConfigError::HomeIsNotSet)));

    let empty_home = [("HOME", String::new())];
    let res = resolve(&["-c", enabled.to_str().unwrap()], &empty_home);
    assert!(matches!(res, Err(ConfigError::HomeIsNotSet)));

    for content in ["paths = []\n", "git-from-home = false\n"] {
        let cfg = d.file("disabled.toml", content);
        let candidates = resolve(&["-c", cfg.to_str().unwrap()], &[]).unwrap();
        assert_eq!(candidates, no_candidates());
    }
}

#[test]
fn unreadable_home_errors_when_discovery_is_enabled() {
    let d = TempDir::new();
    let home = d.child("home");
    fs::set_permissions(&home, fs::Permissions::from_mode(0o000)).unwrap();
    let cfg = d.file("config.toml", "git-from-home = true\n");

    let res = resolve(&["-c", cfg.to_str().unwrap()], &home_env(&home));

    fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(res, Err(ConfigError::IoError(_, _))));
}

#[test]
fn configured_candidates_precede_sorted_discovered_ones() {
    let d = TempDir::new();
    d.child("parent/child");
    let home = d.child("home");
    let zeta = d.child("home/zeta");
    fs::create_dir(zeta.join(".git")).unwrap();
    let alpha = d.child("home/alpha");
    fs::create_dir(alpha.join(".git")).unwrap();
    let cfg = d.file(
        "config.toml",
        &format!(
            "paths = [\"{}\"]\ngit-from-home = true\n",
            d.child("parent").display()
        ),
    );

    let candidates =
        resolve(&["-c", cfg.to_str().unwrap()], &home_env(&home)).unwrap();

    assert_eq!(
        candidates,
        vec![
            format!("{}", d.child("parent/child").display()),
            format!("{}/alpha", home.display()),
            format!("{}/zeta", home.display()),
        ]
    );
}

#[test]
fn canonical_duplicates_keep_the_first_spelling() {
    let d = TempDir::new();
    d.child("parent/real");
    let alias = d.path().join("alias");
    symlink(d.child("parent"), &alias).unwrap();
    let cfg = d.file(
        "config.toml",
        &format!(
            "paths = [\"{}\", \"{}\"]\n",
            d.child("parent").display(),
            alias.display()
        ),
    );

    let candidates = resolve(&["-c", cfg.to_str().unwrap()], &[]).unwrap();

    assert_eq!(
        candidates,
        vec![format!("{}", d.child("parent/real").display())]
    );
}

#[test]
fn uncanonicalizable_duplicates_dedup_textually() {
    let missing = String::from("/definitely/not/a/real/path");
    let candidate =
        || SessionCandidate::new(missing.clone(), "group".to_string());

    let merged = merge_paths(vec![candidate(), candidate()], vec![]);

    assert_eq!(merged, vec![candidate()]);
}

#[test]
fn configured_directory_groups_children_under_parent() {
    let d = TempDir::new();
    let parent = d.child("parent");
    d.child("parent/alpha");
    d.child("parent/beta");
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}\"]\n", parent.display()),
    );

    let candidates =
        resolve_grouped(&["-c", cfg.to_str().unwrap()], &[]).unwrap();

    assert_eq!(candidates.len(), 2);
    for candidate in &candidates {
        assert_eq!(candidate.group, parent.display().to_string());
        assert!(!candidate.from_home_discovery);
    }
    let mut paths = SessionCandidate::paths(&candidates);
    paths.sort();
    assert_eq!(
        paths,
        vec![
            format!("{}/alpha", parent.display()),
            format!("{}/beta", parent.display()),
        ]
    );
}

#[test]
fn wildcard_groups_grandchildren_under_intermediate_parent() {
    let d = TempDir::new();
    let parent = d.child("parent");
    d.child("parent/a/x");
    d.child("parent/a/y");
    d.child("parent/b/z");
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}/*\"]\n", parent.display()),
    );

    let candidates =
        resolve_grouped(&["-c", cfg.to_str().unwrap()], &[]).unwrap();

    let group_of = |path: &str| {
        candidates
            .iter()
            .find(|c| c.path == path)
            .unwrap()
            .group
            .clone()
    };
    assert_eq!(
        group_of(&format!("{}/a/x", parent.display())),
        format!("{}/a", parent.display())
    );
    assert_eq!(
        group_of(&format!("{}/a/y", parent.display())),
        format!("{}/a", parent.display())
    );
    assert_eq!(
        group_of(&format!("{}/b/z", parent.display())),
        format!("{}/b", parent.display())
    );
}

#[test]
fn discovery_groups_repos_under_home() {
    let d = TempDir::new();
    let home = d.child("home");
    let repo = d.child("home/repo");
    fs::create_dir(repo.join(".git")).unwrap();
    let cfg = d.file("config.toml", "git-from-home = true\n");

    let candidates =
        resolve_grouped(&["-c", cfg.to_str().unwrap()], &home_env(&home))
            .unwrap();

    let mut expected = SessionCandidate::new(
        format!("{}/repo", home.display()),
        home.display().to_string(),
    );
    expected.from_home_discovery = true;
    assert_eq!(candidates, vec![expected]);
}

#[test]
fn merge_keeps_configured_discovery_flag() {
    let path = "/home/me/repo".to_string();
    let group = "/home/me".to_string();
    let configured = SessionCandidate::new(path.clone(), group.clone());
    let mut discovered = SessionCandidate::new(path, group);
    discovered.from_home_discovery = true;
    let merged = merge_paths(vec![configured.clone()], vec![discovered]);
    assert_eq!(merged, vec![configured]);
    assert!(!merged[0].from_home_discovery);
}

#[test]
fn canonical_duplicates_keep_the_first_group() {
    let d = TempDir::new();
    d.child("parent/real");
    let alias = d.path().join("alias");
    symlink(d.child("parent"), &alias).unwrap();
    let cfg = d.file(
        "config.toml",
        &format!(
            "paths = [\"{}\", \"{}\"]\n",
            d.child("parent").display(),
            alias.display()
        ),
    );

    let candidates =
        resolve_grouped(&["-c", cfg.to_str().unwrap()], &[]).unwrap();

    assert_eq!(
        candidates,
        vec![SessionCandidate::new(
            format!("{}", d.child("parent/real").display()),
            d.child("parent").display().to_string(),
        )]
    );
}

#[test]
fn missing_multiplexer_key_defaults_to_auto() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "paths = []\n");

    let resolved = resolve_ready(&["-c", cfg.to_str().unwrap()], &[]).unwrap();

    assert_eq!(resolved.multiplexer, Multiplexer::Auto);
    assert!(resolved.candidates.is_empty());
}

#[test]
fn implicit_missing_config_defaults_multiplexer_to_auto() {
    let d = TempDir::new();

    let resolved = resolve_ready(&[], &home_env(d.path())).unwrap();

    assert_eq!(resolved.multiplexer, Multiplexer::Auto);
    assert!(resolved.candidates.is_empty());
}

#[test]
fn config_file_multiplexer_values() {
    let d = TempDir::new();
    for (value, expected) in [
        ("auto", Multiplexer::Auto),
        ("tmux", Multiplexer::Tmux),
        ("herdr", Multiplexer::Herdr),
    ] {
        let cfg = d.file(
            "config.toml",
            &format!("paths = []\nmultiplexer = \"{value}\"\n"),
        );
        let resolved =
            resolve_ready(&["-c", cfg.to_str().unwrap()], &[]).unwrap();
        assert_eq!(resolved.multiplexer, expected);
    }
}

#[test]
fn unknown_config_file_multiplexer_errors() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "multiplexer = \"nope\"\n");

    let res = resolve(&["-c", cfg.to_str().unwrap()], &[]);

    assert!(matches!(res, Err(ConfigError::IncorrectStructure(_))));
}

#[test]
fn cli_multiplexer_wins_over_config_file() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "paths = []\nmultiplexer = \"tmux\"\n");

    let resolved = resolve_ready(
        &["-c", cfg.to_str().unwrap(), "--multiplexer", "herdr"],
        &[],
    )
    .unwrap();

    assert_eq!(resolved.multiplexer, Multiplexer::Herdr);
}

#[test]
fn cli_multiplexer_without_config_file() {
    let d = TempDir::new();

    let resolved =
        resolve_ready(&["--multiplexer", "tmux"], &home_env(d.path())).unwrap();

    assert_eq!(resolved.multiplexer, Multiplexer::Tmux);
}

#[test]
fn unknown_cli_multiplexer_errors() {
    let res = resolve(&["--multiplexer", "nope"], &[]);

    assert!(matches!(res, Err(ConfigError::ArgIsNotValid(_))));
}

#[test]
fn help_flags_are_not_errors() {
    for flag in ["-h", "--help"] {
        assert_eq!(resolve_startup(&[flag], &[]).unwrap(), Startup::Help);
    }
    assert_eq!(
        resolve_startup(&["--multiplexer", "tmux", "-h"], &[]).unwrap(),
        Startup::Help
    );
}

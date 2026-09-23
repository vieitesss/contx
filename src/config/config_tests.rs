use super::{
    Command, ConfigError, Multiplexer, SessionCandidate, Startup,
    append_parent_to_paths, destination_covered, merge_paths, resolve_with,
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
fn clone_settings_default_to_ssh_and_known_github_prefixes() {
    let home = TempDir::new();
    let resolved = resolve_ready(&[], &home_env(home.path())).unwrap();
    assert_eq!(resolved.clone.default_protocol, super::CloneProtocol::Ssh);
    assert_eq!(resolved.clone.ssh_prefix, "git@github.com:");
    assert_eq!(resolved.clone.https_prefix, "https://github.com");
}

#[test]
fn clone_settings_accept_partial_toml_overrides_and_reject_unknown_protocol() {
    let home = TempDir::new();
    let file = home.file("clone.toml", "[clone]\ndefault-protocol = 'https'\nhttps-prefix = 'https://git.example/teams'\n");
    let resolved = resolve_ready(&["-c", file.to_str().unwrap()], &[]).unwrap();
    assert_eq!(resolved.clone.default_protocol, super::CloneProtocol::Https);
    assert_eq!(resolved.clone.https_prefix, "https://git.example/teams");
    assert_eq!(resolved.clone.ssh_prefix, "git@github.com:");

    let invalid =
        home.file("invalid.toml", "[clone]\ndefault-protocol = 'ftp'\n");
    assert!(matches!(
        resolve_ready(&["-c", invalid.to_str().unwrap()], &[]),
        Err(ConfigError::IncorrectStructure(_))
    ));
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
    assert_eq!(
        resolve_startup(&["clone", "-h", "src", "dest"], &[]).unwrap(),
        Startup::Help
    );
    assert_eq!(
        resolve_startup(&["delete", "--help", "path"], &[]).unwrap(),
        Startup::Help
    );
}

#[test]
fn usage_documents_clone_and_delete() {
    assert!(super::USAGE.contains("clone <source> [destination]"));
    assert!(
        super::USAGE
            .contains("delete [--dry-run] [--permanent] [--force] <path>")
    );
    let usage = super::USAGE.to_lowercase();
    assert!(!usage.contains("remove"));
    assert!(!usage.contains("removal"));
}

#[test]
fn picker_is_the_default_command() {
    let d = TempDir::new();
    let resolved = resolve_ready(&[], &home_env(d.path())).unwrap();
    assert_eq!(resolved.command, Command::Picker);
}

#[test]
fn clone_requires_source() {
    let res = resolve_startup(&["clone"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgNotFound)));
}

#[test]
fn clone_omitted_destination_uses_derived_name() {
    let d = TempDir::new();
    let cases = [
        ("git@host:acme/repo.git", "repo"),
        ("/tmp/x/repo/.git", "repo"),
        ("  https://host/acme/repo.git\t", "repo"),
        ("git@host:acme/.git", "acme"),
    ];
    for (source, dest) in cases {
        let resolved =
            resolve_ready(&["clone", source], &home_env(d.path())).unwrap();
        assert_eq!(
            resolved.command,
            Command::Clone {
                source: source.to_string(),
                destination: dest.to_string(),
            },
            "source {source:?}"
        );
    }
}

#[test]
fn clone_undervable_source_without_destination_is_invalid() {
    let res = resolve_startup(&["clone", ".."], &[]);
    assert!(matches!(res, Err(ConfigError::ArgIsNotValid(_))));
}

#[test]
fn clone_parses_source_and_destination() {
    let d = TempDir::new();
    let resolved = resolve_ready(
        &["clone", "git@host:src.git", "dest"],
        &home_env(d.path()),
    )
    .unwrap();
    assert_eq!(
        resolved.command,
        Command::Clone {
            source: "git@host:src.git".to_string(),
            destination: "dest".to_string(),
        }
    );
}

#[test]
fn clone_rejects_extra_operands() {
    let res = resolve_startup(&["clone", "a", "b", "c"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgIsNotValid(_))));
}

#[test]
fn clone_keeps_global_flags() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "paths = []\nmultiplexer = \"tmux\"\n");
    let cfg = cfg.to_str().unwrap();

    let before = resolve_ready(
        &["-c", cfg, "--multiplexer", "herdr", "clone", "src", "dest"],
        &[],
    )
    .unwrap();
    assert_eq!(before.multiplexer, Multiplexer::Herdr);
    assert_eq!(
        before.command,
        Command::Clone {
            source: "src".to_string(),
            destination: "dest".to_string(),
        }
    );

    let after = resolve_ready(
        &["clone", "src", "dest", "-c", cfg, "--multiplexer", "herdr"],
        &[],
    )
    .unwrap();
    assert_eq!(after.multiplexer, Multiplexer::Herdr);
    assert_eq!(after.command, before.command);
}

#[test]
fn delete_requires_a_path() {
    let res = resolve_startup(&["delete"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgNotFound)));
    let res = resolve_startup(&["delete", "--permanent"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgNotFound)));
}

#[test]
fn delete_parses_path_and_flags() {
    let d = TempDir::new();
    let resolved = resolve_ready(
        &["delete", "--dry-run", "--permanent", "~/proj"],
        &home_env(d.path()),
    )
    .unwrap();
    assert_eq!(
        resolved.command,
        Command::Delete {
            path: "~/proj".to_string(),
            dry_run: true,
            permanent: true,
            force: false,
        }
    );
}

#[test]
fn delete_flags_may_follow_the_path() {
    let d = TempDir::new();
    let resolved =
        resolve_ready(&["delete", "/tmp/proj", "--force"], &home_env(d.path()))
            .unwrap();
    assert_eq!(
        resolved.command,
        Command::Delete {
            path: "/tmp/proj".to_string(),
            dry_run: false,
            permanent: false,
            force: true,
        }
    );
}

#[test]
fn delete_rejects_force_with_dry_run_in_either_order() {
    for args in [
        ["delete", "--force", "--dry-run", "path"].as_slice(),
        ["delete", "--dry-run", "--force", "path"].as_slice(),
        ["delete", "--force", "--permanent", "--dry-run", "path"].as_slice(),
    ] {
        let res = resolve_startup(args, &[]);
        assert!(matches!(res, Err(ConfigError::ForceWithDryRun)), "{args:?}");
    }
}

#[test]
fn delete_allows_permanent_with_dry_run() {
    let d = TempDir::new();
    let resolved = resolve_ready(
        &["delete", "--permanent", "--dry-run", "path"],
        &home_env(d.path()),
    )
    .unwrap();
    assert_eq!(
        resolved.command,
        Command::Delete {
            path: "path".to_string(),
            dry_run: true,
            permanent: true,
            force: false,
        }
    );
}

#[test]
fn delete_rejects_extra_operands_and_unknown_flags() {
    let res = resolve_startup(&["delete", "a", "b"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgIsNotValid(_))));
    let res = resolve_startup(&["delete", "--bogus", "path"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgIsNotValid(_))));
    let res = resolve_startup(&["--dry-run"], &[]);
    assert!(matches!(res, Err(ConfigError::ArgIsNotValid(_))));
}

#[test]
fn delete_keeps_global_flags() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "paths = []\n");
    let cfg = cfg.to_str().unwrap();
    let resolved =
        resolve_ready(&["-c", cfg, "delete", "--force", "path"], &[]).unwrap();
    assert_eq!(resolved.multiplexer, Multiplexer::Auto);
    assert_eq!(
        resolved.command,
        Command::Delete {
            path: "path".to_string(),
            dry_run: false,
            permanent: false,
            force: true,
        }
    );
}

fn covered(
    dest: &str,
    paths: &[&str],
    git_from_home: bool,
    home: &Path,
) -> bool {
    let paths: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
    let home = home.display().to_string();
    destination_covered(dest, &paths, git_from_home, &|name| {
        (name == "HOME").then(|| OsString::from(&home))
    })
    .unwrap()
}

#[test]
fn permanent_delete_defaults_false_when_omitted_or_missing_file() {
    let d = TempDir::new();
    let missing = resolve_ready(&[], &home_env(d.path())).unwrap();
    assert!(!missing.permanent_delete);
    assert!(!missing.config_existed);
    assert!(missing.paths.is_empty());
    assert!(!missing.git_from_home);
    assert_eq!(
        missing.config_path,
        format!("{}/.config/contx/config.toml", d.path().display())
    );

    let cfg = d.file("config.toml", "paths = []\n");
    let omitted = resolve_ready(&["-c", cfg.to_str().unwrap()], &[]).unwrap();
    assert!(!omitted.permanent_delete);
    assert!(omitted.config_existed);
    assert_eq!(omitted.config_path, cfg.display().to_string());
    assert!(omitted.paths.is_empty());
}

#[test]
fn permanent_delete_kebab_case_loads() {
    let d = TempDir::new();
    let enabled = d.file("on.toml", "permanent-delete = true\n");
    let resolved =
        resolve_ready(&["-c", enabled.to_str().unwrap()], &[]).unwrap();
    assert!(resolved.permanent_delete);
    assert!(resolved.config_existed);

    let disabled = d.file("off.toml", "permanent-delete = false\n");
    let resolved =
        resolve_ready(&["-c", disabled.to_str().unwrap()], &[]).unwrap();
    assert!(!resolved.permanent_delete);
}

#[test]
fn resolve_keeps_raw_paths_and_git_from_home() {
    let d = TempDir::new();
    d.child("home/work/project");
    let home = d.child("home");
    let cfg = d.file(
        "config.toml",
        "paths = [\"~/work\"]\ngit-from-home = true\n",
    );
    let resolved =
        resolve_ready(&["-c", cfg.to_str().unwrap()], &home_env(&home))
            .unwrap();
    assert_eq!(resolved.paths, vec!["~/work".to_string()]);
    assert!(resolved.git_from_home);
}

#[test]
fn directory_path_covers_immediate_child_not_grandchild() {
    let d = TempDir::new();
    let home = d.path();
    assert!(covered("~/path/repo", &["~/path"], false, home));
    assert!(covered("~/path/repo/", &["~/path/"], false, home));
    assert!(!covered("~/path/dir/repo", &["~/path"], false, home));
    assert!(!covered("~/other/repo", &["~/path"], false, home));
}

#[test]
fn wildcard_path_covers_grandchild_not_child() {
    let d = TempDir::new();
    let home = d.path();
    assert!(covered("~/path/dir/repo", &["~/path/*"], false, home));
    assert!(!covered("~/path/repo", &["~/path/*"], false, home));
    assert!(!covered("~/path/dir/sub/repo", &["~/path/*"], false, home));
}

#[test]
fn coverage_does_not_require_destination_to_exist() {
    let d = TempDir::new();
    let dest = d.path().join("missing/repo");
    assert!(!dest.exists());
    assert!(covered(
        dest.to_str().unwrap(),
        &[&d.path().join("missing").display().to_string()],
        false,
        d.path(),
    ));
}

#[test]
fn git_from_home_covers_immediate_home_child() {
    let d = TempDir::new();
    let home = d.path();
    assert!(covered("~/repo", &[], true, home));
    assert!(!covered("~/dir/repo", &[], true, home));
    assert!(!covered("~/repo", &[], false, home));
}

#[test]
fn coverage_accepts_env_var_paths() {
    let d = TempDir::new();
    let work = d.child("work");
    let dest = format!("{}/repo", work.display());
    let paths = ["$WORKROOT".to_string()];
    let work_s = work.display().to_string();
    let home = d.path().display().to_string();
    let ok = destination_covered(&dest, &paths, false, &|name| match name {
        "HOME" => Some(OsString::from(&home)),
        "WORKROOT" => Some(OsString::from(&work_s)),
        _ => None,
    })
    .unwrap();
    assert!(ok);
}

#[test]
fn append_parent_preserves_comments_and_unknown_keys() {
    let d = TempDir::new();
    let cfg = d.file(
        "config.toml",
        "# keep me\npaths = [\"~/work\"]\nextra = 1\n",
    );
    let dest = d.path().join("foo/repo");
    append_parent_to_paths(
        dest.to_str().unwrap(),
        cfg.to_str().unwrap(),
        &|_| None,
    )
    .unwrap();
    let text = fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("# keep me"));
    assert!(text.contains("extra = 1"));
    assert!(text.contains("~/work"));
    let parent = d.path().join("foo");
    assert!(text.contains(&format!("\"{}\"", parent.display())));
    assert!(!text.contains("repo"));
}

#[test]
fn append_parent_creates_missing_config_with_parent_only() {
    let d = TempDir::new();
    let cfg = d.path().join(".config/contx/config.toml");
    let dest = format!("{}/foo/repo", d.path().display());
    append_parent_to_paths(&dest, cfg.to_str().unwrap(), &|_| None).unwrap();
    let text = fs::read_to_string(&cfg).unwrap();
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    let paths = parsed["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(
        paths[0].as_str().unwrap(),
        format!("{}/foo", d.path().display())
    );
    assert_eq!(parsed.as_table().unwrap().len(), 1);
    assert!(!text.contains("repo"));
}

#[test]
fn append_parent_adds_paths_key_when_missing() {
    let d = TempDir::new();
    let cfg = d.file("config.toml", "git-from-home = true\n");
    let dest = format!("{}/foo/repo", d.path().display());
    append_parent_to_paths(&dest, cfg.to_str().unwrap(), &|_| None).unwrap();
    let text = fs::read_to_string(&cfg).unwrap();
    assert!(text.contains("git-from-home = true"));
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(
        parsed["paths"].as_array().unwrap()[0].as_str().unwrap(),
        format!("{}/foo", d.path().display())
    );
}

#[test]
fn resolved_config_covers_and_appends_through_methods() {
    let d = TempDir::new();
    let parent = d.child("work");
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}\"]\n", parent.display()),
    );
    let resolved = resolve_ready(&["-c", cfg.to_str().unwrap()], &[]).unwrap();
    let dest = format!("{}/repo", parent.display());
    assert!(resolved.destination_covered(&dest, &|_| None).unwrap());

    let uncovered = format!("{}/other/repo", d.path().display());
    assert!(!resolved.destination_covered(&uncovered, &|_| None).unwrap());
    resolved
        .append_parent_to_paths(&uncovered, &|_| None)
        .unwrap();
    let text = fs::read_to_string(&cfg).unwrap();
    assert!(text.contains(&format!("\"{}/other\"", d.path().display())));
}

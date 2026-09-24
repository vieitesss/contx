use super::{
    CloneError, CloneOutcome, GitClone, GitCloneStatus, Interact, run_with,
    run_with_options,
};
use crate::{
    config::{Command, Multiplexer, ResolvedConfig},
    utils::test_utils::TempDir,
};
use std::{
    cell::RefCell, ffi::OsString, fs, io, path::Path,
    process::Command as GitCmd, rc::Rc,
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

fn test_config(config_path: &Path, paths: &[&str]) -> ResolvedConfig {
    ResolvedConfig {
        candidates: vec![],
        multiplexer: Multiplexer::Auto,
        command: Command::Picker,
        json: false,
        permanent_delete: false,
        clone: crate::config::CloneSettings::default(),
        config_path: config_path.display().to_string(),
        paths: paths.iter().map(|s| s.to_string()).collect(),
        git_from_home: false,
        config_existed: config_path.exists(),
    }
}

struct ScriptGit {
    calls: Vec<(String, String)>,
    status: GitCloneStatus,
    create_dest: bool,
    events: Option<Rc<RefCell<Vec<&'static str>>>>,
}

impl ScriptGit {
    fn ok() -> Self {
        Self {
            calls: vec![],
            status: GitCloneStatus::Success,
            create_dest: true,
            events: None,
        }
    }

    fn fail() -> Self {
        Self {
            calls: vec![],
            status: GitCloneStatus::Failed { code: Some(1) },
            create_dest: true,
            events: None,
        }
    }
}

impl GitClone for ScriptGit {
    fn clone_repo(
        &mut self,
        source: &str,
        dest: &str,
    ) -> io::Result<GitCloneStatus> {
        if let Some(events) = &self.events {
            events.borrow_mut().push("git");
        }
        self.calls.push((source.to_string(), dest.to_string()));
        if self.create_dest {
            fs::create_dir_all(dest)?;
        }
        Ok(self.status)
    }
}

struct ScriptInteract {
    interactive: bool,
    reply: bool,
    asked: Vec<String>,
    events: Option<Rc<RefCell<Vec<&'static str>>>>,
}

impl ScriptInteract {
    fn noninteractive() -> Self {
        Self {
            interactive: false,
            reply: false,
            asked: vec![],
            events: None,
        }
    }

    fn yes() -> Self {
        Self {
            interactive: true,
            reply: true,
            asked: vec![],
            events: None,
        }
    }

    fn no() -> Self {
        Self {
            interactive: true,
            reply: false,
            asked: vec![],
            events: None,
        }
    }
}

impl Interact for ScriptInteract {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn confirm_add_path(&mut self, parent: &str) -> io::Result<bool> {
        if let Some(events) = &self.events {
            events.borrow_mut().push("ask");
        }
        self.asked.push(parent.to_string());
        Ok(self.reply)
    }
}

fn run_clone(
    config: &ResolvedConfig,
    source: &str,
    dest: &str,
    cwd: &Path,
    vars: &[(&str, String)],
    git: &mut ScriptGit,
    interact: &mut ScriptInteract,
) -> (Result<CloneOutcome, CloneError>, String) {
    let mut err = Vec::new();
    let env = env_from(vars);
    let result =
        run_with(config, source, dest, cwd, &env, git, interact, &mut err);
    (result, String::from_utf8(err).unwrap())
}

fn git_cmd(dir: Option<&Path>, args: &[&str]) -> String {
    let mut cmd = GitCmd::new("git");
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

struct LocalGit;

impl GitClone for LocalGit {
    fn clone_repo(
        &mut self,
        source: &str,
        dest: &str,
    ) -> io::Result<GitCloneStatus> {
        let status = GitCmd::new("git")
            .arg("clone")
            .arg(source)
            .arg(dest)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .status()?;
        if status.success() {
            Ok(GitCloneStatus::Success)
        } else {
            Ok(GitCloneStatus::Failed {
                code: status.code(),
            })
        }
    }
}

#[test]
fn machine_clone_skips_tty_prompt_and_add_parent_is_explicit() {
    let d = TempDir::new();
    let config_file = d.path().join("config.toml");
    let mut config = test_config(&config_file, &[]);
    config.json = true;
    let mut git = ScriptGit::ok();
    let mut interact = ScriptInteract::yes();
    let mut err = Vec::new();
    let dest = d.path().join("first");
    let result = run_with_options(
        &config,
        "source",
        dest.to_str().unwrap(),
        d.path(),
        &env_from(&[]),
        &mut git,
        &mut interact,
        &mut err,
        false,
    )
    .unwrap();
    assert!(!result.config_updated);
    assert!(!result.discoverable);
    assert!(interact.asked.is_empty());
    assert!(!config_file.exists());

    let dest = d.path().join("second");
    let result = run_with_options(
        &config,
        "source",
        dest.to_str().unwrap(),
        d.path(),
        &env_from(&[]),
        &mut git,
        &mut interact,
        &mut err,
        true,
    )
    .unwrap();
    assert!(result.config_updated);
    assert!(result.discoverable);
    assert!(interact.asked.is_empty());
    assert!(
        fs::read_to_string(config_file)
            .unwrap()
            .contains(&d.path().display().to_string())
    );
}

#[test]
fn default_clone_dest_name_matches_git_clone_naming() {
    let cases = [
        // Local paths: a trailing `/.git` names the git directory.
        ("/tmp/x/repo/.git", Some("repo")),
        ("repo/.git", Some("repo")),
        ("https://github.com/acme/repo.git", Some("repo")),
        ("https://github.com/acme/repo/", Some("repo")),
        ("https://github.com/acme/repo.git/", Some("repo")),
        ("https://github.com/acme/repo.git ", Some("repo")),
        ("  https://github.com/acme/repo.git\t", Some("repo")),
        ("git@github.com:acme/repo.git", Some("repo")),
        ("git@github.com:repo", Some("repo")),
        ("/local/path/repo", Some("repo")),
        ("repo.git", Some("repo")),
        // A trailing `/.git` falls back to the parent component.
        ("https://host/acme/.git", Some("acme")),
        ("https://host/.git", Some("host")),
        ("host:acme/.git", Some("acme")),
        // Host-only sources name the host, without userinfo.
        ("https://host/", Some("host")),
        ("host:", Some("host")),
        ("git@host:", Some("host")),
        ("host:repo.git", Some("repo")),
        // `.git` and dot names are never destinations.
        ("host:.git", None),
        (".git", None),
        ("", None),
        (".", None),
        ("..", None),
        // A URL with neither host nor path has no name (git refuses).
        ("https://", None),
        ("host://", None),
    ];
    for (source, expected) in cases {
        assert_eq!(
            super::default_clone_dest_name(source),
            expected,
            "source {source:?}"
        );
    }
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

    let tilde = super::resolve_destination("~/proj", Some(&cwd), &env).unwrap();
    assert_eq!(tilde, format!("{}/proj", home.display()));

    let from_env =
        super::resolve_destination("$WORK/proj", Some(&cwd), &env).unwrap();
    assert_eq!(from_env, format!("{}/proj", d.child("work").display()));

    let relative =
        super::resolve_destination("repo", Some(&cwd), &env).unwrap();
    assert_eq!(relative, format!("{}/repo", cwd.display()));

    let abs = format!("{}/abs", d.path().display());
    let kept = super::resolve_destination(&abs, Some(&cwd), &env).unwrap();
    assert_eq!(kept, abs);
}

#[test]
fn resolve_does_not_default_empty_dest_to_home_or_cwd() {
    let d = TempDir::new();
    let vars = [("HOME", d.path().display().to_string())];
    let env = env_from(&vars);
    let err = super::resolve_destination("", Some(d.path()), &env).unwrap_err();
    assert!(matches!(err, CloneError::NotAbsolute(_)));
}

#[test]
fn picker_relative_joins_group_not_cwd_and_unset_base_rejects_relative() {
    let d = TempDir::new();
    let group = d.child("work/acme");
    let cwd = d.child("cwd");
    let env = env_from(&[]);
    let joined = super::resolve_destination("api", Some(&group), &env).unwrap();
    assert_eq!(joined, format!("{}/api", group.display()));
    assert!(!joined.starts_with(&cwd.display().to_string()));
    let err = super::resolve_destination("api", None, &env).unwrap_err();
    assert!(matches!(err, CloneError::NotAbsolute(_)));
}

#[test]
fn existing_destination_is_refused_without_cloning() {
    let d = TempDir::new();
    let dest = d.child("already");
    let cfg = d.file("config.toml", "paths = []\n");
    let config = test_config(&cfg, &[]);
    let mut git = ScriptGit::ok();
    let mut interact = ScriptInteract::yes();
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    assert!(matches!(result, Err(CloneError::DestExists(_))));
    assert!(git.calls.is_empty());
    assert!(interact.asked.is_empty());
}

#[test]
fn prints_absolute_destination_before_git() {
    let d = TempDir::new();
    let dest = d.path().join("repo");
    let cfg = d.file("config.toml", "paths = []\n");
    let config = test_config(&cfg, &[]);
    let mut git = ScriptGit::ok();
    let mut interact = ScriptInteract::noninteractive();
    let (result, err) = run_clone(
        &config,
        "git@host:src.git",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    result.unwrap();
    assert!(err.contains(&format!("clone destination: {}", dest.display())));
    assert_eq!(
        git.calls,
        vec![("git@host:src.git".to_string(), dest.display().to_string())]
    );
}

#[test]
fn covered_destination_skips_offer_and_config_write() {
    let d = TempDir::new();
    let parent = d.child("work");
    let dest = parent.join("repo");
    let cfg = d.file(
        "config.toml",
        &format!("paths = [\"{}\"]\n", parent.display()),
    );
    let original = fs::read_to_string(&cfg).unwrap();
    let config = test_config(&cfg, &[&parent.display().to_string()]);
    let mut git = ScriptGit::ok();
    let mut interact = ScriptInteract::yes();
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    let outcome = result.unwrap();
    assert!(!outcome.config_updated);
    assert!(interact.asked.is_empty());
    assert_eq!(fs::read_to_string(&cfg).unwrap(), original);
}

#[test]
fn interactive_offer_is_asked_before_clone_and_written_after_success() {
    let d = TempDir::new();
    let dest = d.path().join("foo/repo");
    let cfg = d.file("config.toml", "paths = []\n");
    let config = test_config(&cfg, &[]);
    let events = Rc::new(RefCell::new(vec![]));
    let mut git = ScriptGit::ok();
    git.events = Some(Rc::clone(&events));
    let mut interact = ScriptInteract::yes();
    interact.events = Some(Rc::clone(&events));
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    let outcome = result.unwrap();
    assert!(outcome.config_updated);
    assert_eq!(*events.borrow(), vec!["ask", "git"]);
    assert_eq!(
        interact.asked,
        vec![d.path().join("foo").display().to_string()]
    );
    let text = fs::read_to_string(&cfg).unwrap();
    assert!(text.contains(&format!("\"{}\"", d.path().join("foo").display())));
    assert!(!text.contains("repo"));
}

#[test]
fn declining_the_offer_does_not_cancel_clone_or_write_config() {
    let d = TempDir::new();
    let dest = d.path().join("foo/repo");
    let cfg = d.file("config.toml", "paths = []\n");
    let original = fs::read_to_string(&cfg).unwrap();
    let config = test_config(&cfg, &[]);
    let mut git = ScriptGit::ok();
    let mut interact = ScriptInteract::no();
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    let outcome = result.unwrap();
    assert!(!outcome.config_updated);
    assert_eq!(git.calls.len(), 1);
    assert_eq!(fs::read_to_string(&cfg).unwrap(), original);
}

#[test]
fn noninteractive_clone_never_mutates_config() {
    let d = TempDir::new();
    let dest = d.path().join("foo/repo");
    let cfg = d.file("config.toml", "paths = []\n");
    let original = fs::read_to_string(&cfg).unwrap();
    let config = test_config(&cfg, &[]);
    let mut git = ScriptGit::ok();
    let mut interact = ScriptInteract::noninteractive();
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    let outcome = result.unwrap();
    assert!(!outcome.config_updated);
    assert!(interact.asked.is_empty());
    assert_eq!(git.calls.len(), 1);
    assert_eq!(fs::read_to_string(&cfg).unwrap(), original);
}

#[test]
fn failed_clone_leaves_surviving_dest_and_skips_config() {
    let d = TempDir::new();
    let dest = d.path().join("foo/repo");
    let cfg = d.file("config.toml", "paths = []\n");
    let original = fs::read_to_string(&cfg).unwrap();
    let config = test_config(&cfg, &[]);
    let mut git = ScriptGit::fail();
    let mut interact = ScriptInteract::yes();
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    assert!(matches!(result, Err(CloneError::GitFailed { .. })));
    assert!(dest.exists());
    assert_eq!(fs::read_to_string(&cfg).unwrap(), original);
}

#[test]
fn interrupted_clone_leaves_surviving_dest_and_skips_config() {
    let d = TempDir::new();
    let dest = d.path().join("foo/repo");
    let cfg = d.file("config.toml", "paths = []\n");
    let original = fs::read_to_string(&cfg).unwrap();
    let config = test_config(&cfg, &[]);
    let mut git = ScriptGit::ok();
    git.status = GitCloneStatus::Interrupted;
    let mut interact = ScriptInteract::yes();
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    assert!(matches!(result, Err(CloneError::GitInterrupted { .. })));
    assert!(dest.exists());
    assert_eq!(fs::read_to_string(&cfg).unwrap(), original);
}

#[test]
fn config_write_failure_after_clone_is_partial_and_does_not_roll_back() {
    let d = TempDir::new();
    let dest = d.path().join("foo/repo");
    let cfg_path = d.child("config.toml");
    let config = test_config(&cfg_path, &[]);
    let mut git = ScriptGit::ok();
    let mut interact = ScriptInteract::yes();
    let (result, _) = run_clone(
        &config,
        "src",
        dest.to_str().unwrap(),
        d.path(),
        &[],
        &mut git,
        &mut interact,
    );
    assert!(matches!(result, Err(CloneError::Partial { .. })));
    assert!(dest.exists());
}

#[test]
fn local_git_clone_creates_repo_without_network() {
    let d = TempDir::new();
    let origin = d.child("origin");
    git_cmd(Some(&origin), &["init", "-b", "topic"]);
    fs::write(origin.join("file.txt"), "one\n").unwrap();
    git_cmd(Some(&origin), &["add", "file.txt"]);
    git_cmd(
        Some(&origin),
        &[
            "-c",
            "user.email=contx-test@example.com",
            "-c",
            "user.name=contx-test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "first",
        ],
    );
    let dest = d.path().join("work");
    let cfg = d.file("config.toml", "paths = []\n");
    let config = test_config(&cfg, &[]);
    let mut err = Vec::new();
    let mut interact = ScriptInteract::noninteractive();
    let result = run_with(
        &config,
        origin.to_str().unwrap(),
        dest.to_str().unwrap(),
        d.path(),
        &|_| None,
        &mut LocalGit,
        &mut interact,
        &mut err,
    )
    .unwrap();
    assert!(!result.config_updated);
    assert!(dest.join(".git").exists());
    assert!(dest.join("file.txt").exists());
    let err = String::from_utf8(err).unwrap();
    assert!(err.contains(&dest.display().to_string()));
}

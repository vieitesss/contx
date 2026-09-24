use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::Value;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "contx_cli_json_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn child(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn config(&self, parent: &Path) -> PathBuf {
        let path = self.0.join("config.toml");
        fs::write(&path, format!("paths = [\"{}\"]\n", parent.display()))
            .unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn contx(cwd: &Path, config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_contx"))
        .current_dir(cwd)
        .env_remove("HERDR_ENV")
        .env_remove("TMUX")
        .args(["-c", config.to_str().unwrap(), "--json"])
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "invalid JSON: {e}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn list_works_from_a_read_only_cwd() {
    let d = TempDir::new();
    let projects = d.child("projects");
    let project = d.child("projects/example");
    let config = d.config(&projects);
    let readonly = d.child("readonly");
    fs::set_permissions(&readonly, fs::Permissions::from_mode(0o500)).unwrap();
    let listed = success(contx(&readonly, &config, &["list"]));
    assert_eq!(listed["candidates"][0]["path"], project.to_str().unwrap());
    fs::set_permissions(&readonly, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn machine_clone_list_worktree_and_delete_preflight_round_trip() {
    let d = TempDir::new();
    let projects = d.child("projects");
    let origin = d.child("origin");
    git(&origin, &["init", "-q", "-b", "main"]);
    git(
        &origin,
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
    let cfg = d.config(&projects);
    let clone = projects.join("clone");
    let linked = projects.join("linked");
    let cloned = success(contx(
        &d.0,
        &cfg,
        &["clone", origin.to_str().unwrap(), clone.to_str().unwrap()],
    ));
    assert_eq!(cloned["destination"], clone.to_str().unwrap());
    assert_eq!(cloned["discoverable"], true);

    let listed = success(contx(&d.0, &cfg, &["list"]));
    assert_eq!(listed["candidates"][0]["path"], clone.to_str().unwrap());

    let created = success(contx(
        &d.0,
        &cfg,
        &[
            "worktree",
            "create",
            "--new-branch",
            clone.to_str().unwrap(),
            "feature",
            linked.to_str().unwrap(),
        ],
    ));
    assert_eq!(created["branch"], "feature");
    assert_eq!(created["discoverable"], true);
    assert!(linked.join(".git").is_file());

    let preflight = success(contx(
        &d.0,
        &cfg,
        &["delete", "--dry-run", linked.to_str().unwrap()],
    ));
    assert_eq!(preflight["preflight"]["target"]["class"], "linked_worktree");
    assert_eq!(preflight["preflight"]["target"]["strategy"], "git_worktree");
    assert_eq!(
        preflight["preflight"]["remote_verification"],
        "not_performed"
    );
    assert!(
        preflight["preflight"]["blockers"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let denied = contx(&d.0, &cfg, &["delete", linked.to_str().unwrap()]);
    assert!(!denied.status.success());
    assert!(
        serde_json::from_slice::<Value>(&denied.stderr).unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("--force")
    );
    assert!(linked.exists());

    let deleted = success(contx(
        &d.0,
        &cfg,
        &["delete", "--force", linked.to_str().unwrap()],
    ));
    assert_eq!(deleted["strategy"], "git_worktree");
    assert!(!linked.exists());
}

#[test]
fn machine_open_focuses_only_a_matching_herdr_workspace() {
    let d = TempDir::new();
    let project = d.child("projects/example");
    let config = d.config(project.parent().unwrap());
    let calls = d.0.join("calls.txt");
    let script = d.0.join("herdr-mock.sh");
    fs::write(&script, format!(r#"#!/bin/sh
echo "$*" >> '{}'
if [ "$1" = pane ]; then
    printf '%s\n' '{{"result":{{"type":"pane_list","panes":[{{"workspace_id":"w7","cwd":"{}"}},{{"workspace_id":"w8","cwd":"{}"}}]}}}}'
else
    printf '%s\n' '{{"result":{{"type":"workspace_info","workspace":{{"workspace_id":"w7"}}}}}}'
fi
"#, calls.display(), project.display(), project.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let run = |id: &str| {
        Command::new(env!("CARGO_BIN_EXE_contx"))
            .current_dir(&d.0)
            .env("HERDR_ENV", "1")
            .env("HERDR_SOCKET_PATH", "/mock/socket")
            .env("HERDR_BIN_PATH", &script)
            .env("HOME", &d.0)
            .env("USER", "tester")
            .args([
                "-c",
                config.to_str().unwrap(),
                "--multiplexer",
                "herdr",
                "--json",
                "open",
                "--workspace-id",
                id,
                project.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let wrong = run("w9");
    assert!(!wrong.status.success());
    assert!(
        serde_json::from_slice::<Value>(&wrong.stderr).unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("does not match")
    );
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 1);

    let ambiguous = Command::new(env!("CARGO_BIN_EXE_contx"))
        .current_dir(&d.0)
        .env("HERDR_ENV", "1")
        .env("HERDR_SOCKET_PATH", "/mock/socket")
        .env("HERDR_BIN_PATH", &script)
        .env("HOME", &d.0)
        .env("USER", "tester")
        .args([
            "-c",
            config.to_str().unwrap(),
            "--multiplexer",
            "herdr",
            "--json",
            "open",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!ambiguous.status.success());
    let error: Value = serde_json::from_slice(&ambiguous.stderr).unwrap();
    assert_eq!(error["workspace_ids"], serde_json::json!(["w7", "w8"]));
    assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);

    let opened = success(run("w7"));
    assert_eq!(opened["multiplexer"], "herdr");
    assert_eq!(opened["workspace_id"], "w7");
    assert!(
        fs::read_to_string(calls)
            .unwrap()
            .contains("workspace focus w7")
    );
}

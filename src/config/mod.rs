mod error;

use serde::Deserialize;
use std::{
    collections::HashSet,
    env::{self, VarError},
    ffi::OsString,
    fs, io,
    path::Path,
};

use error::ConfigError;

type Result<T> = std::result::Result<T, ConfigError>;

/// Implicit configuration file location, used when no `--config-file`
/// argument is given.
const DEFAULT_CONFIG_FILE: &str = "~/.config/contx/config.toml";

/// Multiplexer-neutral usage, printed for `--help` / `-h`.
pub const USAGE: &str = "\
contx [options]

  -c, --config-file <path>         configuration file
  --multiplexer auto|tmux|herdr    multiplexer (default: auto)
  -h, --help                       show this help
";

/// Which multiplexer should activate a selected session candidate.
/// `auto` is resolved later; `tmux` and `herdr` are explicit preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Multiplexer {
    #[default]
    Auto,
    Tmux,
    Herdr,
}

impl Multiplexer {
    fn from_arg(value: &str) -> Result<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "tmux" => Ok(Self::Tmux),
            "herdr" => Ok(Self::Herdr),
            _ => Err(ConfigError::ArgIsNotValid(value.to_string())),
        }
    }
}

/// Resolved startup: session candidates plus the effective multiplexer
/// preference (CLI wins over the config file; missing means `auto`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub candidates: Vec<SessionCandidate>,
    pub multiplexer: Multiplexer,
}

/// Process startup after parsing arguments. Help is not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    Help,
    Ready(ResolvedConfig),
}

struct CliArgs {
    config_file: Option<String>,
    multiplexer: Option<Multiplexer>,
}

struct Loaded {
    candidates: Vec<SessionCandidate>,
    multiplexer: Multiplexer,
}

/// One resolved session candidate plus the config group it
/// renders under. The group is the expanded configured
/// directory for `paths` children, the intermediate parent
/// for `dir/*` grandchildren, or the `$HOME` path for
/// git-from-home discoveries. Selection and Git polling use
/// only `path`; grouping is presentation over the matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCandidate {
    pub path: String,
    pub group: String,
    /// Set only by home-repository discovery. Configured
    /// candidates stay false; `merge_paths` keeps the first
    /// spelling, so a configured `$HOME` path wins the flag.
    pub from_home_discovery: bool,
}

impl SessionCandidate {
    pub fn new(path: String, group: String) -> Self {
        Self {
            path,
            group,
            from_home_discovery: false,
        }
    }

    /// Plain paths in order, for selection and Git polling.
    pub fn paths(candidates: &[SessionCandidate]) -> Vec<String> {
        candidates.iter().map(|c| c.path.clone()).collect()
    }
}

/// Raw deserialized configuration. Stays private: callers receive only the
/// fully resolved session-candidate outcome.
#[derive(Deserialize, Debug)]
struct RawConfig {
    paths: Option<Vec<String>>,
    #[serde(rename = "git-from-home")]
    git_from_home: Option<bool>,
    multiplexer: Option<Multiplexer>,
}

/// Resolve startup from process-global argv and environment; the
/// resolution itself lives in `resolve_with`.
pub fn resolve() -> Result<Startup> {
    let args: Vec<String> = env::args().skip(1).collect();
    resolve_with(&args, &|name| env::var_os(name))
}

/// Resolve startup from explicit arguments and an environment lookup, so
/// tests can drive it deterministically with temporary-directory filesystems.
fn resolve_with(
    args: &[String],
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Startup> {
    let Some(cli) = parse_args(args, env)? else {
        return Ok(Startup::Help);
    };
    let loaded = match &cli.config_file {
        Some(config_file) => load_candidates(config_file, true, env)?,
        None => load_candidates(DEFAULT_CONFIG_FILE, false, env)?,
    };
    Ok(Startup::Ready(ResolvedConfig {
        candidates: loaded.candidates,
        multiplexer: cli.multiplexer.unwrap_or(loaded.multiplexer),
    }))
}

/// Parse startup arguments. `--help` / `-h` yields `None` (help). Otherwise
/// returns the config-file path and optional CLI multiplexer override.
fn parse_args(
    args: &[String],
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Option<CliArgs>> {
    let mut config_file = None;
    let mut multiplexer = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--config-file" | "-c" => {
                let value = args.next().ok_or(ConfigError::ArgNotFound)?;
                let expanded = expand(value, env)?;
                if !Path::new(&expanded).exists() {
                    return Err(ConfigError::PathIsNotValid(value.clone()));
                }
                config_file = Some(expanded);
            }
            "--multiplexer" => {
                let value = args.next().ok_or(ConfigError::ArgNotFound)?;
                multiplexer = Some(Multiplexer::from_arg(value)?);
            }
            _ => return Err(ConfigError::ArgIsNotValid(arg.clone())),
        }
    }
    Ok(Some(CliArgs {
        config_file,
        multiplexer,
    }))
}

/// Load the configuration file and resolve its session candidates. A missing
/// implicit file resolves to no candidates; a missing explicit file is an
/// error.
fn load_candidates(
    config_file: &str,
    explicit: bool,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Loaded> {
    let path = expand_raw(config_file, env).map_err(|e| {
        ConfigError::IoError(
            io::Error::new(io::ErrorKind::NotFound, e),
            config_file.to_string(),
        )
    })?;

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound && !explicit {
                return Ok(Loaded {
                    candidates: vec![],
                    multiplexer: Multiplexer::Auto,
                });
            }
            return Err(ConfigError::IoError(e, config_file.to_string()));
        }
    };

    let raw: RawConfig =
        toml::from_str(&content).map_err(ConfigError::IncorrectStructure)?;

    let configured = match raw.paths {
        Some(paths) => normalize_paths(&paths, env)?,
        None => vec![],
    };
    let discovered = if raw.git_from_home.unwrap_or(false) {
        git_repos_from_home(env)?
    } else {
        vec![]
    };

    Ok(Loaded {
        candidates: merge_paths(configured, discovered),
        multiplexer: raw.multiplexer.unwrap_or_default(),
    })
}

/// Expand every configured path into the session candidates it names.
fn normalize_paths(
    paths: &[String],
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Vec<SessionCandidate>> {
    let mut candidates = vec![];
    for path in paths {
        candidates.append(&mut normalize_path(path, env)?);
    }
    Ok(candidates)
}

/// Expand one configured path into session candidates: a directory expands
/// to its child directories grouped under that directory, `dir/*` to the
/// children of its children grouped under the intermediate parent, and
/// anything else is a configuration error.
fn normalize_path(
    path: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Vec<SessionCandidate>> {
    let expanded = expand(path, env)?;

    if !expanded.starts_with('/') {
        return Err(ConfigError::PathIsNotAbsolute(path.to_string()));
    }

    if is_directory(&expanded) {
        return Ok(inner_dirs(&expanded)?
            .into_iter()
            .map(|c| SessionCandidate::new(c, expanded.clone()))
            .collect());
    }

    if expanded.ends_with("/*") {
        let dir = &expanded[..expanded.len() - 2];
        if !is_directory(dir) {
            return Err(ConfigError::PathIsNotDirectory(path.to_string()));
        }
        let mut all = vec![];
        for child in inner_dirs(dir)? {
            for grandchild in inner_dirs(&child)? {
                all.push(SessionCandidate::new(grandchild, child.clone()));
            }
        }
        return Ok(all);
    }

    Err(ConfigError::PathIsNotValid(path.to_string()))
}

/// List the immediate child directories of `dir`, skipping entries that
/// cannot be read. A failure to read `dir` itself is a configuration error.
fn inner_dirs(dir: &str) -> Result<Vec<String>> {
    let entries = Path::new(dir)
        .read_dir()
        .map_err(|e| ConfigError::IoError(e, dir.to_string()))?;
    Ok(entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.path().display().to_string())
        .collect())
}

fn is_directory(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Expand tildes and environment variables, reporting invalid variables
/// against the original spelling.
fn expand(
    path: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<String> {
    expand_raw(path, env).map_err(|e| {
        ConfigError::PathHasInvalidEnv(e.cause, e.var_name, path.to_string())
    })
}

/// Expand tildes and environment variables through the given environment
/// lookup instead of process-global state.
fn expand_raw(
    path: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> std::result::Result<String, shellexpand::LookupError<VarError>> {
    shellexpand::full_with_context(
        path,
        || match env("HOME") {
            Some(home) if !home.is_empty() => home.into_string().ok(),
            _ => None,
        },
        |name| -> std::result::Result<Option<String>, VarError> {
            match env(name) {
                Some(value) => {
                    value.into_string().map(Some).map_err(VarError::NotUnicode)
                }
                None => Err(VarError::NotPresent),
            }
        },
    )
    .map(|cow| cow.into_owned())
}

/// A directory is a Git repository when it contains a `.git` directory or a
/// `.git` file (linked worktree).
fn is_git_repository(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Discover Git repositories among the immediate children of `home`, sorted.
/// Symlinked and unreadable children are skipped. Every discovery groups
/// under the `$HOME` path itself.
fn discover_git_repos(home: &Path) -> Result<Vec<SessionCandidate>> {
    let entries = home
        .read_dir()
        .map_err(|e| ConfigError::IoError(e, home.display().to_string()))?;

    let group = home.display().to_string();
    let mut repos: Vec<SessionCandidate> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| is_git_repository(p))
        .map(|p| SessionCandidate {
            path: p.display().to_string(),
            group: group.clone(),
            from_home_discovery: true,
        })
        .collect();

    repos.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(repos)
}

/// Home repository discovery: inspect only the immediate children of
/// `$HOME`. Called only when discovery is enabled, so a missing `$HOME` is
/// an error here.
fn git_repos_from_home(
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Vec<SessionCandidate>> {
    match env("HOME") {
        Some(home) if !home.is_empty() => discover_git_repos(Path::new(&home)),
        _ => Err(ConfigError::HomeIsNotSet),
    }
}

/// Canonical identity of a path, falling back to its textual spelling when
/// it cannot be canonicalized.
fn identity(path: &str) -> String {
    match Path::new(path).canonicalize() {
        Ok(c) => c.display().to_string(),
        Err(_) => path.to_string(),
    }
}

/// Configured candidates precede discovered ones; later duplicates by
/// canonical identity are dropped, keeping the first spelling, group,
/// and `from_home_discovery` flag.
fn merge_paths(
    configured: Vec<SessionCandidate>,
    discovered: Vec<SessionCandidate>,
) -> Vec<SessionCandidate> {
    let mut seen = HashSet::new();
    configured
        .into_iter()
        .chain(discovered)
        .filter(|c| seen.insert(identity(&c.path)))
        .collect()
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

mod error;

use serde::Deserialize;
use std::{
    collections::HashSet,
    env::{self, VarError},
    ffi::OsString,
    fs, io,
    path::Path,
};
use toml_edit::{Array, DocumentMut, Item, Value};

pub(crate) use error::ConfigError;

type Result<T> = std::result::Result<T, ConfigError>;

/// Implicit configuration file location, used when no `--config-file`
/// argument is given.
const DEFAULT_CONFIG_FILE: &str = "~/.config/contx/config.toml";

/// Multiplexer-neutral usage, printed for `--help` / `-h`.
pub const USAGE: &str = "\
contx [options]
contx [options] clone <source> [destination]
contx [options] delete [--dry-run] [--permanent] [--force] <path>

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

/// Picker clone presets. They affect only the interactive dialog, not CLI clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CloneProtocol {
    #[default]
    Ssh,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CloneSettings {
    pub default_protocol: CloneProtocol,
    pub ssh_prefix: String,
    pub https_prefix: String,
}

impl Default for CloneSettings {
    fn default() -> Self {
        Self {
            default_protocol: CloneProtocol::Ssh,
            ssh_prefix: "git@github.com:".into(),
            https_prefix: "https://github.com".into(),
        }
    }
}

/// What this invocation should do after configuration is loaded.
/// Clone and delete never open the picker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Command {
    #[default]
    Picker,
    Clone {
        source: String,
        destination: String,
    },
    Delete {
        path: String,
        dry_run: bool,
        permanent: bool,
        force: bool,
    },
}

/// Resolved startup: session candidates plus the effective multiplexer
/// preference (CLI wins over the config file; missing means `auto`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub candidates: Vec<SessionCandidate>,
    pub multiplexer: Multiplexer,
    pub command: Command,
    /// Ordinary-directory / standalone-repo / symlink deletion strategy.
    /// Linked worktrees always use Git. Default false (trash).
    pub permanent_delete: bool,
    pub clone: CloneSettings,
    /// Expanded path of the active config file, even when the implicit
    /// file is missing and will be created on a later write.
    pub config_path: String,
    /// Raw `paths` entries as spelled in the file (empty if omitted).
    pub paths: Vec<String>,
    pub git_from_home: bool,
    /// False when the implicit default file was missing at load.
    pub config_existed: bool,
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
    command: Command,
}

/// Subcommand operands collected while walking argv.
enum PendingCommand {
    Picker,
    Clone {
        source: Option<String>,
        destination: Option<String>,
    },
    Delete {
        path: Option<String>,
        dry_run: bool,
        permanent: bool,
        force: bool,
    },
}

struct Loaded {
    candidates: Vec<SessionCandidate>,
    multiplexer: Multiplexer,
    permanent_delete: bool,
    clone: CloneSettings,
    config_path: String,
    paths: Vec<String>,
    git_from_home: bool,
    config_existed: bool,
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
    #[serde(rename = "permanent-delete")]
    permanent_delete: Option<bool>,
    clone: Option<CloneSettings>,
}

/// Resolve startup from process-global argv and environment; the
/// resolution itself lives in `resolve_with`.
pub fn resolve() -> Result<Startup> {
    let args: Vec<String> = env::args().skip(1).collect();
    resolve_with(&args, &|name| env::var_os(name))
}

/// Resolve startup from explicit arguments and an environment lookup, so
/// tests can drive it deterministically with temporary-directory filesystems.
pub(crate) fn resolve_with(
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
        command: cli.command,
        permanent_delete: loaded.permanent_delete,
        clone: loaded.clone,
        config_path: loaded.config_path,
        paths: loaded.paths,
        git_from_home: loaded.git_from_home,
        config_existed: loaded.config_existed,
    }))
}

/// Parse startup arguments. `--help` / `-h` yields `None` (help). Otherwise
/// returns the config-file path, optional CLI multiplexer override, and
/// the command (picker, clone, or delete).
fn parse_args(
    args: &[String],
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Option<CliArgs>> {
    let mut config_file = None;
    let mut multiplexer = None;
    let mut pending = PendingCommand::Picker;
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
            "clone" if matches!(pending, PendingCommand::Picker) => {
                pending = PendingCommand::Clone {
                    source: None,
                    destination: None,
                };
            }
            "delete" if matches!(pending, PendingCommand::Picker) => {
                pending = PendingCommand::Delete {
                    path: None,
                    dry_run: false,
                    permanent: false,
                    force: false,
                };
            }
            "--dry-run" => match &mut pending {
                PendingCommand::Delete { dry_run, .. } => *dry_run = true,
                _ => return Err(ConfigError::ArgIsNotValid(arg.clone())),
            },
            "--permanent" => match &mut pending {
                PendingCommand::Delete { permanent, .. } => {
                    *permanent = true;
                }
                _ => return Err(ConfigError::ArgIsNotValid(arg.clone())),
            },
            "--force" => match &mut pending {
                PendingCommand::Delete { force, .. } => *force = true,
                _ => return Err(ConfigError::ArgIsNotValid(arg.clone())),
            },
            other if other.starts_with('-') => {
                return Err(ConfigError::ArgIsNotValid(arg.clone()));
            }
            other => take_operand(&mut pending, other, arg)?,
        }
    }
    Ok(Some(CliArgs {
        config_file,
        multiplexer,
        command: finish_command(pending)?,
    }))
}

fn take_operand(
    pending: &mut PendingCommand,
    other: &str,
    arg: &str,
) -> Result<()> {
    match pending {
        PendingCommand::Picker => {
            Err(ConfigError::ArgIsNotValid(arg.to_string()))
        }
        PendingCommand::Clone {
            source,
            destination,
        } => {
            if source.is_none() {
                *source = Some(other.to_string());
                Ok(())
            } else if destination.is_none() {
                *destination = Some(other.to_string());
                Ok(())
            } else {
                Err(ConfigError::ArgIsNotValid(arg.to_string()))
            }
        }
        PendingCommand::Delete { path, .. } => {
            if path.is_none() {
                *path = Some(other.to_string());
                Ok(())
            } else {
                Err(ConfigError::ArgIsNotValid(arg.to_string()))
            }
        }
    }
}

fn finish_command(pending: PendingCommand) -> Result<Command> {
    match pending {
        PendingCommand::Picker => Ok(Command::Picker),
        PendingCommand::Clone {
            source,
            destination,
        } => {
            let source = source.ok_or(ConfigError::ArgNotFound)?;
            let destination = match destination {
                Some(dest) => dest,
                None => crate::clone::default_clone_dest_name(&source)
                    .ok_or_else(|| ConfigError::ArgIsNotValid(source.clone()))?
                    .to_string(),
            };
            Ok(Command::Clone {
                source,
                destination,
            })
        }
        PendingCommand::Delete {
            path,
            dry_run,
            permanent,
            force,
        } => {
            if force && dry_run {
                return Err(ConfigError::ForceWithDryRun);
            }
            let path = path.ok_or(ConfigError::ArgNotFound)?;
            Ok(Command::Delete {
                path,
                dry_run,
                permanent,
                force,
            })
        }
    }
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
                    permanent_delete: false,
                    clone: CloneSettings::default(),
                    config_path: path,
                    paths: vec![],
                    git_from_home: false,
                    config_existed: false,
                });
            }
            return Err(ConfigError::IoError(e, config_file.to_string()));
        }
    };

    let raw: RawConfig =
        toml::from_str(&content).map_err(ConfigError::IncorrectStructure)?;

    let raw_paths = raw.paths.unwrap_or_default();
    let git_from_home = raw.git_from_home.unwrap_or(false);
    let configured = normalize_paths(&raw_paths, env)?;
    let discovered = if git_from_home {
        git_repos_from_home(env)?
    } else {
        vec![]
    };

    Ok(Loaded {
        candidates: merge_paths(configured, discovered),
        multiplexer: raw.multiplexer.unwrap_or_default(),
        permanent_delete: raw.permanent_delete.unwrap_or(false),
        clone: raw.clone.unwrap_or_default(),
        config_path: path,
        paths: raw_paths,
        git_from_home,
        config_existed: true,
    })
}

/// Re-read session candidates from an already-resolved config path.
/// A missing file is treated as implicit (empty catalog).
pub(crate) fn reread_candidates(
    config_path: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<Vec<SessionCandidate>> {
    let explicit = Path::new(config_path).is_file();
    Ok(load_candidates(config_path, explicit, env)?.candidates)
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
pub(crate) fn expand(
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
pub(crate) fn identity(path: &str) -> String {
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

/// Trim trailing slashes except for the filesystem root.
fn trim_trailing_slashes(path: &str) -> &str {
    if path == "/" {
        path
    } else {
        path.trim_end_matches('/')
    }
}

/// Immediate parent of an expanded absolute destination.
fn expanded_parent(
    dest: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<String> {
    let dest = expand(dest, env)?;
    let dest = trim_trailing_slashes(&dest);
    if !dest.starts_with('/') {
        return Err(ConfigError::PathIsNotAbsolute(dest.to_string()));
    }
    match Path::new(dest).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            Ok(parent.display().to_string())
        }
        _ => Err(ConfigError::PathIsNotValid(dest.to_string())),
    }
}

/// Whether `dest` would already be discovered by the current config.
/// `dest` need not exist. A directory entry covers its immediate children;
/// `dir/*` covers grandchildren; `git-from-home` covers immediate children
/// of `$HOME`.
pub(crate) fn destination_covered(
    dest: &str,
    paths: &[String],
    git_from_home: bool,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<bool> {
    let dest = expand(dest, env)?;
    let dest = trim_trailing_slashes(&dest);
    if !dest.starts_with('/') {
        return Err(ConfigError::PathIsNotAbsolute(dest.to_string()));
    }
    let Some(parent) = Path::new(dest).parent() else {
        return Ok(false);
    };
    let parent =
        trim_trailing_slashes(&parent.display().to_string()).to_string();

    if git_from_home {
        match env("HOME") {
            Some(home) if !home.is_empty() => {
                let home = home.to_string_lossy();
                if trim_trailing_slashes(&home) == parent {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }

    for raw in paths {
        let expanded = expand(raw, env)?;
        if let Some(dir) = expanded.strip_suffix("/*") {
            let Some(grand) = Path::new(&parent).parent() else {
                continue;
            };
            if trim_trailing_slashes(&grand.display().to_string())
                == trim_trailing_slashes(dir)
            {
                return Ok(true);
            }
        } else if trim_trailing_slashes(&expanded) == parent {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Append `dest`'s expanded absolute parent to `paths` in `config_path`.
/// Existing files keep comments and unknown keys. A missing file is created
/// as a minimal `paths = ["<parent>"]` document, including parent dirs.
pub(crate) fn append_parent_to_paths(
    dest: &str,
    config_path: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<()> {
    let parent = expanded_parent(dest, env)?;
    if Path::new(config_path).exists() {
        let content = fs::read_to_string(config_path)
            .map_err(|e| ConfigError::IoError(e, config_path.to_string()))?;
        let mut doc: DocumentMut =
            content.parse().map_err(ConfigError::IncorrectEdit)?;
        match doc.get_mut("paths") {
            Some(item) => {
                let Some(arr) = item.as_array_mut() else {
                    return Err(ConfigError::IoError(
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "`paths` must be an array",
                        ),
                        config_path.to_string(),
                    ));
                };
                arr.push(parent.as_str());
            }
            None => {
                let mut arr = Array::new();
                arr.push(parent.as_str());
                doc["paths"] = Item::Value(Value::Array(arr));
            }
        }
        fs::write(config_path, doc.to_string())
            .map_err(|e| ConfigError::IoError(e, config_path.to_string()))?;
        return Ok(());
    }

    if let Some(dir) = Path::new(config_path).parent() {
        fs::create_dir_all(dir)
            .map_err(|e| ConfigError::IoError(e, config_path.to_string()))?;
    }
    let mut doc = DocumentMut::new();
    let mut arr = Array::new();
    arr.push(parent.as_str());
    doc["paths"] = Item::Value(Value::Array(arr));
    fs::write(config_path, doc.to_string())
        .map_err(|e| ConfigError::IoError(e, config_path.to_string()))?;
    Ok(())
}

impl ResolvedConfig {
    pub(crate) fn destination_covered(
        &self,
        dest: &str,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Result<bool> {
        destination_covered(dest, &self.paths, self.git_from_home, env)
    }

    pub(crate) fn append_parent_to_paths(
        &self,
        dest: &str,
        env: &dyn Fn(&str) -> Option<OsString>,
    ) -> Result<()> {
        append_parent_to_paths(dest, &self.config_path, env)
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

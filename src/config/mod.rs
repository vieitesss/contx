mod error;

use serde::Deserialize;
use shellexpand;
use std::{env, fs, io, path::Path};

use error::ConfigError;

type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub paths: Option<Vec<String>>,
    pub config_file: Option<String>,
    #[serde(rename = "git-from-home")]
    pub git_from_home: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            paths: Default::default(),
            config_file: Some(String::from("~/.config/contx/config.toml")),
            git_from_home: None,
        }
    }
}

fn is_directory(path: &str) -> bool {
    Path::new(path).is_dir()
}

fn inner_dirs(dir: &str) -> Vec<String> {
    Path::new(dir)
        .read_dir()
        .expect("read_dir call failed")
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.path().display().to_string())
        .collect()
}

fn normalize_path(path: &str) -> Result<Vec<String>> {
    let p: String = match shellexpand::full(path) {
        Ok(o) => String::from(o),
        Err(e) => {
            return Err(ConfigError::PathHasInvalidEnv(
                e.cause,
                e.var_name,
                path.to_string(),
            ));
        }
    };

    if !p.starts_with('/') {
        return Err(ConfigError::PathIsNotAbsolute(path.to_string()));
    }

    if is_directory(&p) {
        return Ok(inner_dirs(&p));
    }

    if p.ends_with("/*") {
        let dir = p.get(..(p.len() - 2)).unwrap();
        if !is_directory(&dir) {
            return Err(ConfigError::PathIsNotDirectory(path.to_string()));
        }
        let all: Vec<String> =
            inner_dirs(dir).iter().flat_map(|d| inner_dirs(d)).collect();

        return Ok(all);
    }

    Err(ConfigError::PathIsNotValid(path.to_string()))
}

fn normalize_paths(paths: &[String]) -> Result<Vec<String>> {
    let mut ps = vec![];
    for p in paths.iter() {
        ps.append(&mut normalize_path(p)?);
    }
    Ok(ps)
}

fn next_not_empty_arg(args: &mut env::Args) -> Result<String> {
    if let Some(arg) = args.next() {
        Ok(arg)
    } else {
        Err(ConfigError::ArgNotFound)
    }
}

fn set_config_file(config: &mut Config, arg: &str) -> Result<()> {
    let p: String = match shellexpand::full(arg) {
        Ok(o) => String::from(o),
        Err(e) => {
            return Err(ConfigError::PathHasInvalidEnv(
                e.cause,
                e.var_name,
                arg.to_string(),
            ));
        }
    };
    if !Path::new(&p).exists() {
        return Err(ConfigError::PathIsNotValid(arg.to_string()));
    }
    config.config_file = Some(p);
    Ok(())
}

fn parse_args(config: &mut Config) -> Result<bool> {
    let mut args = env::args();
    let mut explicit_config_file = false;

    args.next(); // the script
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config-file" | "-c" => {
                let config_file = next_not_empty_arg(&mut args)?;
                set_config_file(config, &config_file)?;
                explicit_config_file = true;
            }
            _ => return Err(ConfigError::ArgIsNotValid(a)),
        }
    }

    Ok(explicit_config_file)
}

fn is_git_repository(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn discover_git_repos(home: &Path) -> Result<Vec<String>> {
    let entries = match home.read_dir() {
        Ok(entries) => entries,
        Err(e) => {
            return Err(ConfigError::IoError(
                e,
                home.display().to_string(),
            ));
        }
    };

    let mut repos: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| is_git_repository(p))
        .map(|p| p.display().to_string())
        .collect();

    repos.sort();

    Ok(repos)
}

fn git_repos_from_home() -> Result<Vec<String>> {
    let home = std::env::var_os("HOME").unwrap_or_default();
    if home.is_empty() {
        return Err(ConfigError::HomeIsNotSet);
    }

    discover_git_repos(Path::new(&home))
}

fn identity(path: &str) -> String {
    match Path::new(path).canonicalize() {
        Ok(c) => c.display().to_string(),
        Err(_) => path.to_string(),
    }
}

fn merge_paths(
    configured: Vec<String>,
    discovered: Vec<String>,
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    configured
        .into_iter()
        .chain(discovered)
        .filter(|p| seen.insert(identity(p)))
        .collect()
}

fn load_config(config_file: &str, explicit: bool) -> Result<Config> {
    let p: String = match shellexpand::full(config_file) {
        Ok(o) => String::from(o),
        Err(e) => {
            return Err(ConfigError::IoError(
                io::Error::new(io::ErrorKind::NotFound, e),
                config_file.to_string(),
            ));
        }
    };

    let content = match fs::read_to_string(&p) {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound && !explicit {
                return Ok(Config::default());
            }
            return Err(ConfigError::IoError(e, config_file.to_string()));
        }
    };

    match toml::from_str::<Config>(&content) {
        Ok(c) => {
            let paths = if let Some(p) = c.paths {
                normalize_paths(&p)?
            } else {
                vec![]
            };
            let git_repos = if let Some(ok) = c.git_from_home && ok {
                git_repos_from_home()?
            } else {
                vec![]
            };

            Ok(Config { paths: Some(merge_paths(paths, git_repos)), ..c })
        }
        Err(e) => Err(ConfigError::IncorrectStructure(e)),
    }
}

pub fn parse() -> Result<Config> {
    let mut config = Config::default();

    let explicit_config_file = parse_args(&mut config)?;

    let config_file = config
        .config_file
        .clone()
        .expect("default value should be set");

    load_config(&config_file, explicit_config_file)
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

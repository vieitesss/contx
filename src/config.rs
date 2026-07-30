use serde::Deserialize;
use shellexpand;
use std::error;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug)]
pub enum ConfigError {
    IoError(io::Error, String),
    IncorrectStructure(toml::de::Error),
    PathHasInvalidEnv(std::env::VarError, String, String),
    PathIsNotAbsolute(String),
    PathIsNotDirectory(String),
    PathIsNotValid(String),
}

impl error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            ConfigError::IoError(e, _) => Some(e),
            ConfigError::IncorrectStructure(e) => Some(e),
            ConfigError::PathHasInvalidEnv(e, _, _) => Some(e),
            ConfigError::PathIsNotAbsolute(_) => None,
            ConfigError::PathIsNotDirectory(_) => None,
            ConfigError::PathIsNotValid(_) => None,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::IoError(e, path) => {
                write!(f, "{e}; {path}")
            }
            ConfigError::PathIsNotAbsolute(path) => {
                write!(f, "not an absolute path: `{path}`")
            }
            ConfigError::PathIsNotDirectory(path) => {
                write!(f, "not a directory: `{path}`")
            }
            ConfigError::PathIsNotValid(path) => {
                write!(f, "not a valid path: `{path}`; TODO: refer to help")
            }
            ConfigError::IncorrectStructure(e) => {
                write!(f, "{e}")
            }
            ConfigError::PathHasInvalidEnv(e, env, path) => {
                write!(f, "{e}; `{env}` in `{path}`")
            }
        }
    }
}

#[derive(Default, Deserialize)]
pub struct Config {
    pub paths: Vec<String>,
}

fn is_directory(path: &str) -> bool {
    Path::new(path).is_dir()
}

fn inner_dirs(dir: &str) -> Vec<String> {
    Path::new(dir)
        .read_dir()
        .expect("read_dir call failed")
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.path().display().to_string())
        .collect()
}

fn normalize_path(path: &str) -> Result<Vec<String>, ConfigError> {
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

fn normalize_paths(paths: &[&str]) -> Result<Vec<String>, ConfigError> {
    let mut ps = vec![];
    for p in paths.iter() {
        ps.append(&mut normalize_path(p)?);
    }
    Ok(ps)
}

pub fn parse(config_path: &str) -> Result<Config, ConfigError> {
    let p: String = match shellexpand::full(config_path) {
        Ok(o) => String::from(o),
        Err(e) => {
            return Err(ConfigError::IoError(
                io::Error::new(io::ErrorKind::NotFound, e),
                config_path.to_string(),
            ));
        }
    };

    let res = fs::read_to_string(p);
    if let Err(e) = res {
        return Err(ConfigError::IoError(e, config_path.to_string()));
    }

    let content = res.unwrap();

    match toml::from_str::<Config>(&content) {
        Ok(config) => {
            let paths: Vec<&str> =
                config.paths.iter().map(String::as_str).collect();
            let norm_paths = normalize_paths(&paths)?;

            Ok(Config { paths: norm_paths })
        }
        Err(e) => return Err(ConfigError::IncorrectStructure(e)),
    }
}

mod error;

use log::debug;
use serde::Deserialize;
use shellexpand;
use std::{env, fs, io, path::Path};

use error::ConfigError;

type Result<T> = std::result::Result<T, ConfigError>;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub paths: Option<Vec<String>>,
    pub config_file: Option<String>,
    pub git_from_home: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            paths: Default::default(),
            config_file: Some(String::from("~/.config/contx/config.toml")),
            git_from_home: Some(true),
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

fn normalize_paths(paths: &[&str]) -> Result<Vec<String>> {
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

fn parse_args(config: &mut Config) -> Result<()> {
    let mut args = env::args();

    args.next(); // the script
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config-file" | "-c" => {
                let config_file = next_not_empty_arg(&mut args)?;
                let p = Path::new(&config_file);
                if !p.exists() {
                    return Err(ConfigError::PathIsNotValid(config_file));
                }
                config.config_file = Some(config_file);
            }
            _ => return Err(ConfigError::ArgIsNotValid(a)),
        }
    }

    Ok(())
}

pub fn parse() -> Result<Config> {
    let mut config = Config::default();

    parse_args(&mut config)?;

    let p: String = match shellexpand::full(
        &config
            .config_file
            .clone()
            .expect("default value should be set"),
    ) {
        Ok(o) => String::from(o),
        Err(e) => {
            return Err(ConfigError::IoError(
                io::Error::new(io::ErrorKind::NotFound, e),
                config.config_file.unwrap(),
            ));
        }
    };

    let res = fs::read_to_string(p);
    if let Err(e) = res {
        return Err(ConfigError::IoError(e, config.config_file.unwrap()));
    }

    let content = res.unwrap();

    config = match toml::from_str::<Config>(&content) {
        Ok(c) => {
            let paths = if let Some(paths) = c.paths {
                let paths: Vec<&str> =
                    paths.iter().map(String::as_str).collect();
                let norm_paths = normalize_paths(&paths)?;
                Some(norm_paths)
            } else {
                None
            };

            Config { paths, ..c }
        }
        Err(e) => return Err(ConfigError::IncorrectStructure(e)),
    };

    Ok(config)
}

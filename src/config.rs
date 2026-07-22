use serde::Deserialize;
use std::error;
use std::fmt;
use std::fs;

#[derive(Debug)]
pub enum ConfigError {
    FileMissing(std::io::Error, String),
    IncorrectStructure(toml::de::Error),
}

impl error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            ConfigError::FileMissing(e, _) => Some(e),
            ConfigError::IncorrectStructure(e) => Some(e),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::FileMissing(e, path) => {
                write!(f, "{e}; {path}")
            }
            ConfigError::IncorrectStructure(e) => {
                write!(f, "{e}")
            }
        }
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(error: toml::de::Error) -> Self {
        ConfigError::IncorrectStructure(error)
    }
}

#[derive(Default, Deserialize)]
pub struct Config {
    pub paths: Vec<String>,
}

pub fn parse(config_path: &str) -> Result<Config, ConfigError> {
    let res = fs::read_to_string(config_path);
    if let Err(e) = res {
        return Err(ConfigError::FileMissing(e, config_path.to_string()));
    }

    let content = res.unwrap();

    match toml::from_str::<Config>(&content) {
        Ok(content) => Ok(content),
        Err(e) => Err(ConfigError::IncorrectStructure(e)),
    }
}

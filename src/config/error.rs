use std::{error, fmt, io};

#[derive(Debug)]
pub enum ConfigError {
    IoError(io::Error, String),
    IncorrectStructure(toml::de::Error),
    ArgIsNotValid(String),
    ArgNotFound,
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
            ConfigError::ArgIsNotValid(_) => None,
            ConfigError::ArgNotFound => None,
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
            ConfigError::ArgIsNotValid(arg) => {
                write!(f, "not a valid argument: {arg}")
            }
            ConfigError::ArgNotFound => {
                write!(f, "an argument was expected but wasn't found")
            }
        }
    }
}

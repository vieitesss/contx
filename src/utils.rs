use std::path::Path;

use crate::tmux;

pub fn path_to_tmux_session_name(path: &str) -> String {
    let p = Path::new(path);
    let basename = String::from(p.file_name().unwrap().to_string_lossy());

    let parent_path = p.parent().unwrap();
    let parent_dir =
        String::from(parent_path.file_name().unwrap().to_string_lossy());

    let final_name: String;
    let home =
        String::from(std::env::var_os("HOME").unwrap().to_string_lossy());
    if let Ok(_) = parent_path.strip_prefix(&home) {
        let grandparent = parent_path.parent().unwrap();
        if let Ok(res) = grandparent.strip_prefix(&home)
            && res.is_empty()
        {
            let user = String::from(
                std::env::var_os("USER").unwrap().to_string_lossy(),
            );
            if parent_dir != user {
                final_name = vec![&parent_dir, "_", &basename].join("");
            } else {
                final_name = basename;
            }
        } else {
            final_name = vec![&parent_dir, "_", &basename].join("");
        }
    } else {
        final_name = basename
    }

    tmux::normalize_session_name(&final_name)
}

#[cfg(test)]
pub mod test_utils {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Serializes environment mutations across all test modules.
    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A temporary directory that cleans itself up on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "contx_test_{}_{n}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        /// Create a child directory at `path` (e.g. "work/something") and return its full path.
        pub fn child(&self, path: &str) -> PathBuf {
            let p = self.0.join(path);
            fs::create_dir_all(&p).unwrap();
            p
        }

        /// Write a file at `path` (creating parents) and return its full path.
        pub fn file(&self, path: &str, content: &str) -> PathBuf {
            let p = self.0.join(path);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&p, content).unwrap();
            p
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Set `$HOME` to `home` for the duration of `f`, then restore the original.
    pub fn with_home(home: &Path, f: impl FnOnce()) {
        let _lock = ENV_LOCK.lock().unwrap();
        // SAFETY: serialized by ENV_LOCK; test-only.
        let old = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", home) };
        f();
        match old {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    /// Unset `$HOME` for the duration of `f`, then restore the original.
    pub fn without_home(f: impl FnOnce()) {
        let _lock = ENV_LOCK.lock().unwrap();
        // SAFETY: serialized by ENV_LOCK; test-only.
        let old = std::env::var_os("HOME");
        unsafe { std::env::remove_var("HOME") };
        f();
        if let Some(v) = old {
            unsafe { std::env::set_var("HOME", v) };
        }
    }
}

#[cfg(test)]
mod test {
    use super::path_to_tmux_session_name;
    use super::test_utils::{TempDir, with_home};
    use std::fs;

    #[test]
    fn path_to_tmux_session_name_test() {
        let d = TempDir::new();

        with_home(d.path(), || {
            let p = d.child("work/something");
            assert_eq!(
                path_to_tmux_session_name(p.to_str().unwrap()),
                "work_something"
            );

            let p = d.child("personal/contx");
            assert_eq!(
                path_to_tmux_session_name(p.to_str().unwrap()),
                "personal_contx"
            );

            let p = d.child("personal/.dot");
            assert_eq!(
                path_to_tmux_session_name(p.to_str().unwrap()),
                "personal__dot"
            );
        });
    }

    #[test]
    fn paths_with_home_test() {
        let d = TempDir::new();

        // Simulate ~/work/something under a mocked $HOME
        with_home(d.path(), || {
            let expanded = shellexpand::full("~/work/something").unwrap();
            // Create the directory so open_from_path could find it
            fs::create_dir_all(expanded.as_ref()).unwrap();

            assert_eq!(
                path_to_tmux_session_name(expanded.as_ref()),
                "work_something"
            );

            // Also works with explicit $HOME variable in the path
            let expanded2 = shellexpand::full("$HOME/work/foo").unwrap();
            fs::create_dir_all(expanded2.as_ref()).unwrap();
            assert_eq!(
                path_to_tmux_session_name(expanded2.as_ref()),
                "work_foo"
            );
        });
    }
}

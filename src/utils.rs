#[cfg(test)]
pub mod test_utils {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// A temporary directory that cleans itself up on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir()
                .join(format!("contx_test_{}_{n}", std::process::id()));
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
}

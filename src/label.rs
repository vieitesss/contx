use std::{ffi::OsString, path::Path};

/// A session candidate that cannot produce a project-target label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InvalidCandidate(pub(crate) String);

/// Shared project-target label from a session-candidate path.
/// tmux uses the result as the session name; Herdr uses it only
/// as a workspace-create label. Dots become underscores.
pub(crate) fn project_target_label(
    candidate: &str,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Result<String, InvalidCandidate> {
    let invalid = || InvalidCandidate(candidate.to_string());
    let p = Path::new(candidate);
    let basename = p
        .file_name()
        .ok_or_else(invalid)?
        .to_string_lossy()
        .into_owned();
    let parent_path = p.parent().ok_or_else(invalid)?;
    let parent_dir = parent_path
        .file_name()
        .ok_or_else(invalid)?
        .to_string_lossy()
        .into_owned();
    let home = env("HOME")
        .ok_or_else(invalid)?
        .to_string_lossy()
        .into_owned();

    let final_name: String;
    if parent_path.strip_prefix(&home).is_ok() {
        let grandparent = parent_path.parent().ok_or_else(invalid)?;
        if grandparent
            .strip_prefix(&home)
            .is_ok_and(|res| res.is_empty())
        {
            let user = env("USER")
                .ok_or_else(invalid)?
                .to_string_lossy()
                .into_owned();
            if parent_dir != user {
                final_name = [&parent_dir, "_", &basename].join("");
            } else {
                final_name = basename;
            }
        } else {
            final_name = [&parent_dir, "_", &basename].join("");
        }
    } else {
        final_name = basename
    }

    Ok(final_name.replace(".", "_"))
}

#[cfg(test)]
mod tests {
    use super::{InvalidCandidate, project_target_label};
    use std::ffi::OsString;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let owned: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), OsString::from(*v)))
            .collect();
        move |name: &str| {
            owned
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    fn inside() -> impl Fn(&str) -> Option<OsString> {
        env(&[("HOME", "/home/tester"), ("USER", "tester")])
    }

    /// One table: home direct child, nested home path, USER-named
    /// home child, outside HOME, dotted name, deeper nest.
    const VALID: &[(&str, &str)] = &[
        ("/home/tester/contx", "tester_contx"),
        ("/home/tester/work/something", "work_something"),
        ("/home/tester/personal/contx", "personal_contx"),
        ("/home/tester/personal/.dot", "personal__dot"),
        ("/home/tester/tester/proj", "proj"),
        ("/srv/other/proj", "proj"),
        ("/home/tester/a/b/c", "b_c"),
    ];

    #[test]
    fn derives_labels_from_candidate_paths() {
        let env = inside();
        for (candidate, expected) in VALID {
            assert_eq!(
                project_target_label(candidate, &env).as_deref(),
                Ok(*expected),
                "{candidate}"
            );
        }
    }

    #[test]
    fn invalid_candidates_fail() {
        let inside_env = inside();
        assert!(matches!(
            project_target_label("/foo", &inside_env),
            Err(InvalidCandidate(c)) if c == "/foo"
        ));

        let no_home = env(&[("USER", "tester")]);
        assert!(matches!(
            project_target_label("/home/tester/work/foo", &no_home),
            Err(InvalidCandidate(_))
        ));

        let no_user = env(&[("HOME", "/home/tester")]);
        assert!(matches!(
            project_target_label("/home/tester/sub/proj", &no_user),
            Err(InvalidCandidate(_))
        ));
    }

    #[test]
    fn missing_user_is_ok_when_home_relative_rules_do_not_need_it() {
        let no_user = env(&[("HOME", "/home/tester")]);
        assert_eq!(
            project_target_label("/srv/other/proj", &no_user).as_deref(),
            Ok("proj")
        );
        assert_eq!(
            project_target_label("/home/tester/a/b/c", &no_user).as_deref(),
            Ok("b_c")
        );
    }
}

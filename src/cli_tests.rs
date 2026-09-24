use super::{Opened, candidate_path, open};
use crate::{
    config::{Command, Multiplexer, ResolvedConfig, SessionCandidate},
    utils::test_utils::TempDir,
};

fn cfg(project: &std::path::Path) -> ResolvedConfig {
    ResolvedConfig {
        candidates: vec![SessionCandidate::new(
            project.display().to_string(),
            project.parent().unwrap().display().to_string(),
        )],
        multiplexer: Multiplexer::Tmux,
        command: Command::Picker,
        json: true,
        permanent_delete: false,
        clone: crate::config::CloneSettings::default(),
        config_path: String::new(),
        paths: vec![],
        git_from_home: false,
        config_existed: false,
    }
}

#[test]
fn resolves_current_candidate_by_canonical_identity() {
    let d = TempDir::new();
    let project = d.child("group/project");
    let config = cfg(&project);
    let alternate = format!("{}/group/../group/project", d.path().display());
    assert_eq!(
        candidate_path(&config, &alternate).unwrap(),
        project.to_str().unwrap()
    );
    assert!(candidate_path(&config, d.path().to_str().unwrap()).is_err());
}

#[test]
fn refuses_workspace_id_before_attempting_tmux_activation() {
    let d = TempDir::new();
    let project = d.child("group/project");
    let config = cfg(&project);
    let err =
        open(&config, project.to_str().unwrap(), Some("other")).unwrap_err();
    assert!(err.to_string().contains("--workspace-id"));
}

#[test]
fn opened_serializes_a_stable_multiplexer_discriminator() {
    let value = serde_json::to_value(Opened::Herdr {
        path: "/repo".into(),
        workspace_id: "w7".into(),
    })
    .unwrap();
    assert_eq!(value["multiplexer"], "herdr");
    assert_eq!(value["workspace_id"], "w7");
}

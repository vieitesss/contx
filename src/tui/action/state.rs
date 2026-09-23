use super::field::Field;
use crate::config::{CloneProtocol, CloneSettings};

/// What the clone dialog does with a valid destination: run
/// `git clone`, or create an empty directory. Both share the
/// destination field, the add-parent offer, and the result stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloneKind {
    Repository,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloneStage {
    SourceDest,
    Authenticate,
    Clone,
    Result,
}

impl CloneStage {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::SourceDest => 0,
            Self::Authenticate => 1,
            Self::Clone => 2,
            Self::Result => 3,
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::SourceDest => "Source & destination",
            Self::Authenticate => "Authenticate",
            Self::Clone => "Clone",
            Self::Result => "Result",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeleteStage {
    Target,
    RemoteVerification,
    Findings,
    Confirm,
    Delete,
}

impl DeleteStage {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Target => 0,
            Self::RemoteVerification => 1,
            Self::Findings => 2,
            Self::Confirm => 3,
            Self::Delete => 4,
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Target => "Target & strategy",
            Self::RemoteVerification => "Remote verification",
            Self::Findings => "Findings",
            Self::Confirm => "Confirm",
            Self::Delete => "Delete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusItem {
    Source,
    ProtocolSsh,
    ProtocolHttps,
    SshPrefix,
    HttpsPrefix,
    PresetsToggle,
    Dest,
    AddParent,
    Prompt,
    PermPath,
    Warnings,
    Cancel,
    Action,
    Ack,
    AcceptKey,
    RejectKey,
    RequestCancel,
    ForceStop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    Cancelled,
    Failed {
        message: String,
    },
    /// Mutation succeeded. Either ancillary error means the Result
    /// stage stays until Acknowledge; both `None` is full success.
    Completed {
        summary: String,
        config_error: Option<String>,
        refresh_error: Option<String>,
    },
}

impl DialogOutcome {
    pub(crate) fn needs_ack(&self) -> bool {
        match self {
            Self::Cancelled => false,
            Self::Failed { .. } => true,
            Self::Completed {
                config_error,
                refresh_error,
                ..
            } => config_error.is_some() || refresh_error.is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelState {
    Idle,
    Grace,
    ForceReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeleteConfirm {
    Trash,
    Worktree,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloneAuth {
    None,
    HostKey,
    Passphrase,
    Username,
}

pub(crate) struct CloneForm {
    pub kind: CloneKind,
    pub stage: CloneStage,
    pub source: Field,
    pub protocol: CloneProtocol,
    pub ssh_prefix: Field,
    pub https_prefix: Field,
    pub show_prefixes: bool,
    pub dest: Field,
    pub dest_edited: bool,
    pub prompt: Field,
    pub add_parent: bool,
    pub parent: Option<String>,
    pub running: bool,
    pub auth: CloneAuth,
}

impl CloneForm {
    pub(crate) fn new(
        parent: Option<String>,
        kind: CloneKind,
        settings: CloneSettings,
    ) -> Self {
        let mut ssh_prefix = Field::new();
        ssh_prefix.set_str(&settings.ssh_prefix);
        let mut https_prefix = Field::new();
        https_prefix.set_str(&settings.https_prefix);
        Self {
            kind,
            stage: CloneStage::SourceDest,
            source: Field::new(),
            protocol: settings.default_protocol,
            ssh_prefix,
            https_prefix,
            show_prefixes: false,
            dest: Field::new(),
            dest_edited: false,
            prompt: Field::new(),
            add_parent: false,
            parent,
            running: false,
            auth: CloneAuth::None,
        }
    }

    pub(crate) fn prefix(&self) -> &str {
        match self.protocol {
            CloneProtocol::Ssh => self.ssh_prefix.text(),
            CloneProtocol::Https => self.https_prefix.text(),
        }
    }

    pub(crate) fn assembled_source(&self) -> String {
        let prefix = self.prefix().trim();
        let repo = self.source.text().trim().trim_start_matches('/');
        let separator = if prefix.ends_with(':')
            || prefix.ends_with('/')
            || repo.is_empty()
        {
            ""
        } else {
            "/"
        };
        format!("{prefix}{separator}{repo}")
    }

    pub(crate) fn git_started(&self) -> bool {
        self.stage != CloneStage::SourceDest
    }

    /// Accordion length: git clone has four stages; creating a
    /// directory collapses authentication and run into the result.
    pub(crate) fn stage_n(&self) -> usize {
        match self.kind {
            CloneKind::Repository => 4,
            CloneKind::Directory => 2,
        }
    }

    /// Display index for the current stage. Directory mode never
    /// enters `Authenticate`/`Clone`, so `Result` is the second row.
    pub(crate) fn stage_index(&self) -> usize {
        match self.kind {
            CloneKind::Repository => self.stage.index(),
            CloneKind::Directory => match self.stage {
                CloneStage::Result => 1,
                _ => 0,
            },
        }
    }
}

pub(crate) struct DeleteForm {
    pub stage: DeleteStage,
    pub path: String,
    pub perm: Field,
    pub prompt: Field,
    pub running: bool,
    pub blocked: bool,
    pub confirm: DeleteConfirm,
    pub class: String,
    pub strategy: String,
    pub findings: Vec<String>,
}

impl DeleteForm {
    pub(crate) fn new(path: String) -> Self {
        Self {
            stage: DeleteStage::Target,
            path,
            perm: Field::new(),
            prompt: Field::new(),
            running: false,
            blocked: false,
            confirm: DeleteConfirm::Trash,
            class: "session candidate".into(),
            strategy: "trash".into(),
            findings: vec![],
        }
    }

    pub(crate) fn git_started(&self) -> bool {
        self.stage != DeleteStage::Target
    }
}

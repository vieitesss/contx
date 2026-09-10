use super::field::Field;

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
    pub stage: CloneStage,
    pub source: Field,
    pub dest: Field,
    pub prompt: Field,
    pub add_parent: bool,
    pub parent: Option<String>,
    pub running: bool,
    pub auth: CloneAuth,
}

impl CloneForm {
    pub(crate) fn new(parent: Option<String>) -> Self {
        Self {
            stage: CloneStage::SourceDest,
            source: Field::new(),
            dest: Field::new(),
            prompt: Field::new(),
            add_parent: false,
            parent,
            running: false,
            auth: CloneAuth::None,
        }
    }

    pub(crate) fn git_started(&self) -> bool {
        self.stage != CloneStage::SourceDest
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

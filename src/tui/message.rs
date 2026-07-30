use crate::tmux::errors::TmuxError;

pub enum Message {
    TmuxError(TmuxError),
    Exit,
    FilterSessions(String),
    NextSession,
    PrevSession,
    FirstSession,
    LastSession,
    SelectSession,
}

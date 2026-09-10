//! Recognize known Git/SSH prompts from a VT grid, and keep secrets
//! out of logs and debug output.

use std::fmt;

/// Known interactive Git/SSH prompts that get native controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptKind {
    HostKey,
    Username,
    Password,
    Passphrase,
}

/// How the dialog should present the current subprocess output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Interaction {
    /// Unknown output: embed the VT grid and forward keys.
    Terminal,
    /// Recognized prompt. `hint` is the (redacted) prompt line.
    Native { kind: PromptKind, hint: String },
}

/// Bullet-mask a secret for the native widget.
pub(crate) fn mask_secret(value: &str) -> String {
    value.chars().map(|_| '•').collect()
}

fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.is_empty() {
            continue;
        }
        if out.contains(secret.as_str()) {
            out = out.replace(secret, &mask_secret(secret));
        }
    }
    out
}

fn nonempty_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect()
}

fn classify_line(line: &str) -> Option<PromptKind> {
    let lower = line.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if lower.contains("continue connecting (yes/no") {
        return Some(PromptKind::HostKey);
    }
    if lower.contains("enter passphrase for")
        || lower.contains("passphrase for key")
        || lower.ends_with("passphrase:")
    {
        return Some(PromptKind::Passphrase);
    }
    if lower.contains("username for ")
        || lower.starts_with("username:")
        || lower.starts_with("login as:")
    {
        return Some(PromptKind::Username);
    }
    if lower.contains("password for ")
        || lower.contains("'s password:")
        || lower.ends_with("password:")
    {
        return Some(PromptKind::Password);
    }
    None
}

fn classify_text(text: &str) -> Option<PromptKind> {
    let lines = nonempty_lines(text);
    let last = lines.last().copied()?;
    if let Some(kind) = classify_line(last) {
        return Some(kind);
    }
    if let Some(prev) = lines.iter().rev().nth(1) {
        classify_line(&format!("{prev} {last}"))
    } else {
        None
    }
}

fn hint_line(text: &str) -> String {
    nonempty_lines(text)
        .last()
        .copied()
        .unwrap_or("")
        .to_string()
}

fn is_secret_write(text: &str, secrets: &[String]) -> bool {
    let trimmed = text.trim_end_matches(['\r', '\n']);
    secrets.iter().any(|s| {
        !s.is_empty() && (trimmed == s || trimmed.contains(s.as_str()))
    })
}

/// VT parser plus prompt classification and secret redaction.
pub(crate) struct PromptDecoder {
    parser: vt100::Parser,
    secrets: Vec<String>,
    log: Vec<String>,
    writes: Vec<String>,
    interaction: Interaction,
}

impl PromptDecoder {
    pub(crate) fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 64),
            secrets: Vec::new(),
            log: Vec::new(),
            writes: Vec::new(),
            interaction: Interaction::Terminal,
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
        self.refresh();
    }

    /// Remember a secret so it is never stored in logs or debug.
    pub(crate) fn note_secret(&mut self, secret: &str) {
        if secret.is_empty() {
            return;
        }
        if !self.secrets.iter().any(|s| s == secret) {
            self.secrets.push(secret.to_string());
        }
        self.refresh();
    }

    /// Record a PTY write. Secret writes are dropped, not logged.
    pub(crate) fn record_write(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        if is_secret_write(&text, &self.secrets) {
            return;
        }
        let redacted = redact(text.trim_end(), &self.secrets);
        if !redacted.is_empty() {
            self.writes.push(redacted);
        }
    }

    pub(crate) fn interaction(&self) -> &Interaction {
        &self.interaction
    }

    pub(crate) fn log(&self) -> &[String] {
        &self.log
    }

    pub(crate) fn writes(&self) -> &[String] {
        &self.writes
    }

    /// Visible VT rows, secrets replaced with bullets.
    pub(crate) fn grid_lines(&self) -> Vec<String> {
        let (_, cols) = self.parser.screen().size();
        self.parser
            .screen()
            .rows(0, cols)
            .map(|row| redact(&row, &self.secrets).trim_end().to_string())
            .collect()
    }

    pub(crate) fn contents(&self) -> String {
        redact(&self.parser.screen().contents(), &self.secrets)
    }

    fn refresh(&mut self) {
        let raw = self.parser.screen().contents();
        let redacted = redact(&raw, &self.secrets);
        self.interaction = match classify_text(&raw) {
            Some(kind) => Interaction::Native {
                kind,
                hint: redact(&hint_line(&raw), &self.secrets),
            },
            None => Interaction::Terminal,
        };
        self.log = nonempty_lines(&redacted)
            .into_iter()
            .map(str::to_string)
            .collect();
        self.writes = self
            .writes
            .iter()
            .map(|w| redact(w, &self.secrets))
            .collect();
    }
}

impl fmt::Debug for PromptDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PromptDecoder")
            .field("interaction", &self.interaction)
            .field("log", &self.log)
            .field("writes", &self.writes)
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;

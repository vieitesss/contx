use super::{Interaction, PromptDecoder, PromptKind, mask_secret};

fn decoder() -> PromptDecoder {
    PromptDecoder::new(24, 80)
}

fn native_kind(decoder: &PromptDecoder) -> PromptKind {
    match decoder.interaction() {
        Interaction::Native { kind, .. } => *kind,
        other => panic!("expected native prompt, got {other:?}"),
    }
}

#[test]
fn mask_secret_uses_bullets() {
    assert_eq!(mask_secret(""), "");
    assert_eq!(mask_secret("ab"), "••");
    assert!(mask_secret("hunter2").chars().all(|c| c == '•'));
    assert_eq!(mask_secret("hunter2").chars().count(), 7);
    assert!(!mask_secret("hunter2").contains('h'));
}

#[test]
fn host_key_prompt_is_native_and_keeps_context() {
    let mut d = decoder();
    d.feed(
        b"The authenticity of host 'github.com (140.82.114.4)' can't be established.\r\n",
    );
    d.feed(b"ED25519 key fingerprint is SHA256:abc.\r\n");
    d.feed(b"Are you sure you want to continue connecting (yes/no/[fingerprint])? ");
    assert_eq!(native_kind(&d), PromptKind::HostKey);
    let log = d.log().join("\n");
    assert!(log.contains("authenticity of host"), "context lost: {log}");
    match d.interaction() {
        Interaction::Native { hint, .. } => {
            assert!(hint.contains("continue connecting (yes/no"));
        }
        Interaction::Terminal => panic!("expected native"),
    }
}

#[test]
fn username_password_and_passphrase_prompts_are_native() {
    let mut d = decoder();
    d.feed(b"Username for 'https://github.com': ");
    assert_eq!(native_kind(&d), PromptKind::Username);

    let mut d = decoder();
    d.feed(b"Password for 'https://user@github.com': ");
    assert_eq!(native_kind(&d), PromptKind::Password);

    let mut d = decoder();
    d.feed(b"user@host's password: ");
    assert_eq!(native_kind(&d), PromptKind::Password);

    let mut d = decoder();
    d.feed(b"Enter passphrase for key '/home/me/.ssh/id_ed25519': ");
    assert_eq!(native_kind(&d), PromptKind::Passphrase);
}

#[test]
fn unknown_output_stays_terminal() {
    let mut d = decoder();
    d.feed(b"Receiving objects:  62% (2165/3492), 1.21 MiB | 412 KiB/s\r\n");
    assert_eq!(d.interaction(), &Interaction::Terminal);

    let mut d = decoder();
    d.feed(b"Enter 2FA code: ");
    assert_eq!(d.interaction(), &Interaction::Terminal);
}

#[test]
fn ansi_host_key_still_recognized_and_grid_is_plain() {
    let mut d = decoder();
    d.feed(
        b"\x1b[31mAre you sure you want to continue connecting (yes/no)?\x1b[0m",
    );
    assert_eq!(native_kind(&d), PromptKind::HostKey);
    let contents = d.contents();
    assert!(
        !contents.contains('\u{1b}'),
        "grid should be plain text: {contents:?}"
    );
    assert!(contents.contains("continue connecting (yes/no)"));
    let lines = d.grid_lines();
    assert!(
        lines.iter().any(|l| l.contains("continue connecting")),
        "grid lines: {lines:?}"
    );
}

#[test]
fn secret_never_appears_in_log_debug_or_writes() {
    let mut d = decoder();
    d.feed(b"Enter passphrase for key '/tmp/id': ");
    d.note_secret("hunter2");
    d.record_write(b"hunter2\n");
    d.feed(b"echo hunter2 leaked\r\n");
    let dumped = format!("{d:?}");
    assert!(!dumped.contains("hunter2"), "debug leaked secret: {dumped}");
    let log = d.log().join("\n");
    assert!(!log.contains("hunter2"), "log leaked secret: {log}");
    assert!(
        d.writes().iter().all(|w| !w.contains("hunter2")),
        "writes leaked secret: {:?}",
        d.writes()
    );
    assert!(
        !d.contents().contains("hunter2"),
        "contents leaked secret: {}",
        d.contents()
    );
    assert!(
        d.grid_lines().iter().all(|l| !l.contains("hunter2")),
        "grid leaked secret: {:?}",
        d.grid_lines()
    );
    assert!(
        d.contents().contains("•••••••")
            || d.log().iter().any(|l| l.contains('•'))
    );
}

#[test]
fn non_secret_write_is_kept() {
    let mut d = decoder();
    d.feed(b"Are you sure you want to continue connecting (yes/no)? ");
    d.record_write(b"yes\n");
    assert_eq!(d.writes(), &["yes".to_string()]);
}

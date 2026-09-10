use super::{
    FakePty, PortablePty, PtyEvent, PtySession, PtySize, PtyTransport,
};

fn size() -> PtySize {
    PtySize { cols: 40, rows: 12 }
}

#[test]
fn fake_records_spawn_write_resize_interrupt_and_force_kill() {
    let mut fake = FakePty::new();
    let mut session = fake.spawn(&["cat", "-"], size()).expect("spawn");
    assert_eq!(
        fake.spawns(),
        vec![(vec!["cat".into(), "-".into()], size())]
    );

    session.write(b"hello").unwrap();
    session.resize(PtySize { cols: 80, rows: 24 }).unwrap();
    session.interrupt().unwrap();
    assert_eq!(fake.interrupts(), 1);
    assert_eq!(fake.force_kills(), 0);
    assert_eq!(fake.writes(), vec![b"hello".to_vec()]);
    assert_eq!(fake.resizes(), vec![PtySize { cols: 80, rows: 24 }]);

    session.force_kill().unwrap();
    assert_eq!(fake.force_kills(), 1);
    assert_eq!(fake.interrupts(), 1);
}

#[test]
fn fake_injects_output_and_exit_without_blocking() {
    let mut fake = FakePty::new();
    let mut session = fake.spawn(&["prog"], size()).unwrap();
    assert_eq!(session.try_recv(), None);

    fake.inject(PtyEvent::Output(b"out".to_vec()));
    fake.inject(PtyEvent::Exit { code: Some(0) });
    assert_eq!(session.try_recv(), Some(PtyEvent::Output(b"out".to_vec())));
    assert_eq!(session.try_recv(), Some(PtyEvent::Exit { code: Some(0) }));
    assert_eq!(session.try_recv(), None);
}

#[test]
fn fake_interrupt_is_not_force_kill() {
    let mut fake = FakePty::new();
    let mut session = fake.spawn(&["prog"], size()).unwrap();
    session.interrupt().unwrap();
    session.interrupt().unwrap();
    assert_eq!(fake.interrupts(), 2);
    assert_eq!(fake.force_kills(), 0);
    assert!(fake.writes().is_empty());
}

#[test]
fn empty_argv_is_an_error_on_both_transports() {
    let mut fake = FakePty::new();
    assert!(fake.spawn(&[], size()).is_err());
    let mut portable = PortablePty;
    assert!(portable.spawn(&[], size()).is_err());
}

#[cfg(unix)]
fn wait_event(
    session: &mut dyn PtySession,
    pred: impl Fn(&PtyEvent) -> bool,
) -> Option<PtyEvent> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Some(event) = session.try_recv() {
            if pred(&event) {
                return Some(event);
            }
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    None
}

#[cfg(unix)]
#[test]
fn portable_echo_emits_output_and_exit() {
    let mut pty = PortablePty;
    let mut session = pty
        .spawn(&["/bin/echo", "hello"], size())
        .expect("spawn echo");
    let mut buf = Vec::new();
    let mut exit = None;
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match session.try_recv() {
            Some(PtyEvent::Output(bytes)) => buf.extend(bytes),
            Some(PtyEvent::Exit { code }) => {
                exit = Some(code);
                break;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let text = String::from_utf8_lossy(&buf);
    assert!(text.contains("hello"), "output was {text:?}");
    assert_eq!(exit, Some(Some(0)));
}

#[cfg(unix)]
#[test]
fn portable_force_kill_stops_a_sleeping_child() {
    let mut pty = PortablePty;
    let mut session = pty
        .spawn(&["/bin/sleep", "30"], size())
        .expect("spawn sleep");
    session.force_kill().expect("force kill");
    let exit =
        wait_event(&mut *session, |e| matches!(e, PtyEvent::Exit { .. }));
    assert!(
        matches!(exit, Some(PtyEvent::Exit { .. })),
        "child did not exit after force_kill: {exit:?}"
    );
}

#[cfg(unix)]
#[test]
fn portable_interrupt_writes_vintr_and_can_stop_cat() {
    let mut pty = PortablePty;
    let mut session = pty.spawn(&["/bin/cat"], size()).expect("spawn cat");
    session.write(b"ping\n").expect("write");
    let output = wait_event(&mut *session, |e| match e {
        PtyEvent::Output(bytes) => {
            String::from_utf8_lossy(bytes).contains("ping")
        }
        _ => false,
    });
    assert!(output.is_some(), "cat did not echo ping");
    session.interrupt().expect("interrupt");
    let exit =
        wait_event(&mut *session, |e| matches!(e, PtyEvent::Exit { .. }));
    assert!(
        matches!(exit, Some(PtyEvent::Exit { .. })),
        "cat did not exit after VINTR: {exit:?}"
    );
}

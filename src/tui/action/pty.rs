//! Injectable PTY transport. Tests use `FakePty`; production wraps
//! `portable-pty`. Interrupt writes VINTR to the master — never
//! `kill(-pid)`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use portable_pty::{CommandBuilder, MasterPty, native_pty_system};

/// Display size of the slave terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    fn portable(self) -> portable_pty::PtySize {
        portable_pty::PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Bytes from the slave, or the child exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PtyEvent {
    Output(Vec<u8>),
    Exit { code: Option<i32> },
}

#[derive(Debug)]
pub(crate) struct PtyError(String);

impl PtyError {
    fn from_display(err: impl std::fmt::Display) -> Self {
        Self(err.to_string())
    }
}

impl std::fmt::Display for PtyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PtyError {}

impl From<io::Error> for PtyError {
    fn from(err: io::Error) -> Self {
        Self(err.to_string())
    }
}

/// ASCII VINTR (Ctrl-C). The PTY driver signals the slave FG group.
const VINTR: u8 = 0x03;

pub(crate) trait PtyTransport {
    fn spawn(
        &mut self,
        argv: &[&str],
        size: PtySize,
    ) -> Result<Box<dyn PtySession>, PtyError>;
}

pub(crate) trait PtySession {
    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError>;
    fn resize(&mut self, size: PtySize) -> Result<(), PtyError>;
    fn interrupt(&mut self) -> Result<(), PtyError>;
    fn force_kill(&mut self) -> Result<(), PtyError>;
    fn try_recv(&mut self) -> Option<PtyEvent>;
}

/// Scripted PTY for tests on every platform.
#[derive(Clone)]
pub(crate) struct FakePty {
    inner: Rc<RefCell<FakeInner>>,
}

struct FakeInner {
    spawns: Vec<(Vec<String>, PtySize)>,
    writes: Vec<Vec<u8>>,
    interrupts: usize,
    force_kills: usize,
    resizes: Vec<PtySize>,
    events: VecDeque<PtyEvent>,
}

impl FakePty {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(FakeInner {
                spawns: vec![],
                writes: vec![],
                interrupts: 0,
                force_kills: 0,
                resizes: vec![],
                events: VecDeque::new(),
            })),
        }
    }

    pub(crate) fn inject(&self, event: PtyEvent) {
        self.inner.borrow_mut().events.push_back(event);
    }

    pub(crate) fn spawns(&self) -> Vec<(Vec<String>, PtySize)> {
        self.inner.borrow().spawns.clone()
    }

    pub(crate) fn writes(&self) -> Vec<Vec<u8>> {
        self.inner.borrow().writes.clone()
    }

    pub(crate) fn interrupts(&self) -> usize {
        self.inner.borrow().interrupts
    }

    pub(crate) fn force_kills(&self) -> usize {
        self.inner.borrow().force_kills
    }

    pub(crate) fn resizes(&self) -> Vec<PtySize> {
        self.inner.borrow().resizes.clone()
    }
}

impl PtyTransport for FakePty {
    fn spawn(
        &mut self,
        argv: &[&str],
        size: PtySize,
    ) -> Result<Box<dyn PtySession>, PtyError> {
        if argv.is_empty() {
            return Err(PtyError("empty argv".into()));
        }
        self.inner
            .borrow_mut()
            .spawns
            .push((argv.iter().map(|s| (*s).to_string()).collect(), size));
        Ok(Box::new(self.clone()))
    }
}

impl PtySession for FakePty {
    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.inner.borrow_mut().writes.push(bytes.to_vec());
        Ok(())
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.inner.borrow_mut().resizes.push(size);
        Ok(())
    }

    fn interrupt(&mut self) -> Result<(), PtyError> {
        self.inner.borrow_mut().interrupts += 1;
        Ok(())
    }

    fn force_kill(&mut self) -> Result<(), PtyError> {
        self.inner.borrow_mut().force_kills += 1;
        Ok(())
    }

    fn try_recv(&mut self) -> Option<PtyEvent> {
        self.inner.borrow_mut().events.pop_front()
    }
}

/// Production transport. Cross-platform via `portable-pty`.
pub(crate) struct PortablePty;

struct PortableSession {
    writer: Option<Box<dyn Write + Send>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    rx: Receiver<PtyEvent>,
    exited: bool,
}

impl PtyTransport for PortablePty {
    fn spawn(
        &mut self,
        argv: &[&str],
        size: PtySize,
    ) -> Result<Box<dyn PtySession>, PtyError> {
        if argv.is_empty() {
            return Err(PtyError("empty argv".into()));
        }
        let system = native_pty_system();
        let pair = system
            .openpty(size.portable())
            .map_err(PtyError::from_display)?;
        let mut cmd = CommandBuilder::new(argv[0]);
        cmd.args(&argv[1..]);
        cmd.env("TERM", "xterm-256color");
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(PtyError::from_display)?;
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(PtyError::from_display)?;
        let writer =
            pair.master.take_writer().map_err(PtyError::from_display)?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(PtyEvent::Output(buf[..n].to_vec())).is_err()
                        {
                            break;
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                        continue;
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Box::new(PortableSession {
            writer: Some(writer),
            master: pair.master,
            child,
            rx,
            exited: false,
        }))
    }
}

impl PortableSession {
    fn writer_mut(&mut self) -> Result<&mut dyn Write, PtyError> {
        self.writer
            .as_mut()
            .map(|w| w.as_mut() as &mut dyn Write)
            .ok_or_else(|| PtyError("pty writer closed".into()))
    }
}

impl PtySession for PortableSession {
    fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        let writer = self.writer_mut()?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(size.portable())
            .map_err(PtyError::from_display)
    }

    fn interrupt(&mut self) -> Result<(), PtyError> {
        self.write(&[VINTR])
    }

    fn force_kill(&mut self) -> Result<(), PtyError> {
        self.child.kill()?;
        self.writer.take();
        Ok(())
    }

    fn try_recv(&mut self) -> Option<PtyEvent> {
        match self.rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => {
                if self.exited {
                    return None;
                }
                match self.child.try_wait() {
                    Ok(Some(status)) => {
                        self.exited = true;
                        let code = if status.success() {
                            Some(0)
                        } else {
                            Some(status.exit_code() as i32)
                        };
                        Some(PtyEvent::Exit { code })
                    }
                    _ => None,
                }
            }
        }
    }
}

impl Drop for PortableSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        self.writer.take();
    }
}

#[cfg(test)]
#[path = "pty_tests.rs"]
mod tests;

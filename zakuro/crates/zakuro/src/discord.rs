//! Discord Rich Presence: what the game is doing, shown on the player's
//! Discord profile. It talks to the Discord app on this computer through its
//! local IPC pipe; with no Discord app running here, nothing happens.

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// the Discord application the presence shows as, its name the "Playing"
/// line, made at https://discord.com/developers/applications
const APP_ID: &str = "1564029962808463410";

/// how often a Discord that is closed or not there yet is tried again.
const RETRY: Duration = Duration::from_secs(20);

/// the IPC frames' kinds.
const HANDSHAKE: u32 = 0;
const FRAME: u32 = 1;
const CLOSE: u32 = 2;

/// what the game overrides say the game is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doing {
    Menus,
    OnFoot,
    Flying,
}

impl Doing {
    fn text(self) -> &'static str {
        match self {
            Doing::Menus => "In the menus",
            Doing::OnFoot => "Fighting on foot",
            Doing::Flying => "Flying",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Activity {
    pub doing: Doing,
    /// when the game started, in seconds since 1970, for the time played.
    pub since: u64,
}

/// the presence shown, kept up to date by a thread of its own so a slow or
/// missing Discord never holds up a frame.
pub struct Presence {
    shown: Option<Activity>,
    to: Sender<Option<Activity>>,
}

impl Presence {
    pub fn new() -> Presence {
        let (to, from) = mpsc::channel();
        // ponytail: the thread sleeps for the program's life even with the presence off
        if let Err(error) = std::thread::Builder::new().name("discord".into()).spawn(move || run(&from)) {
            log::warn!("no Discord presence, {error}");
        }
        Presence { shown: None, to }
    }

    /// shows what the game is doing, or nothing, when it has changed.
    pub fn show(&mut self, activity: Option<Activity>) {
        if activity != self.shown {
            self.shown = activity;
            let _ = self.to.send(activity);
        }
    }
}

fn run(from: &Receiver<Option<Activity>>) {
    let mut wanted = None;
    let mut pending = false;
    let mut pipe: Option<Box<dyn Pipe>> = None;
    loop {
        match from.recv_timeout(RETRY) {
            Ok(activity) => {
                wanted = activity;
                pending = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // ponytail: a Discord closed while nothing changes is only noticed at the next change
        if !pending {
            continue;
        }
        if pipe.is_none() {
            if wanted.is_none() {
                // nothing shown, nothing to clear
                pending = false;
                continue;
            }
            pipe = connect().ok();
        }
        let Some(open) = &mut pipe else { continue };
        match set_activity(open, wanted, std::process::id()) {
            Ok(()) => pending = false,
            Err(error) => {
                log::info!("lost Discord, {error}");
                pipe = None;
            }
        }
    }
}

trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

/// the first Discord pipe there is, shaken hands with.
fn connect() -> io::Result<Box<dyn Pipe>> {
    let mut last = io::Error::from(io::ErrorKind::NotFound);
    for n in 0..10 {
        match open(n) {
            Ok(mut pipe) => {
                send(&mut pipe, HANDSHAKE, &format!(r#"{{"v":1,"client_id":"{APP_ID}"}}"#))?;
                log::info!("showing the game on Discord");
                return Ok(pipe);
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

#[cfg(windows)]
fn open(n: u32) -> io::Result<Box<dyn Pipe>> {
    let pipe = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{n}"))?;
    Ok(Box::new(pipe))
}

#[cfg(unix)]
fn open(n: u32) -> io::Result<Box<dyn Pipe>> {
    let dir = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(std::env::var_os)
        .unwrap_or_else(|| "/tmp".into());
    let socket = std::os::unix::net::UnixStream::connect(std::path::Path::new(&dir).join(format!("discord-ipc-{n}")))?;
    Ok(Box::new(socket))
}

fn set_activity(pipe: &mut impl Pipe, activity: Option<Activity>, pid: u32) -> io::Result<()> {
    // ponytail: the JSON is written by hand, its strings are this file's own and need no escaping
    let activity = match activity {
        Some(activity) => {
            format!(r#"{{"details":"{}","timestamps":{{"start":{}}}}}"#, activity.doing.text(), activity.since)
        }
        None => "null".to_owned(),
    };
    send(pipe, FRAME, &format!(r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{pid},"activity":{activity}}},"nonce":"{pid}"}}"#))
}

/// one frame out, then Discord's answer, an error when it closed the pipe.
fn send(pipe: &mut impl Pipe, op: u32, json: &str) -> io::Result<()> {
    pipe.write_all(&frame(op, json))?;
    let mut header = [0; 8];
    pipe.read_exact(&mut header)?;
    let [op, len] = [0, 4].map(|at| u32::from_le_bytes(header[at..at + 4].try_into().unwrap()));
    let mut answer = vec![0; len as usize];
    pipe.read_exact(&mut answer)?;
    let answer = String::from_utf8_lossy(&answer);
    if op == CLOSE {
        return Err(io::Error::new(io::ErrorKind::ConnectionAborted, answer.into_owned()));
    }
    if answer.contains(r#""evt":"ERROR""#) {
        log::warn!("Discord did not take the presence: {answer}");
    }
    Ok(())
}

fn frame(op: u32, json: &str) -> Vec<u8> {
    let mut frame = op.to_le_bytes().to_vec();
    frame.extend((json.len() as u32).to_le_bytes());
    frame.extend(json.as_bytes());
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Discord's end: answers read from, frames written to.
    struct Fake {
        answers: io::Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.answers.read(buf)
        }
    }

    impl Write for Fake {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_activity_goes_out_framed_and_a_close_is_an_error() {
        let flying = Activity { doing: Doing::Flying, since: 1_700_000_000 };
        let mut pipe = Fake { answers: io::Cursor::new(frame(FRAME, r#"{"evt":null}"#)), written: Vec::new() };
        set_activity(&mut pipe, Some(flying), 42).unwrap();
        let json = r#"{"cmd":"SET_ACTIVITY","args":{"pid":42,"activity":{"details":"Flying","timestamps":{"start":1700000000}}},"nonce":"42"}"#;
        assert_eq!(pipe.written, frame(FRAME, json));
        assert_eq!(&pipe.written[..8], &[1, 0, 0, 0, json.len() as u8, 0, 0, 0]);

        let mut pipe = Fake { answers: io::Cursor::new(frame(CLOSE, r#"{"code":4000}"#)), written: Vec::new() };
        assert!(set_activity(&mut pipe, None, 42).is_err());
        assert!(String::from_utf8_lossy(&pipe.written).contains(r#""activity":null"#));
    }
}

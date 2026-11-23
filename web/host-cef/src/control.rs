//! The control socket: the shell listens on a Unix socket, the host connects at
//! start. Events are written from CEF's UI thread; one reader thread turns each
//! command line into a task on the UI thread. When the shell closes the socket the
//! host quits.

use crate::protocol::{encode_event, event_text, Event, PROTOCOL_VERSION};
use cef::*;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

static CONTROL: Mutex<Option<UnixStream>> = Mutex::new(None);

pub fn connect(path: &str) -> std::io::Result<()> {
    let s = UnixStream::connect(path)?;
    *CONTROL.lock().unwrap() = Some(s);
    emit(Event::Hello {
        protocol: PROTOCOL_VERSION,
        engine: "cef".into(),
    });
    Ok(())
}

/// Sends one event (and logs it). Without a control socket it is only logged.
pub fn emit(ev: Event) {
    log!("event: {}", event_text(&ev));
    let mut g = CONTROL.lock().unwrap();
    if let Some(s) = g.as_mut() {
        let mut line = encode_event(&ev);
        line.push('\n');
        if let Err(e) = s.write_all(line.as_bytes()) {
            log!("control socket write failed ({e}); events are only logged from now on");
            *g = None;
        }
    }
}

wrap_task! {
    struct CommandTask {
        line: String,
    }

    impl Task {
        fn execute(&self) {
            crate::page::run_command(&self.line);
        }
    }
}

wrap_task! {
    struct ShellGoneTask {}

    impl Task {
        fn execute(&self) {
            log!("control socket closed by the shell: quitting");
            crate::page::quit();
        }
    }
}

/// Starts the reader thread (after `cef::initialize`, so tasks can be posted).
pub fn start_reader() {
    let Some(s) = CONTROL.lock().unwrap().as_ref().and_then(|s| s.try_clone().ok()) else {
        return;
    };
    std::thread::spawn(move || {
        for line in BufReader::new(s).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            log!("command: {line}");
            post_task(ThreadId::UI, Some(&mut CommandTask::new(line)));
        }
        post_task(ThreadId::UI, Some(&mut ShellGoneTask::new()));
    });
}

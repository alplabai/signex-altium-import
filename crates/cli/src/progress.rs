//! NDJSON progress events (contract section 3.1): one flushed line per
//! event, a heartbeat while a phase runs quiet, `summary` always last.

use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Signex cancels a run after 120 s of silence; the contract asks for an
/// event at least every 10 s.
pub const HEARTBEAT: Duration = Duration::from_secs(10);

struct State {
    out: Box<dyn Write + Send>,
    /// The last phase or progress line, repeated as the heartbeat.
    last: Option<String>,
    last_emit: Instant,
    done: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl State {
    fn emit(&mut self, line: &str) {
        // A closed stdout must not abort the conversion.
        let _ = writeln!(self.out, "{line}");
        let _ = self.out.flush();
        self.last_emit = Instant::now();
    }
}

/// Writes progress events; a no-op when `--progress` is off.
pub struct Progress {
    shared: Option<Arc<Shared>>,
    beat: Option<JoinHandle<()>>,
}

impl Progress {
    pub fn disabled() -> Self {
        Progress {
            shared: None,
            beat: None,
        }
    }

    pub fn new(out: Box<dyn Write + Send>, heartbeat: Duration) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                out,
                last: None,
                last_emit: Instant::now(),
                done: false,
            }),
            wake: Condvar::new(),
        });
        let t = Arc::clone(&shared);
        let beat = std::thread::spawn(move || {
            let mut st = t.state.lock().unwrap();
            while !st.done {
                let quiet = st.last_emit.elapsed();
                if quiet >= heartbeat {
                    if let Some(line) = st.last.clone() {
                        st.emit(&line);
                    } else {
                        st.last_emit = Instant::now();
                    }
                    continue;
                }
                st = t.wake.wait_timeout(st, heartbeat - quiet).unwrap().0;
            }
        });
        Progress {
            shared: Some(shared),
            beat: Some(beat),
        }
    }

    /// Progress to stdout, with the contract's 10 s heartbeat.
    pub fn stdout() -> Self {
        Self::new(Box::new(std::io::stdout()), HEARTBEAT)
    }

    pub fn enabled(&self) -> bool {
        self.shared.is_some()
    }

    fn event(&self, v: Value) {
        if let Some(s) = &self.shared {
            let line = v.to_string();
            let mut st = s.state.lock().unwrap();
            st.emit(&line);
            st.last = Some(line);
        }
    }

    /// A new phase starts (`read`, `convert`, `write`, `models`).
    pub fn phase(&self, phase: &str, total: Option<usize>) {
        self.event(json!({ "event": "phase", "phase": phase, "total": total }));
    }

    /// `done` of `total` in the current phase; `item` is display only.
    pub fn progress(&self, done: usize, total: Option<usize>, item: Option<&str>) {
        self.event(json!({ "event": "progress", "done": done, "total": total, "item": item }));
    }

    /// Write the `summary` event, the last line, and stop the heartbeat.
    pub fn summary(mut self, summary: &Value) {
        if let Some(s) = self.shared.take() {
            {
                let mut st = s.state.lock().unwrap();
                st.emit(&summary.to_string());
                st.done = true;
            }
            s.wake.notify_all();
        }
        if let Some(h) = self.beat.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        if let Some(s) = self.shared.take() {
            s.state.lock().unwrap().done = true;
            s.wake.notify_all();
        }
        if let Some(h) = self.beat.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn lines(b: &Buf) -> Vec<Value> {
        String::from_utf8(b.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn events_in_order_and_summary_last() {
        let b = Buf::default();
        let p = Progress::new(Box::new(b.clone()), Duration::from_secs(60));
        p.phase("read", Some(2));
        p.progress(1, Some(2), Some("A:B"));
        p.summary(&json!({ "event": "summary", "ok": true }));
        let l = lines(&b);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0]["event"], "phase");
        assert_eq!(l[1]["item"], "A:B");
        assert_eq!(l[2]["event"], "summary");
    }

    #[test]
    fn heartbeat_repeats_the_last_event() {
        let b = Buf::default();
        let p = Progress::new(Box::new(b.clone()), Duration::from_millis(30));
        p.progress(5, None, Some("slow"));
        std::thread::sleep(Duration::from_millis(200));
        p.summary(&json!({ "event": "summary" }));
        let l = lines(&b);
        assert!(l.len() >= 3, "{l:?}");
        assert_eq!(l[1]["item"], "slow");
        assert_eq!(l.last().unwrap()["event"], "summary");
        assert!(l[..l.len() - 1].iter().all(|v| v["event"] == "progress"));
    }

    #[test]
    fn disabled_writes_nothing() {
        let p = Progress::disabled();
        p.phase("read", None);
        assert!(!p.enabled());
        p.summary(&json!({}));
    }
}

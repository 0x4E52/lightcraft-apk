//! Opening the library session off android_main.
//!
//! Until `run` reaches the event loop's first poll, the Java main thread waits for android_main
//! (onStart/onResume, the input queue and surfaceCreated block until it handles their commands),
//! so a slow start is a black screen and a tap during it an input-dispatch ANR. Opening a library
//! replays its log and may seed the demo photos: seconds for a large one. So it runs on a worker
//! thread; the eframe app shows "Opening library…" and asks [`Loader::poll`] each frame, which
//! never waits. No session exists yet, so there is no activity stack for a `TaskGuard`: the
//! loading frame is the progress indicator.

use std::sync::mpsc::{self, Receiver, TryRecvError};

use lightcraft_engine::Session;
use lightcraft_ui_egui::panels::library_problem::LibraryProblem;

/// What opening the library gives: the session and, when the library couldn't be opened, why.
pub type Opened = (Session, Option<LibraryProblem>);

// The session is opened on the worker and handed to the UI thread, so it must be `Send`; this
// fails to compile otherwise.
const _: fn() = || {
    fn is_send<T: Send>() {}
    is_send::<Opened>();
};

/// Start opening the library at `library` (a new one starts with the procedural demo photos) on a
/// worker, with the folders and the SAM model of the desktop's `open_session`.
pub fn open_library(library: std::path::PathBuf) -> Loader {
    Loader::start(move || {
        let (session, problem) = lightcraft_ui_egui::host::session_open::open_session(false, Some(library), true);
        (session.with_default_sam3_model(), problem)
    })
}

/// The worker opening the session, until it answered once.
pub struct Loader {
    state: State,
}

enum State {
    Waiting(Receiver<Opened>),
    /// The worker couldn't be started: reported by the next poll.
    Failed(String),
    /// The answer was handed over.
    Done,
}

/// Why opening the library failed when the worker stopped without an answer (it panicked: the
/// panic hook has logged it).
pub const STOPPED: &str = "opening the library stopped after an internal error";

impl Loader {
    /// Start `open` on a worker thread and return at once.
    pub fn start(open: impl FnOnce() -> Opened + Send + 'static) -> Loader {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("open-library".into()).spawn(move || {
            // the UI may be gone by the time the library is open: nobody to hand it to
            let _ = tx.send(open());
        });
        let state = match spawned {
            Ok(_) => State::Waiting(rx),
            Err(e) => State::Failed(format!("LightCraft could not start opening the library: {e}")),
        };
        Loader { state }
    }

    /// The opened session once the worker has it (`Some` exactly once), `None` while it works or
    /// after the answer was taken. `Err` when the worker couldn't start or stopped without an
    /// answer. Never blocks.
    pub fn poll(&mut self) -> Option<Result<Opened, String>> {
        let answer = match &self.state {
            State::Waiting(rx) => match rx.try_recv() {
                Ok(opened) => Ok(opened),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => Err(STOPPED.to_owned()),
            },
            State::Failed(e) => Err(e.clone()),
            State::Done => return None,
        };
        self.state = State::Done;
        Some(answer)
    }

    /// Whether the answer was handed over.
    pub fn is_done(&self) -> bool {
        matches!(self.state, State::Done)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    /// Poll until the loader answers (at most 10 s).
    fn wait(loader: &mut Loader) -> Result<Opened, String> {
        let t0 = Instant::now();
        loop {
            if let Some(answer) = loader.poll() {
                return answer;
            }
            assert!(t0.elapsed() < Duration::from_secs(10), "the loader never answered");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// How long a test opener waits for its go: a `start` that ran the opener on the calling
    /// thread then fails the test after this, rather than hanging it.
    const GO_TIMEOUT: Duration = Duration::from_secs(3);

    /// `start` hands the work to a worker and returns while the opener is still blocked, and
    /// `poll` doesn't wait for it.
    #[test]
    fn startup_work_is_deferred() {
        let (go, gate) = channel::<()>();
        let t0 = Instant::now();
        let mut loader = Loader::start(move || {
            let _ = gate.recv_timeout(GO_TIMEOUT);
            (Session::new(), None)
        });
        assert!(t0.elapsed() < Duration::from_secs(1), "start returned at once, not after the opener");
        assert!(loader.poll().is_none(), "nothing yet: the opener is blocked");
        assert!(loader.poll().is_none());
        assert!(t0.elapsed() < Duration::from_secs(2), "start and poll returned at once");
        assert!(!loader.is_done());
        go.send(()).unwrap();
        assert!(wait(&mut loader).is_ok());
    }

    /// Once the opener finishes, `poll` yields its session once, then nothing.
    #[test]
    fn loader_hands_over_session() {
        let (go, gate) = channel::<()>();
        let mut loader = Loader::start(move || {
            let _ = gate.recv_timeout(GO_TIMEOUT);
            (Session::with_demo(), Some(LibraryProblem::new("/lib", "damaged")))
        });
        go.send(()).unwrap();
        let (session, problem) = wait(&mut loader).unwrap();
        assert!(!session.catalog.is_empty(), "the opener's session arrives");
        assert_eq!(problem.unwrap().error, "damaged");
        assert!(loader.is_done());
        assert!(loader.poll().is_none(), "handed over once");
    }

    /// An opener that panics ends in an error to show, not a hang or a crash of the UI thread.
    #[test]
    fn a_panicking_opener_is_an_error() {
        let mut loader = Loader::start(|| panic!("boom"));
        match wait(&mut loader) {
            Err(e) => assert_eq!(e, STOPPED),
            Ok(_) => panic!("expected an error"),
        }
        assert!(loader.poll().is_none());
    }
}

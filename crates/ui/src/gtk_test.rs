use std::panic::{self, AssertUnwindSafe};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};
use std::thread;

use relm4::gtk;

type Job = Box<dyn FnOnce() + Send>;

/// GTK binds itself to the first thread that initializes it, while libtest
/// runs tests on a thread pool, so every GTK test body goes through one
/// dedicated thread.
pub(crate) fn run(test: impl FnOnce() + Send + 'static) {
    let Some(jobs) = gtk_thread() else {
        eprintln!("no display, skipping");
        return;
    };
    let (done_tx, done_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let _ = done_tx.send(panic::catch_unwind(AssertUnwindSafe(test)));
    });
    jobs.send(job).expect("GTK test thread is gone");
    if let Err(payload) = done_rx.recv().expect("GTK test thread is gone") {
        panic::resume_unwind(payload);
    }
}

fn gtk_thread() -> Option<&'static Sender<Job>> {
    static JOBS: OnceLock<Option<Sender<Job>>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        thread::spawn(move || {
            let ready = gtk::init().is_ok();
            let _ = ready_tx.send(ready);
            if ready {
                for job in jobs_rx {
                    job();
                }
            }
        });
        ready_rx.recv().unwrap_or(false).then_some(jobs_tx)
    })
    .as_ref()
}

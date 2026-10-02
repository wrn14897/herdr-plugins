//! A small bounded worker pool for background lookups (git, screen reads,
//! transcripts). Urgent jobs jump the queue so the selected row is served first.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    closed: bool,
}

#[derive(Clone)]
pub struct Pool {
    shared: Arc<(Mutex<Queue>, Condvar)>,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool").finish_non_exhaustive()
    }
}

impl Pool {
    pub fn new(workers: usize) -> Self {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        for _ in 0..workers.max(1) {
            let shared = Arc::clone(&shared);
            thread::spawn(move || worker(&shared));
        }
        Self { shared }
    }

    /// Queues a job; `urgent` jobs run before everything already queued.
    pub fn submit(&self, urgent: bool, job: impl FnOnce() + Send + 'static) {
        let (lock, cvar) = &*self.shared;
        let mut queue = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if urgent {
            queue.jobs.push_front(Box::new(job));
        } else {
            queue.jobs.push_back(Box::new(job));
        }
        cvar.notify_one();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // Only the last handle closes the queue; idle workers then exit.
        if Arc::strong_count(&self.shared) == 1 {
            let (lock, cvar) = &*self.shared;
            if let Ok(mut queue) = lock.lock() {
                queue.closed = true;
                queue.jobs.clear();
            }
            cvar.notify_all();
        }
    }
}

fn worker(shared: &(Mutex<Queue>, Condvar)) {
    let (lock, cvar) = shared;
    loop {
        let job = {
            let mut queue = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if let Some(job) = queue.jobs.pop_front() {
                    break job;
                }
                if queue.closed {
                    return;
                }
                queue = cvar
                    .wait(queue)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        job();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn runs_all_jobs() {
        let pool = Pool::new(3);
        let (tx, rx) = channel();
        for i in 0..50 {
            let tx = tx.clone();
            pool.submit(false, move || tx.send(i).unwrap());
        }
        let mut got: Vec<i32> = (0..50)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        got.sort_unstable();
        assert_eq!(got, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn urgent_jobs_run_first() {
        let pool = Pool::new(1);
        let (gate_tx, gate_rx) = channel::<()>();
        let (tx, rx) = channel();
        // Block the single worker so the queue order is observable.
        pool.submit(false, move || gate_rx.recv().unwrap());
        for i in 0..3 {
            let tx = tx.clone();
            pool.submit(false, move || tx.send(i).unwrap());
        }
        let tx2 = tx.clone();
        pool.submit(true, move || tx2.send(99).unwrap());
        gate_tx.send(()).unwrap();
        let order: Vec<i32> = (0..4)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert_eq!(order, [99, 0, 1, 2]);
    }
}

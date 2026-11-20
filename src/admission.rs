// SPDX-License-Identifier: Apache-2.0

//! Per-stage admission: bounded active work and a bounded, timed queue.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Admission was refused because capacity could not be obtained in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Overloaded;

pub(crate) struct Admission {
    active: Arc<Semaphore>,
    waiting: AtomicUsize,
    max_queue: usize,
    queue_timeout: Duration,
}

struct Waiting<'a>(&'a AtomicUsize);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Admission {
    pub(crate) fn new(max_in_flight: usize, max_queue: usize, queue_timeout: Duration) -> Self {
        Self {
            active: Arc::new(Semaphore::new(max_in_flight)),
            waiting: AtomicUsize::new(0),
            max_queue,
            queue_timeout,
        }
    }

    /// Take an execution slot, waiting in the bounded queue for at most the
    /// queue timeout. The returned permit is held until the work finishes,
    /// including blocking work that outlives a disconnected caller.
    pub(crate) async fn admit(&self) -> Result<OwnedSemaphorePermit, Overloaded> {
        if let Ok(permit) = Arc::clone(&self.active).try_acquire_owned() {
            return Ok(permit);
        }
        let queued = self.waiting.fetch_add(1, Ordering::SeqCst);
        let _waiting = Waiting(&self.waiting);
        if queued >= self.max_queue {
            return Err(Overloaded);
        }
        match tokio::time::timeout(self.queue_timeout, Arc::clone(&self.active).acquire_owned())
            .await
        {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(Overloaded),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_immediately_when_the_queue_is_full() {
        let admission = Admission::new(1, 0, Duration::from_secs(5));
        let held = admission.admit().await.unwrap();
        assert_eq!(admission.admit().await.unwrap_err(), Overloaded);
        drop(held);
        assert!(admission.admit().await.is_ok());
    }

    #[tokio::test]
    async fn queued_work_times_out_then_the_queue_slot_is_released() {
        let admission = Admission::new(1, 1, Duration::from_millis(20));
        let _held = admission.admit().await.unwrap();
        assert_eq!(admission.admit().await.unwrap_err(), Overloaded);
        assert_eq!(admission.waiting.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn queued_work_runs_when_a_slot_frees_in_time() {
        let admission = Arc::new(Admission::new(1, 1, Duration::from_secs(5)));
        let held = admission.admit().await.unwrap();
        let waiter = tokio::spawn({
            let admission = Arc::clone(&admission);
            async move { admission.admit().await.map(drop) }
        });
        while admission.waiting.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        // A third caller finds the one queue place taken.
        assert_eq!(admission.admit().await.unwrap_err(), Overloaded);
        drop(held);
        assert_eq!(waiter.await.unwrap(), Ok(()));
    }
}

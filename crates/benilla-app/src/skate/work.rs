//! Replaceable work is bounded independently from session transitions.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::{Job, Reply};

#[derive(Default)]
struct Pending {
    control: Option<Job>,
    rebuild: Option<Job>,
    settings: Option<Job>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    pending: Mutex<Pending>,
    wake: Condvar,
}

pub(super) struct Sender(Arc<Shared>);
pub(super) struct Receiver(Arc<Shared>);

pub(super) enum Received {
    Job(Job),
    Timeout,
    Closed,
}

pub(super) fn channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared::default());
    (Sender(shared.clone()), Receiver(shared))
}

impl Sender {
    pub(super) fn send(&self, mut job: Job) -> Result<(), ()> {
        let mut pending = self.0.pending.lock().unwrap();
        if pending.closed {
            return Err(());
        }
        match &mut job {
            Job::Activate { .. } | Job::Suspend => {
                pending.control = Some(job);
                pending.rebuild = None;
                pending.settings = None;
            }
            Job::Rebuild(..) => pending.rebuild = Some(job),
            Job::Step { epoch, boosts, .. } => {
                if let Some(Job::Step {
                    epoch: previous,
                    boosts: old,
                    ..
                }) = &pending.settings
                {
                    if previous == epoch {
                        boosts.rocket |= old.rocket;
                    }
                }
                pending.settings = Some(job);
            }
        }
        self.0.wake.notify_one();
        Ok(())
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.0.pending.lock().unwrap().closed = true;
        self.0.wake.notify_one();
    }
}

impl Receiver {
    pub(super) fn recv_timeout(&self, timeout: Duration) -> Received {
        let pending = self.0.pending.lock().unwrap();
        let (mut pending, _) = self
            .0
            .wake
            .wait_timeout_while(pending, timeout, |p| {
                !p.closed && p.control.is_none() && p.settings.is_none() && p.rebuild.is_none()
            })
            .unwrap();
        if pending.closed {
            return Received::Closed;
        }
        if let Some(job) = pending
            .control
            .take()
            .or_else(|| pending.settings.take())
            .or_else(|| pending.rebuild.take())
        {
            Received::Job(job)
        } else {
            Received::Timeout
        }
    }
}

#[derive(Default)]
struct Published {
    transition: Option<Reply>,
    pose: Option<Reply>,
    closed: bool,
}

pub(super) struct Publisher(Arc<Mutex<Published>>);
pub(super) struct Replies(Arc<Mutex<Published>>);

pub(super) fn replies() -> (Publisher, Replies) {
    let shared = Arc::new(Mutex::new(Published::default()));
    (Publisher(shared.clone()), Replies(shared))
}

impl Publisher {
    pub(super) fn send(&self, reply: Reply) {
        let mut published = self.0.lock().unwrap();
        match reply {
            Reply::Pose(..) => published.pose = Some(reply),
            _ => {
                published.pose = None;
                published.transition = Some(reply);
            }
        }
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.0.lock().unwrap().closed = true;
    }
}

impl Replies {
    pub(super) fn take(&self) -> Vec<Reply> {
        let mut published = self.0.lock().unwrap();
        let mut replies = Vec::with_capacity(2);
        if let Some(transition) = published.transition.take() {
            replies.push(transition);
        }
        if let Some(pose) = published.pose.take() {
            replies.push(pose);
        }
        if published.closed && !replies.iter().any(|r| matches!(r, Reply::Error(_))) {
            replies.push(Reply::Error("the skate worker stopped".into()));
        }
        replies
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(epoch: u64, rocket: bool) -> Job {
        Job::Step {
            epoch,
            aspect: 1.0,
            boosts: crate::skate::profession::Boosts {
                rocket,
                ..Default::default()
            },
        }
    }

    #[test]
    fn coalesced_settings_keep_a_boost_exactly_once() {
        let (sender, receiver) = channel();
        sender.send(step(1, true)).unwrap();
        for _ in 0..100_000 {
            sender.send(step(1, false)).unwrap();
        }
        match receiver.recv_timeout(Duration::ZERO) {
            Received::Job(Job::Step { boosts, .. }) => assert!(boosts.rocket),
            _ => panic!("pending settings missing"),
        }
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Received::Timeout
        ));
    }

    #[test]
    fn suspension_discards_old_work() {
        let (sender, receiver) = channel();
        sender.send(step(1, true)).unwrap();
        sender.send(Job::Suspend).unwrap();
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Received::Job(Job::Suspend)
        ));
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Received::Timeout
        ));
    }

    #[test]
    fn new_session_cannot_inherit_an_old_boost() {
        let (sender, receiver) = channel();
        sender.send(step(1, true)).unwrap();
        sender.send(step(2, false)).unwrap();
        match receiver.recv_timeout(Duration::ZERO) {
            Received::Job(Job::Step { epoch, boosts, .. }) => {
                assert_eq!(epoch, 2);
                assert!(!boosts.rocket);
            }
            _ => panic!("pending settings missing"),
        }
    }
}

//! Inbound messages to the model in arrival order per chat, with slow work (media
//! downloads, voice note transcription) done off the receiving loop.
//!
//! A message takes its [`Turn`] with [`Lanes::reserve`] as soon as it arrives, then does
//! its slow work after [`Turn::wait`]. Each turn waits only for the one reserved before
//! it in the same chat: a text sent right after a voice note reaches the model after
//! that note's transcript, while other chats and the receiving loop go on. A turn ends
//! when it is dropped, whether or not its message was delivered.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Condvar, Mutex};
use tokio::sync::watch;

#[derive(Default)]
pub struct Lanes {
    lanes: Mutex<HashMap<String, Lane>>,
    /// Wakes [`Turn::wait_blocking`] when a lane moves on.
    moved: Condvar,
}

/// One chat's tickets: `serving` and everything after it are not done yet.
struct Lane {
    next: u64,
    serving: u64,
    /// Done out of order, ahead of `serving`.
    done: BTreeSet<u64>,
    tx: watch::Sender<u64>,
}

/// One message's place in its chat's lane.
pub struct Turn {
    lanes: &'static Lanes,
    key: String,
    ticket: u64,
    rx: watch::Receiver<u64>,
}

impl Lanes {
    pub fn reserve(&'static self, key: &str) -> Turn {
        let mut lanes = self.lanes.lock().unwrap();
        let lane = lanes.entry(key.to_string()).or_insert_with(|| Lane {
            next: 0,
            serving: 0,
            done: BTreeSet::new(),
            tx: watch::Sender::new(0),
        });
        let ticket = lane.next;
        lane.next += 1;
        Turn { lanes: self, key: key.to_string(), ticket, rx: lane.tx.subscribe() }
    }
}

impl Turn {
    /// Until every message reserved before this one in its chat is done.
    pub async fn wait(&mut self) {
        let ticket = self.ticket;
        let _ = self.rx.wait_for(|s| *s >= ticket).await;
    }

    /// [`Turn::wait`] for a plain thread (not inside an async runtime).
    pub fn wait_blocking(&mut self) {
        let mut lanes = self.lanes.lanes.lock().unwrap();
        while lanes.get(&self.key).is_some_and(|l| l.serving < self.ticket) {
            lanes = self.lanes.moved.wait(lanes).unwrap();
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let mut lanes = self.lanes.lanes.lock().unwrap();
        let Some(lane) = lanes.get_mut(&self.key) else { return };
        lane.done.insert(self.ticket);
        while lane.done.remove(&lane.serving) {
            lane.serving += 1;
        }
        lane.tx.send_replace(lane.serving);
        if lane.serving == lane.next {
            lanes.remove(&self.key);
        }
        self.lanes.moved.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn a_chat_keeps_its_order_and_others_do_not_wait() {
        static L: std::sync::LazyLock<Lanes> = std::sync::LazyLock::new(Lanes::default);
        let lanes = &*L;
        let got = Arc::new(Mutex::new(Vec::new()));
        let push = |what: &'static str, mut turn: Turn, slow: u64| {
            let got = got.clone();
            tokio::spawn(async move {
                // the slow work (a download) comes first, the delivery in turn
                tokio::time::sleep(Duration::from_millis(slow)).await;
                turn.wait().await;
                got.lock().unwrap().push(what);
            })
        };
        let a1 = push("a1", lanes.reserve("a"), 200);
        let skipped = lanes.reserve("a");
        let a2 = push("a2", lanes.reserve("a"), 0);
        let b1 = push("b1", lanes.reserve("b"), 0);
        b1.await.unwrap();
        assert_eq!(*got.lock().unwrap(), vec!["b1"], "b does not wait for a");
        drop(skipped);
        a1.await.unwrap();
        a2.await.unwrap();
        assert_eq!(*got.lock().unwrap(), vec!["b1", "a1", "a2"]);
        assert!(lanes.lanes.lock().unwrap().is_empty(), "finished lanes are gone");
    }

    #[test]
    fn a_dropped_turn_does_not_let_the_next_one_pass_an_earlier_one() {
        static L: std::sync::LazyLock<Lanes> = std::sync::LazyLock::new(Lanes::default);
        let first = L.reserve("a");
        let filtered = L.reserve("a");
        let mut last = L.reserve("a");
        drop(filtered);
        let t = std::thread::spawn(move || {
            last.wait_blocking();
            std::time::Instant::now()
        });
        std::thread::sleep(Duration::from_millis(100));
        let released = std::time::Instant::now();
        drop(first);
        assert!(t.join().unwrap() >= released);
    }
}

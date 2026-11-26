use std::sync::Mutex as StdMutex;

use tokio::sync::mpsc;

use super::*;
use crate::web_client::port::NextCandidate;

const ALICE: i64 = 1;
const BOB: i64 = 2;
const T0: Duration = Duration::from_secs(1000);
const WAIT: Duration = Duration::from_secs(20);

#[derive(Clone, Default)]
struct Sent(Arc<StdMutex<Vec<String>>>);

impl CandidateSink for Sent {
    fn send(&self, candidate: String) -> Result<(), CandidateRejected> {
        self.0.lock().unwrap().push(candidate);
        Ok(())
    }
}

struct Device(mpsc::UnboundedReceiver<String>);

impl CandidateSource for Device {
    fn next(&mut self) -> NextCandidate<'_> {
        Box::pin(self.0.recv())
    }
}

fn open(table: &Table, user: i64, id: &str) -> (Sent, mpsc::UnboundedSender<String>) {
    let sent = Sent::default();
    let (device, rx) = mpsc::unbounded_channel();
    let reservation = table.reserve(user, T0).unwrap();
    let new = NewSession {
        device_id: "123456789".to_owned(),
        sender: Box::new(sent.clone()),
        receiver: Box::new(Device(rx)),
    };
    table.insert(reservation, id.to_owned(), new, T0);
    (sent, device)
}

fn page(candidates: &[&str], next: usize) -> Result<Candidates, Ended> {
    Ok(Candidates {
        candidates: candidates.iter().map(|c| (*c).to_owned()).collect(),
        next,
    })
}

#[test]
fn a_user_holds_at_most_four_sessions_punches_in_flight_included() {
    let table = Table::default();
    let _a = open(&table, ALICE, "a");
    let _b = open(&table, ALICE, "b");
    let _c = open(&table, ALICE, "c");

    let in_flight = table.reserve(ALICE, T0).unwrap();
    let fifth = table.reserve(ALICE, T0).is_none();
    let bob = table.reserve(BOB, T0).is_some();
    drop(in_flight);
    let after_release = table.reserve(ALICE, T0).is_some();

    assert!(fifth, "four held, the fifth is refused");
    assert!(bob, "the cap is per user");
    assert!(after_release, "a failed punch gives its slot back");
}

#[test]
fn sessions_belong_to_their_owner_and_expire() {
    let table = Table::default();
    let _a = open(&table, ALICE, "a");
    let _b = open(&table, ALICE, "b");
    let _c = open(&table, ALICE, "c");
    let _d = open(&table, ALICE, "d");
    let expired = T0 + SESSION_TTL;

    let own = table.get(ALICE, "a", T0).is_some();
    let other = table.get(BOB, "a", T0).is_none();
    let removed_by_other = table.remove(BOB, "a", T0).is_none();
    let gone = table.get(ALICE, "a", expired).is_none();
    let slot_back = table.reserve(ALICE, expired).is_some();

    assert!(own && other && removed_by_other);
    assert!(gone, "expired after {SESSION_TTL:?}");
    assert!(slot_back, "expired sessions leave the cap");
}

#[tokio::test]
async fn the_long_poll_pages_the_device_candidates_by_index() {
    let table = Table::default();
    let (_, device) = open(&table, ALICE, "a");
    let session = table.get(ALICE, "a", T0).unwrap();
    device.send("c0".to_owned()).unwrap();

    let first = session.poll(0, WAIT).await;
    let waiting = tokio::spawn({
        let session = Arc::clone(&session);
        async move { session.poll(1, WAIT).await }
    });
    tokio::task::yield_now().await;
    device.send("c1".to_owned()).unwrap();
    let second = waiting.await.unwrap();
    let again = session.poll(0, WAIT).await;
    let beyond = session.poll(9, Duration::ZERO).await;

    assert_eq!(first, page(&["c0"], 1));
    assert_eq!(second, page(&["c1"], 2));
    assert_eq!(
        again,
        page(&["c0", "c1"], 2),
        "a lost reply can be read again"
    );
    assert_eq!(beyond, page(&[], 2));
}

#[tokio::test]
async fn a_long_poll_without_candidates_ends_empty_after_its_wait() {
    let table = Table::default();
    let (_, _device) = open(&table, ALICE, "a");
    let session = table.get(ALICE, "a", T0).unwrap();
    let wait = Duration::from_millis(50);

    let started = tokio::time::Instant::now();
    let empty = session.poll(0, wait).await;

    assert_eq!(empty, page(&[], 0));
    assert!(started.elapsed() >= wait);
}

#[tokio::test]
async fn sending_is_not_held_up_by_a_waiting_poll() {
    let table = Table::default();
    let (sent, _device) = open(&table, ALICE, "a");
    let session = table.get(ALICE, "a", T0).unwrap();
    let waiting = tokio::spawn({
        let session = Arc::clone(&session);
        async move { session.poll(0, WAIT).await }
    });
    tokio::task::yield_now().await;

    let result = session.send("browser-c0".to_owned());

    assert_eq!(result, Ok(()));
    assert_eq!(*sent.0.lock().unwrap(), ["browser-c0"]);
    assert!(!waiting.is_finished());
    waiting.abort();
}

#[tokio::test]
async fn a_closed_route_ends_the_poll_once_everything_was_read() {
    let table = Table::default();
    let (_, device) = open(&table, ALICE, "a");
    let session = table.get(ALICE, "a", T0).unwrap();
    device.send("c0".to_owned()).unwrap();
    drop(device);

    let first = session.poll(0, WAIT).await;
    let ended = session.poll(1, WAIT).await;
    let reread = session.poll(0, WAIT).await;

    assert_eq!(first, page(&["c0"], 1));
    assert_eq!(ended, Err(Ended));
    assert_eq!(reread, page(&["c0"], 1));
}

#[tokio::test]
async fn closing_a_session_wakes_its_waiting_poll() {
    let table = Table::default();
    let (_, _device) = open(&table, ALICE, "a");
    let session = table.get(ALICE, "a", T0).unwrap();
    let waiting = tokio::spawn({
        let session = Arc::clone(&session);
        async move { session.poll(0, WAIT).await }
    });
    tokio::task::yield_now().await;

    let removed = table.remove(ALICE, "a", T0);
    let woken = tokio::time::timeout(Duration::from_secs(5), waiting).await;

    assert!(removed.is_some());
    assert_eq!(woken.unwrap().unwrap(), Err(Ended));
    assert!(table.get(ALICE, "a", T0).is_none());
}

//! The device presence table is fed from these notifications, so they must follow the server's
//! own notion of online.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{
    AllowAll, Authz, ConnectReq, ControlPlane, HttpProxyRequest, HttpProxyResponse, RegisterInfo,
    RegisterVerdict,
};
use hbb_common::{
    rendezvous_proto::register_pk_response::Result::OK,
    tokio::{
        self,
        time::{Duration, Instant, sleep},
    },
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
};
use support::{PeerKeys, Rendezvous, STEP, TestServer};

const POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Online(String, IpAddr),
    Offline(String),
}

#[derive(Default)]
struct Recorder {
    inner: AllowAll,
    events: Mutex<Vec<Event>>,
}

impl Recorder {
    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    fn online_count(&self, id: &str) -> usize {
        let is_online = |e: &&Event| matches!(e, Event::Online(i, _) if i == id);
        self.events().iter().filter(is_online).count()
    }

    async fn wait(&self, what: &str, cond: impl Fn(&[Event]) -> bool) {
        let deadline = Instant::now() + STEP;
        while !cond(&self.events()) {
            assert!(Instant::now() < deadline, "{what}: {:?}", self.events());
            sleep(POLL).await;
        }
    }
}

#[async_trait::async_trait]
impl ControlPlane for Recorder {
    fn register_pk(&self, r: &RegisterInfo<'_>) -> RegisterVerdict {
        self.inner.register_pk(r)
    }
    async fn authorize(&self, r: &ConnectReq<'_>) -> Authz {
        self.inner.authorize(r).await
    }
    async fn http_proxy(&self, from: SocketAddr, req: HttpProxyRequest) -> HttpProxyResponse {
        self.inner.http_proxy(from, req).await
    }
    async fn health_check(&self, token: &str, from: SocketAddr) -> Option<i32> {
        self.inner.health_check(token, from).await
    }
    fn pick_relay(&self, a: IpAddr, b: IpAddr, relays: &[String]) -> Option<String> {
        self.inner.pick_relay(a, b, relays)
    }
    fn peer_online(&self, id: &str, ip: IpAddr) {
        let event = Event::Online(id.to_owned(), ip);
        self.events.lock().unwrap().push(event);
    }
    fn peer_offline(&self, id: &str) {
        self.events
            .lock()
            .unwrap()
            .push(Event::Offline(id.to_owned()));
    }
}

async fn start() -> (TestServer, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let control: Arc<dyn ControlPlane> = recorder.clone();
    let server = TestServer::start_with_control(|cfg| cfg, control).await;
    (server, recorder)
}

#[tokio::test]
async fn udp_registration_and_keep_alives_report_the_peer_online() {
    const ID: &str = "events-udp";
    let (server, recorder) = start().await;
    let mut peer = server.udp().await;
    let ip = peer.local_addr().ip();

    let registered = peer.register_pk(ID, &PeerKeys::random()).await;
    let after_register = recorder.online_count(ID);
    peer.register_peer(ID).await;
    peer.register_peer(ID).await;

    assert_eq!(registered, OK);
    assert_eq!(after_register, 1, "{:?}", recorder.events());
    assert_eq!(recorder.online_count(ID), 3, "{:?}", recorder.events());
    assert!(
        recorder
            .events()
            .iter()
            .all(|e| *e == Event::Online(ID.to_owned(), ip))
    );
    server.stop().await;
}

#[tokio::test]
async fn register_peer_without_a_key_reports_nothing() {
    const ID: &str = "events-unknown";
    let (server, recorder) = start().await;
    let mut peer = server.udp().await;

    let first = peer.register_peer(ID).await;
    let second = peer.register_peer(ID).await;

    assert!(first.request_pk && second.request_pk);
    assert!(recorder.events().is_empty(), "{:?}", recorder.events());
    server.stop().await;
}

#[tokio::test]
async fn tcp_channel_reports_online_and_offline_on_close() {
    const ID: &str = "events-tcp";
    let (server, recorder) = start().await;
    let mut peer = server.tcp().await;
    let ip = peer.local_addr().ip();

    let registered = peer.register_pk(ID, &PeerKeys::random()).await;
    let online = Event::Online(ID.to_owned(), ip);
    recorder
        .wait("online after RegisterPk", |e| e.contains(&online))
        .await;
    drop(peer);

    assert_eq!(registered, OK);
    let offline = Event::Offline(ID.to_owned());
    recorder
        .wait("offline after close", |e| e.last() == Some(&offline))
        .await;
    server.stop().await;
}

#[tokio::test]
async fn closing_a_replaced_channel_reports_nothing() {
    const ID: &str = "events-moved";
    let (server, recorder) = start().await;
    let keys = PeerKeys::random();
    let mut stream = server.tcp().await;
    assert_eq!(stream.register_pk(ID, &keys).await, OK);
    let mut udp = server.udp().await;
    assert_eq!(udp.register_pk(ID, &keys).await, OK);
    assert!(!udp.register_peer(ID).await.request_pk);

    drop(stream);
    sleep(Duration::from_millis(300)).await;
    udp.register_peer(ID).await;

    let events = recorder.events();
    assert!(
        !events.contains(&Event::Offline(ID.to_owned())),
        "{events:?}"
    );
    server.stop().await;
}

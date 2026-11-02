//! `SignalPeerStore`: field mapping on both backends, and a real signal server persisting
//! registrations to SQLite across a restart.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use deskhaven_core::devices::DeviceRepository;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::{DbOptions, connect, migrate};
use deskhaven_signal::{
    PeerRecord, PeerStore, PeerUpsert, SignalConfig, SignalError, SignalServer, StoreError,
};
use hbb_common::protobuf::Message as _;
use hbb_common::rendezvous_proto::{
    RegisterPk, RendezvousMessage, register_pk_response, rendezvous_message,
};
use sea_orm::DatabaseConnection;
use sodiumoxide::crypto::sign;
use tokio::net::{TcpStream, UdpSocket};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use super::SignalPeerStore;

const ID: &str = "123456789";
const OWNER_UUID: &[u8] = b"owner-uuid";
/// RegisterPk carries 32-byte ed25519 public keys (anything else is dropped).
const PK1: &[u8] = &[1; 32];
const PK2: &[u8] = &[2; 32];
const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const STEP: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
const START_ATTEMPTS: usize = 10;

fn store(conn: &DatabaseConnection) -> Arc<dyn PeerStore> {
    Arc::new(SignalPeerStore::new(DeviceRepository::new(conn.clone())))
}

fn upsert(uuid: &[u8], pk: &[u8], ip_last: u8) -> PeerUpsert {
    PeerUpsert {
        id: ID.to_owned(),
        uuid: uuid.to_vec(),
        pk: pk.to_vec(),
        ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, ip_last)),
        registered_at: SystemTime::now(),
    }
}

#[tokio::test]
async fn upsert_then_get_round_trips_through_devices() {
    each_migrated_backend(async |db: &TestDb| {
        let store = store(&db.conn);
        assert!(store.get(ID).await.unwrap().is_none(), "{}", db.backend);

        store.upsert_pk(&upsert(OWNER_UUID, PK1, 7)).await.unwrap();
        store.upsert_pk(&upsert(OWNER_UUID, PK2, 8)).await.unwrap();

        let rec = store.get(ID).await.unwrap().unwrap();
        let b = db.backend;
        assert_eq!(
            (rec.uuid, rec.pk),
            (OWNER_UUID.to_vec(), PK2.to_vec()),
            "{b}"
        );
        assert_eq!(rec.ip, Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 8))), "{b}");
    })
    .await;
}

#[tokio::test]
async fn load_page_returns_devices_as_peer_records_after_the_cursor() {
    each_migrated_backend(async |db: &TestDb| {
        let store = store(&db.conn);
        assert!(
            store.load_page(None, 10).await.unwrap().is_empty(),
            "{}",
            db.backend
        );
        store.upsert_pk(&upsert(OWNER_UUID, PK1, 7)).await.unwrap();

        let all = store.load_page(None, 10).await.unwrap();
        let after_it = store.load_page(Some(ID), 10).await.unwrap();

        let expected = PeerRecord {
            id: ID.to_owned(),
            uuid: OWNER_UUID.to_vec(),
            pk: PK1.to_vec(),
            ip: Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))),
        };
        assert_eq!(all, vec![expected], "{}", db.backend);
        assert!(after_it.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn other_uuid_maps_to_uuid_mismatch() {
    each_migrated_backend(async |db: &TestDb| {
        let store = store(&db.conn);
        store.upsert_pk(&upsert(OWNER_UUID, PK1, 7)).await.unwrap();

        let res = store.upsert_pk(&upsert(b"thief", PK2, 9)).await;

        assert!(
            matches!(&res, Err(StoreError::UuidMismatch { id }) if id == ID),
            "{}: {res:?}",
            db.backend
        );
    })
    .await;
}

/// A signal server on free loopback ports.
struct Server {
    addr: SocketAddr,
    shutdown: CancellationToken,
    task: JoinHandle<Result<(), SignalError>>,
}

impl Server {
    async fn start(key: &sign::SecretKey, store: &Arc<dyn PeerStore>) -> Self {
        for _ in 0..START_ATTEMPTS {
            let port = free_base_port();
            let cfg = SignalConfig {
                bind: Some(LOCALHOST),
                port,
                ..SignalConfig::new(key.clone())
            };
            let shutdown = CancellationToken::new();
            let server = SignalServer::new(cfg, store.clone());
            let task = tokio::spawn(server.run(shutdown.clone()));
            let addr = SocketAddr::new(LOCALHOST, port);
            if wait_until_listening(addr, &task).await {
                return Self {
                    addr,
                    shutdown,
                    task,
                };
            }
        }
        panic!("could not start the signal server");
    }

    /// Returns once `run` has returned, i.e. the store writer has flushed.
    async fn stop(self) {
        self.shutdown.cancel();
        let res = timeout(STEP, self.task).await.unwrap().unwrap();
        assert!(res.is_ok(), "{res:?}");
    }
}

/// The NAT test port is `port - 1`, the websocket port `port + 2`.
fn free_base_port() -> u16 {
    loop {
        let port = std::net::TcpListener::bind((LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let free = (3..=65533).contains(&port)
            && [port - 1, port, port + 2]
                .iter()
                .all(|p| std::net::TcpListener::bind((LOCALHOST, *p)).is_ok())
            && std::net::UdpSocket::bind((LOCALHOST, port)).is_ok();
        if free {
            return port;
        }
    }
}

/// `false` if `run` ended first (lost a port race). The UDP socket is bound before the TCP
/// listeners, so an accepting main TCP port means UDP is ready too.
async fn wait_until_listening(
    addr: SocketAddr,
    task: &JoinHandle<Result<(), SignalError>>,
) -> bool {
    let deadline = tokio::time::Instant::now() + STEP;
    while tokio::time::Instant::now() < deadline {
        if task.is_finished() {
            return false;
        }
        if TcpStream::connect(addr).await.is_ok() {
            return true;
        }
        sleep(POLL).await;
    }
    panic!("signal server on {addr} did not start listening");
}

async fn register_pk(server: SocketAddr, uuid: &[u8], pk: &[u8]) -> register_pk_response::Result {
    let socket = UdpSocket::bind((LOCALHOST, 0)).await.unwrap();
    let mut msg = RendezvousMessage::new();
    msg.set_register_pk(RegisterPk {
        id: ID.to_owned(),
        uuid: uuid.to_vec().into(),
        pk: pk.to_vec().into(),
        ..Default::default()
    });
    socket
        .send_to(&msg.write_to_bytes().unwrap(), server)
        .await
        .unwrap();
    let mut buf = vec![0u8; 1024];
    let (n, _) = timeout(STEP, socket.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    match RendezvousMessage::parse_from_bytes(&buf[..n])
        .unwrap()
        .union
    {
        Some(rendezvous_message::Union::RegisterPkResponse(res)) => {
            res.result.enum_value().unwrap()
        }
        other => panic!("expected RegisterPkResponse, got {other:?}"),
    }
}

/// The acceptance check of M1-05: a restart (new server, new connection pool, same SQLite
/// file) keeps the id bound to its uuid, so another uuid still gets UUID_MISMATCH.
#[tokio::test]
async fn registration_survives_restart_on_the_same_database() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("deskhaven.db").display());
    let options = DbOptions::new(url);
    let key = sign::gen_keypair().1;
    let conn = connect(&options).await.unwrap();
    migrate(&conn).await.unwrap();
    let server = Server::start(&key, &store(&conn)).await;
    let first = register_pk(server.addr, OWNER_UUID, PK1).await;
    server.stop().await;
    conn.close().await.unwrap();

    // Act
    let conn = connect(&options).await.unwrap();
    let server = Server::start(&key, &store(&conn)).await;
    let thief = register_pk(server.addr, b"thief-uuid", PK2).await;
    let owner = register_pk(server.addr, OWNER_UUID, PK1).await;
    server.stop().await;

    // Assert
    assert_eq!(first, register_pk_response::Result::OK);
    assert_eq!(thief, register_pk_response::Result::UUID_MISMATCH);
    assert_eq!(owner, register_pk_response::Result::OK);
    let device = DeviceRepository::new(conn)
        .find_by_rustdesk_id(ID)
        .await
        .unwrap()
        .expect("devices row");
    assert_eq!(
        (device.uuid, device.pk),
        (OWNER_UUID.to_vec(), PK1.to_vec())
    );
    assert_eq!(device.register_ip, "127.0.0.1");
}

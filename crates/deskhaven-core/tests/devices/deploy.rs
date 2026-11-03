//! `--deploy` (M2-19, 03 §2.8): off unless `devices.require_deploy`; binds an id to the
//! device's uuid and marks it deployed, refusing ids that another uuid holds, with an audit
//! row in the same transaction.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::console_audit::Actor;
use deskhaven_core::devices::{DeployOutcome, DeployRequest, Requester};
use deskhaven_db::entity::console_audit::{self, ActorKind};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;

use super::support::{Ctx, ID, b64, ctx, uuid_of};

pub const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9));
pub const PK: [u8; 32] = [5; 32];
pub const TOKEN: i64 = 7;

pub fn requester(ip: IpAddr) -> Requester {
    Requester {
        actor: Actor::ApiToken(TOKEN),
        client: ClientAddr {
            addr: ip,
            is_proxy_fallback: false,
        },
    }
}

pub fn request(rustdesk_id: &str, uuid: &[u8], pk: &[u8]) -> DeployRequest {
    DeployRequest {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: b64(uuid),
        pk: b64(pk),
        by: requester(IP),
    }
}

impl Ctx {
    pub async fn require_deploy(&self) {
        self.set("devices.require_deploy", serde_json::json!(true))
            .await;
    }

    pub async fn deploy(&self, req: DeployRequest) -> DeployOutcome {
        self.devices.deploy(req).await.unwrap()
    }

    pub async fn audit_rows(&self) -> Vec<console_audit::Model> {
        console_audit::Entity::find().all(&self.conn).await.unwrap()
    }
}

#[tokio::test]
async fn deploy_is_not_enabled_by_default() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;

        let outcome = c.deploy(request(ID, &uuid_of(ID), &PK)).await;

        assert_eq!(outcome, DeployOutcome::NotEnabled, "{}", db.backend);
        let repo = deskhaven_core::devices::DeviceRepository::new(c.conn.clone());
        assert!(repo.find_by_rustdesk_id(ID).await.unwrap().is_none());
        assert!(c.audit_rows().await.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn deploying_a_new_id_registers_it_as_deployed() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.require_deploy().await;

        // Act
        let outcome = c.deploy(request(ID, &uuid_of(ID), &PK)).await;

        // Assert
        let b = db.backend;
        assert_eq!(outcome, DeployOutcome::Deployed, "{b}");
        let row = c.row(ID).await;
        assert_eq!((row.uuid, row.pk), (uuid_of(ID), PK.to_vec()), "{b}");
        assert!(row.deployed_at.is_some(), "{b}");
        assert_eq!(row.register_ip, IP.to_string(), "{b}");
        assert_eq!((row.owner_user_id, row.sysinfo_hash), (None, None), "{b}");
        let verified = c.devices.verify(ID, &b64(&uuid_of(ID))).await.unwrap();
        assert_eq!(verified, Some(row.id), "{b}: the device endpoints know it");
        let audit = c.audit_rows().await;
        assert_eq!(audit.len(), 1, "{b}");
        let entry = &audit[0];
        assert_eq!(
            (entry.actor_kind, entry.actor_id),
            (ActorKind::ApiToken, Some(TOKEN))
        );
        assert_eq!(
            (entry.action.as_str(), entry.target_id.as_str()),
            ("device.deploy", ID)
        );
        let after = entry.after.clone().unwrap_or_default();
        assert!(after.contains("\"registered\":true"), "{b}: {after}");
        assert_eq!(entry.ip.as_deref(), Some("198.51.100.9"), "{b}");
    })
    .await;
}

#[tokio::test]
async fn deploying_a_registered_device_marks_it_and_keeps_its_key() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        c.require_deploy().await;

        // Act
        let first = c.deploy(request(ID, &uuid_of(ID), &PK)).await;
        let again = c.deploy(request(ID, &uuid_of(ID), &PK)).await;

        // Assert
        let b = db.backend;
        assert_eq!(
            (first, again),
            (DeployOutcome::Deployed, DeployOutcome::Deployed)
        );
        let row = c.row(ID).await;
        assert_eq!(row.id, device_id, "{b}");
        assert!(row.deployed_at.is_some(), "{b}");
        assert_eq!(row.pk, vec![9; 32], "{b}: the registration owns the key");
        let audit = c.audit_rows().await;
        assert_eq!(audit.len(), 2, "{b}");
        let after = audit[0].after.clone().unwrap_or_default();
        assert!(after.contains("\"registered\":false"), "{b}: {after}");
    })
    .await;
}

#[tokio::test]
async fn an_id_held_by_another_uuid_is_taken() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.require_deploy().await;

        let outcome = c.deploy(request(ID, b"another-machine", &PK)).await;

        assert_eq!(outcome, DeployOutcome::IdTaken, "{}", db.backend);
        let row = c.row(ID).await;
        assert_eq!((row.uuid, row.deployed_at), (uuid_of(ID), None));
        assert!(c.audit_rows().await.is_empty(), "{}", db.backend);
    })
    .await;
}

/// The uuid is compared even with `devices.verify_uuid` off: deploy tokens travel in
/// scripts, so they are no proof of which machine is asking.
#[tokio::test]
async fn the_uuid_is_checked_even_with_uuid_verification_off() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.device(ID).await;
        c.require_deploy().await;
        c.set("devices.verify_uuid", serde_json::json!(false)).await;

        let outcome = c.deploy(request(ID, b"another-machine", &PK)).await;

        assert_eq!(outcome, DeployOutcome::IdTaken, "{}", db.backend);
        assert_eq!(c.row(ID).await.deployed_at, None, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn malformed_ids_uuids_and_keys_are_invalid_input() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        c.require_deploy().await;
        let uuid = uuid_of(ID);
        let bad_b64 = |id: &str, uuid: &str, pk: &str| DeployRequest {
            rustdesk_id: id.to_owned(),
            uuid: uuid.to_owned(),
            pk: pk.to_owned(),
            by: requester(IP),
        };
        let cases = [
            request("", &uuid, &PK),
            request("12345", &uuid, &PK),
            request("12345 678", &uuid, &PK),
            request(&"1".repeat(254), &uuid, &PK),
            request(ID, b"", &PK),
            request(ID, &[1; 129], &PK),
            request(ID, &uuid, &[5; 31]),
            request(ID, &uuid, &[5; 33]),
            bad_b64(ID, "not base64!", &b64(&PK)),
            bad_b64(ID, &b64(&uuid), "not base64!"),
        ];

        for req in cases {
            let shown = format!("{:?}", req.rustdesk_id);
            let outcome = c.deploy(req).await;
            assert_eq!(
                outcome,
                DeployOutcome::InvalidInput,
                "{}: {shown}",
                db.backend
            );
        }
        let repo = deskhaven_core::devices::DeviceRepository::new(c.conn.clone());
        assert!(repo.find_by_rustdesk_id(ID).await.unwrap().is_none());
    })
    .await;
}

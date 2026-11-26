//! A name resolving only to forbidden addresses fails; mixed answers keep the rest. Loopback is not
//! filtered: `url_policy` limits it to loopback issuers.

use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

pub type LookupFuture = Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>>;
pub type Lookup = Arc<dyn Fn(String) -> LookupFuture + Send + Sync>;

const ALIBABA_METADATA: Ipv4Addr = Ipv4Addr::new(100, 100, 100, 200);
const AWS_METADATA_V6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254);

pub fn system_lookup() -> Lookup {
    Arc::new(|host: String| {
        Box::pin(async move { Ok(tokio::net::lookup_host((host.as_str(), 0)).await?.collect()) })
    })
}

pub(crate) fn is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local() || v4 == ALIBABA_METADATA,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_forbidden(IpAddr::V4(v4)),
            None => v6.is_unicast_link_local() || v6 == AWS_METADATA_V6,
        },
    }
}

pub(crate) struct FilteringResolver {
    lookup: Lookup,
}

impl FilteringResolver {
    pub(crate) fn new(lookup: Lookup) -> Self {
        Self { lookup }
    }
}

impl Resolve for FilteringResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let lookup = Arc::clone(&self.lookup);
        Box::pin(async move {
            let allowed = permitted(lookup(host).await?)?;
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

fn permitted(addrs: Vec<SocketAddr>) -> io::Result<Vec<SocketAddr>> {
    let allowed: Vec<SocketAddr> = addrs
        .into_iter()
        .filter(|addr| !is_forbidden(addr.ip()))
        .collect();
    if allowed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the host resolves only to link-local or metadata addresses",
        ));
    }
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 0)
    }

    #[test]
    fn link_local_and_metadata_addresses_are_forbidden() {
        for bad in [
            "169.254.169.254",
            "169.254.0.1",
            "100.100.100.200",
            "fe80::1",
            "febf::1",
            "fd00:ec2::254",
            "::ffff:169.254.169.254",
            "::ffff:100.100.100.200",
        ] {
            assert!(is_forbidden(bad.parse().unwrap()), "{bad}");
        }
        for ok in [
            "127.0.0.1",
            "::1",
            "10.0.0.1",
            "93.184.216.34",
            "2001:db8::1",
            "100.100.100.201",
        ] {
            assert!(!is_forbidden(ok.parse().unwrap()), "{ok}");
        }
    }

    #[test]
    fn only_forbidden_answers_fail_and_mixed_answers_are_filtered() {
        assert!(permitted(vec![addr("169.254.169.254"), addr("fd00:ec2::254")]).is_err());
        assert!(permitted(Vec::new()).is_err());
        assert_eq!(
            permitted(vec![addr("169.254.169.254"), addr("93.184.216.34")]).unwrap(),
            vec![addr("93.184.216.34")]
        );
    }
}

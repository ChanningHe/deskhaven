//! Where a directory connection goes: the URL's host resolved here, not by ldap3, and
//! refused when **any** address it resolves to is link-local or a cloud metadata address
//! (`oidc::is_forbidden`, the IdP policy). The TCP connection is made to a vetted address;
//! ldap3 then runs TLS over it with the URL's host name, so SNI and certificate
//! verification still check the name. Applies to logins and diagnostics alike. IP-literal
//! hosts skip DNS and get the same check (`validate` refuses them when written already).

use std::io;
use std::net::{IpAddr, SocketAddr};

use tokio::net::TcpStream;
use url::{Host, Url};

use super::directory::DirectoryFault;
use crate::oidc::{Lookup, is_forbidden};

const LDAP_PORT: u16 = 389;
const LDAPS_PORT: u16 = 636;

/// A TCP connection to the directory `url` names, made to a vetted address.
pub(crate) async fn connect(lookup: &Lookup, url: &str) -> Result<TcpStream, DirectoryFault> {
    let parsed = Url::parse(url).map_err(|_| unreachable("the directory URL is invalid"))?;
    let port = parsed.port().unwrap_or(match parsed.scheme() {
        "ldaps" => LDAPS_PORT,
        _ => LDAP_PORT,
    });
    let addrs = vetted(lookup, &parsed).await?;
    let mut last = None;
    for ip in addrs {
        match TcpStream::connect(SocketAddr::new(ip, port)).await {
            Ok(stream) => return Ok(stream),
            Err(err) => last = Some(err),
        }
    }
    let err = last.unwrap_or_else(|| io::Error::other("no address to connect to"));
    Err(unreachable(&err.to_string()))
}

/// The addresses of the URL's host; refused when one of them is forbidden.
async fn vetted(lookup: &Lookup, url: &Url) -> Result<Vec<IpAddr>, DirectoryFault> {
    let addrs = match url.host() {
        Some(Host::Ipv4(ip)) => vec![IpAddr::V4(ip)],
        Some(Host::Ipv6(ip)) => vec![IpAddr::V6(ip)],
        // `ldap:` is not a special scheme: IPv4 literals arrive as domain strings.
        Some(Host::Domain(name)) => match name.parse::<IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => resolve(lookup, name).await?,
        },
        None => return Err(unreachable("the directory URL names no host")),
    };
    if addrs.is_empty() {
        return Err(unreachable("the directory host name does not resolve"));
    }
    if addrs.iter().any(|ip| is_forbidden(*ip)) {
        tracing::warn!(host = ?url.host_str(), "directory host resolves to a forbidden address; refused");
        return Err(unreachable(
            "the directory host resolves to a link-local or cloud metadata address",
        ));
    }
    Ok(addrs)
}

async fn resolve(lookup: &Lookup, name: &str) -> Result<Vec<IpAddr>, DirectoryFault> {
    let found = lookup(name.to_owned())
        .await
        .map_err(|err| unreachable(&format!("cannot resolve {name}: {err}")))?;
    let mut ips: Vec<IpAddr> = found.iter().map(SocketAddr::ip).collect();
    ips.dedup();
    Ok(ips)
}

fn unreachable(message: &str) -> DirectoryFault {
    DirectoryFault::Unreachable(message.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::net::TcpListener;

    use super::*;

    /// A lookup answering `ips` for every name, recording the names asked.
    fn answering(ips: &[&str]) -> (Lookup, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&asked);
        let addrs: Vec<SocketAddr> = ips
            .iter()
            .map(|ip| SocketAddr::new(ip.parse().unwrap(), 0))
            .collect();
        let lookup: Lookup = Arc::new(move |name: String| {
            seen.lock().unwrap().push(name);
            let addrs = addrs.clone();
            Box::pin(async move { Ok(addrs) })
        });
        (lookup, asked)
    }

    #[tokio::test]
    async fn a_name_resolving_to_a_metadata_address_is_refused_even_among_others() {
        for answer in [
            &["169.254.169.254"][..],
            &["127.0.0.1", "169.254.169.254"],
            &["::ffff:100.100.100.200"],
        ] {
            let (lookup, asked) = answering(answer);

            let refused = connect(&lookup, "ldaps://evil.example.org").await;

            let Err(DirectoryFault::Unreachable(message)) = refused else {
                panic!("{answer:?}: {refused:?}");
            };
            assert!(message.contains("link-local"), "{message}");
            assert_eq!(*asked.lock().unwrap(), ["evil.example.org"]);
        }
    }

    #[tokio::test]
    async fn the_connection_goes_to_the_vetted_address() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (lookup, asked) = answering(&["127.0.0.1"]);

        let stream = connect(&lookup, &format!("ldap://ldap.internal.test:{port}")).await;

        let stream = stream.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        assert_eq!(stream.peer_addr().unwrap(), accepted.local_addr().unwrap());
        assert_eq!(*asked.lock().unwrap(), ["ldap.internal.test"]);
    }

    #[tokio::test]
    async fn ip_literals_skip_the_lookup_and_are_checked_too() {
        let (lookup, asked) = answering(&["127.0.0.1"]);

        let refused = connect(&lookup, "ldap://169.254.169.254:389").await;

        assert!(matches!(refused, Err(DirectoryFault::Unreachable(_))));
        assert!(asked.lock().unwrap().is_empty());
    }
}

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use ipnet::IpNet;

/// An IPv4 or IPv6 network in CIDR notation, e.g. `"10.0.0.0/8"`; a bare
/// address means a single host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cidr(IpNet);

impl Cidr {
    pub fn contains(&self, addr: IpAddr) -> bool {
        self.0.contains(&addr)
    }

    pub const fn net(&self) -> IpNet {
        self.0
    }
}

/// Parse failure for [`Cidr`].
#[derive(Debug, thiserror::Error)]
#[error("invalid CIDR {input:?} (expected e.g. \"10.0.0.0/8\" or \"fd00::/8\")")]
pub struct CidrError {
    input: String,
}

impl FromStr for Cidr {
    type Err = CidrError;

    /// A bare address is a single host (`/32` or `/128`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        trimmed
            .parse::<IpNet>()
            .or_else(|_| trimmed.parse::<IpAddr>().map(IpNet::from))
            .map(Self)
            .map_err(|_| CidrError {
                input: s.to_owned(),
            })
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

super::string_serde!(Cidr, "a CIDR string, e.g. \"10.0.0.0/8\"");
super::string_schema!(Cidr, "Cidr", "IP network in CIDR notation", "10.0.0.0/8");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_v4_and_v6() {
        let v4: Cidr = "10.0.0.0/8".parse().unwrap();
        assert!(v4.contains("10.1.2.3".parse().unwrap()));
        assert!(!v4.contains("11.0.0.1".parse().unwrap()));
        let v6: Cidr = "fd00::/8".parse().unwrap();
        assert!(v6.contains("fd12::1".parse().unwrap()));
    }

    #[test]
    fn bare_ip_is_a_host_network() {
        assert_eq!(
            "10.0.0.1".parse::<Cidr>().unwrap().to_string(),
            "10.0.0.1/32"
        );
        assert_eq!(
            " fd00::1 ".parse::<Cidr>().unwrap().to_string(),
            "fd00::1/128"
        );
    }

    #[test]
    fn rejects_bad_prefix_and_garbage() {
        assert!("10.0.0.0/33".parse::<Cidr>().is_err());
        assert!("not-an-ip".parse::<Cidr>().is_err());
    }
}

//! hbb_common's `AddrMangle::decode` overflows on client-chosen bytes (a panic with overflow
//! checks on, a garbage address without); [`decode`] refuses those and otherwise matches it.

use std::net::SocketAddr;

use hbb_common::AddrMangle;

/// Longest IPv4 encoding; IPv6 ones are longer and involve no arithmetic.
const MAX_V4_LEN: usize = 16;
const TM_MASK: u128 = u32::MAX as u128;
const PORT_FIELD: u128 = 0xFF_FFFF;
const TM_PORT_MASK: u128 = 0xFFFF;

pub(crate) fn decode(bytes: &[u8]) -> Option<SocketAddr> {
    if bytes.len() <= MAX_V4_LEN {
        let mut padded = [0u8; MAX_V4_LEN];
        padded[..bytes.len()].copy_from_slice(bytes);
        let number = u128::from_le_bytes(padded);
        let tm = (number >> 17) & TM_MASK;
        (number >> 49).checked_sub(tm)?;
        (number & PORT_FIELD).checked_sub(tm & TM_PORT_MASK)?;
    }
    Some(AddrMangle::decode(bytes))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn what_the_server_encodes_decodes_unchanged() {
        for addr in ["203.0.113.7:50000", "127.0.0.1:1", "[2001:db8::1]:21116"] {
            let addr: SocketAddr = addr.parse().unwrap();
            let mangled = AddrMangle::encode(addr);

            assert_eq!(decode(&mangled), Some(AddrMangle::decode(&mangled)));
            assert_eq!(decode(&mangled), Some(addr));
        }
    }

    #[test]
    fn bytes_that_would_overflow_are_refused() {
        assert_eq!(decode(&[1, 2, 3, 4, 5, 6]), None);
    }
}

//! Checked decoding of client-supplied mangled addresses (deskhaven code; M4-05 fuzzing).
//!
//! hbb_common's `AddrMangle::decode` recovers an IPv4 address and port by subtracting the
//! timestamp folded into them, without checking: bytes a client chose (`socket_addr` of
//! RelayResponse, PunchHoleSent, LocalAddr) overflow it, which panics with overflow checks on
//! and yields a garbage address without. [`decode`] refuses those inputs and otherwise returns
//! exactly what `AddrMangle::decode` does.

use std::net::SocketAddr;

use hbb_common::AddrMangle;

/// Longest IPv4 encoding; IPv6 ones are longer and involve no arithmetic.
const MAX_V4_LEN: usize = 16;
const TM_MASK: u128 = u32::MAX as u128;
const PORT_FIELD: u128 = 0xFF_FFFF;
const TM_PORT_MASK: u128 = 0xFFFF;

/// `AddrMangle::decode(bytes)`, or `None` where it would overflow.
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

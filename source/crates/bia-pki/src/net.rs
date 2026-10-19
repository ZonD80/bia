//! Addresses and networks for certificates bound to where they are used from.
use anyhow::{anyhow, Result};
use std::net::IpAddr;

/// An IPv4 or IPv6 network; a bare address is a /32 or /128.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cidr {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl Cidr {
    pub fn parse(s: &str) -> Result<Cidr> {
        let s = s.trim();
        let (a, p) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let addr = parse_ip(a).map_err(|_| anyhow!("{s:?} is not an IP address or network"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match p {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|n| *n <= max)
                .ok_or_else(|| anyhow!("{s:?}: bad prefix length"))?,
            None => max,
        };
        Ok(Cidr {
            addr: mask(addr, prefix),
            prefix,
        })
    }

    pub fn single(ip: IpAddr) -> Cidr {
        let ip = canonical(ip);
        Cidr {
            addr: ip,
            prefix: if ip.is_ipv4() { 32 } else { 128 },
        }
    }

    pub fn is_single(&self) -> bool {
        self.prefix == if self.addr.is_ipv4() { 32 } else { 128 }
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        ip.is_ipv4() == self.addr.is_ipv4() && mask(ip, self.prefix) == self.addr
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_single() {
            write!(f, "{}", self.addr)
        } else {
            write!(f, "{}/{}", self.addr, self.prefix)
        }
    }
}

/// Parse an address; IPv4-mapped IPv6 (`::ffff:1.2.3.4`) becomes IPv4.
pub fn parse_ip(s: &str) -> Result<IpAddr> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    let ip: IpAddr = s.parse().map_err(|_| anyhow!("not an IP address: {s}"))?;
    Ok(canonical(ip))
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

fn mask(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(v) => {
            let n = u32::from(v);
            let m = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix as u32) };
            IpAddr::V4((n & m).into())
        }
        IpAddr::V6(v) => {
            let n = u128::from(v);
            let m = if prefix == 0 { 0 } else { u128::MAX << (128 - prefix as u32) };
            IpAddr::V6((n & m).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks() {
        let n = Cidr::parse("10.0.0.0/24").unwrap();
        assert!(n.contains("10.0.0.7".parse().unwrap()));
        assert!(!n.contains("10.0.1.7".parse().unwrap()));
        assert!(n.contains(parse_ip("::ffff:10.0.0.9").unwrap()));
        assert_eq!(Cidr::parse("10.0.0.5/24").unwrap().to_string(), "10.0.0.0/24");
        assert_eq!(Cidr::parse("198.51.100.4").unwrap().to_string(), "198.51.100.4");
        assert!(Cidr::parse("2001:db8::/32").unwrap().contains("2001:db8::1".parse().unwrap()));
        assert!(Cidr::parse("10.0.0.0/33").is_err());
        assert!(Cidr::parse("bia").is_err());
    }
}

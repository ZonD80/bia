//! The running system's network, read from `ip -j` (not from netplan or ifupdown files:
//! what is live is what works), and written as ifupdown stanzas for bia. bia names
//! interfaces differently (`net.ifnames=0`), so stanzas use `@IF@` and are keyed by MAC;
//! `bia-net` fills in the name at boot.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::IpAddr;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Iface {
    pub mac: String,
    /// Name on the old system (for the summary only).
    #[serde(default)]
    pub name: String,
    /// IPv4 from DHCP.
    #[serde(default)]
    pub dhcp4: bool,
    /// Static addresses with prefix (`203.0.113.7/32`, `2a01:4f8::1/64`).
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    pub gateway4: Option<String>,
    /// The IPv4 gateway is outside every local subnet (e.g. Hetzner's 172.31.1.1 with a /32).
    #[serde(default)]
    pub onlink4: bool,
    #[serde(default)]
    pub gateway6: Option<String>,
    /// IPv6 by router advertisements (SLAAC) or DHCPv6.
    #[serde(default)]
    pub auto6: bool,
    #[serde(default)]
    pub dns: Vec<String>,
    #[serde(default)]
    pub mtu: Option<u32>,
}

impl Iface {
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("{} ({})", self.name, self.mac)];
        if self.dhcp4 {
            parts.push("IPv4 DHCP".into());
        }
        if !self.addresses.is_empty() {
            parts.push(self.addresses.join(" "));
        }
        if let Some(g) = &self.gateway4 {
            parts.push(format!("gw {g}{}", if self.onlink4 { " (on-link)" } else { "" }));
        }
        if let Some(g) = &self.gateway6 {
            parts.push(format!("gw6 {g}"));
        }
        if self.auto6 {
            parts.push("IPv6 auto".into());
        }
        if let Some(m) = self.mtu {
            if m != 1500 {
                parts.push(format!("mtu {m}"));
            }
        }
        parts.join(", ")
    }

    pub fn validate(&self) -> Result<()> {
        let mac = self.mac.to_ascii_lowercase();
        if mac.len() != 17 || !mac.split(':').all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok()) {
            bail!("interface {}: bad mac {:?}", self.name, self.mac);
        }
        for a in &self.addresses {
            let (ip, p) = a.split_once('/').unwrap_or((a, ""));
            let ip: IpAddr = ip.parse().map_err(|_| anyhow::anyhow!("interface {}: bad address {a}", self.name))?;
            let max = if ip.is_ipv4() { 32 } else { 128 };
            if p.parse::<u8>().ok().filter(|n| *n <= max).is_none() {
                bail!("interface {}: address {a} needs a prefix length (/{max} for a single address)", self.name);
            }
        }
        for g in self.gateway4.iter().chain(self.gateway6.iter()).chain(self.dns.iter()) {
            g.parse::<IpAddr>().map_err(|_| anyhow::anyhow!("interface {}: bad address {g}", self.name))?;
        }
        if !self.dhcp4 && !self.auto6 && self.addresses.is_empty() {
            bail!("interface {}: no DHCP and no addresses", self.name);
        }
        Ok(())
    }
}

/// ifupdown stanzas for one interface, with `@IF@` for its name.
pub fn stanza(i: &Iface) -> String {
    let v4: Vec<&String> = i.addresses.iter().filter(|a| !a.contains(':')).collect();
    let v6: Vec<&String> = i.addresses.iter().filter(|a| a.contains(':')).collect();
    let mut s = String::from("auto @IF@\n");
    if i.dhcp4 && v4.is_empty() {
        s.push_str("iface @IF@ inet dhcp\n");
    } else if let Some(first) = v4.first() {
        s.push_str(&format!("iface @IF@ inet static\n    address {first}\n"));
        if let Some(m) = i.mtu {
            s.push_str(&format!("    mtu {m}\n"));
        }
        for a in v4.iter().skip(1) {
            s.push_str(&format!("    post-up ip addr add {a} dev @IF@\n"));
        }
        if let Some(g) = &i.gateway4 {
            if i.onlink4 {
                s.push_str(&format!("    post-up ip route replace {g} dev @IF@\n"));
                s.push_str(&format!("    post-up ip route replace default via {g} dev @IF@ onlink\n"));
            } else {
                s.push_str(&format!("    post-up ip route replace default via {g} dev @IF@\n"));
            }
        }
    }
    if let Some(first) = v6.first() {
        s.push_str(&format!("iface @IF@ inet6 static\n    address {first}\n"));
        for a in v6.iter().skip(1) {
            s.push_str(&format!("    post-up ip -6 addr add {a} dev @IF@\n"));
        }
        if let Some(g) = &i.gateway6 {
            s.push_str(&format!("    post-up ip -6 route replace default via {g} dev @IF@\n"));
        }
    } else if i.auto6 {
        s.push_str("iface @IF@ inet6 auto\n");
    }
    s
}

/// `resolv.conf` for static configurations (DHCP brings its own).
pub fn resolv(ifaces: &[Iface]) -> Option<String> {
    let mut seen = vec![];
    for d in ifaces.iter().flat_map(|i| i.dns.iter()) {
        if !seen.contains(d) {
            seen.push(d.clone());
        }
    }
    (!seen.is_empty()).then(|| seen.iter().take(3).map(|d| format!("nameserver {d}\n")).collect())
}

/// Name servers of the running system: systemd-resolved's real upstreams, else
/// /etc/resolv.conf unless it points at the local stub.
pub fn parse_resolv(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .map(|s| s.trim().to_string())
        .filter(|s| s.parse::<IpAddr>().is_ok_and(|ip| !ip.is_loopback()))
        .collect()
}

/// Build the interface list from `ip -j -d link`, `ip -j addr`, `ip -j route show default`
/// and `ip -j -6 route show default`. Returns the interfaces that carry a global address
/// or a default route, and warnings for what cannot be carried over.
pub fn capture(link: &Value, addr: &Value, route4: &Value, route6: &Value, dns: &[String]) -> Result<(Vec<Iface>, Vec<String>)> {
    let mut out = vec![];
    let mut warn = vec![];
    let defaults_on = |r: &Value, dev: &str| -> Vec<Value> {
        r.as_array()
            .map(|a| a.iter().filter(|x| x["dev"].as_str() == Some(dev)).cloned().collect())
            .unwrap_or_default()
    };
    let kinds: std::collections::HashMap<String, String> = link
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    let n = l["ifname"].as_str()?.to_string();
                    let k = l["linkinfo"]["info_kind"].as_str().unwrap_or("").to_string();
                    let slave = l["linkinfo"]["info_slave_kind"].as_str().map(|s| format!("slave:{s}")).unwrap_or_default();
                    Some((n, if k.is_empty() { slave } else { k }))
                })
                .collect()
        })
        .unwrap_or_default();
    for l in addr.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let name = l["ifname"].as_str().unwrap_or("").to_string();
        if name.is_empty() || name == "lo" {
            continue;
        }
        let d4 = defaults_on(route4, &name);
        let d6 = defaults_on(route6, &name);
        let infos: Vec<&Value> = l["addr_info"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        let global: Vec<&&Value> = infos.iter().filter(|a| a["scope"].as_str() == Some("global")).collect();
        if global.is_empty() && d4.is_empty() && d6.is_empty() {
            continue;
        }
        let kind = kinds.get(&name).cloned().unwrap_or_default();
        if l["link_type"].as_str() != Some("ether") || !kind.is_empty() {
            let what = if kind.is_empty() { l["link_type"].as_str().unwrap_or("?").to_string() } else { kind };
            if !d4.is_empty() || !d6.is_empty() {
                bail!("the default route goes over {name} ({what}); bonds, bridges, VLANs and tunnels are not carried over — give \"network\" explicitly");
            }
            warn.push(format!("{name} ({what}) is not carried over"));
            continue;
        }
        let mac = l["address"].as_str().unwrap_or("").to_ascii_lowercase();
        let mut i = Iface { mac, name: name.clone(), dns: dns.to_vec(), mtu: l["mtu"].as_u64().map(|m| m as u32), ..Default::default() };
        for a in &global {
            let local = a["local"].as_str().unwrap_or("");
            let plen = a["prefixlen"].as_u64().unwrap_or(0);
            let dynamic = a["dynamic"].as_bool().unwrap_or(false);
            match a["family"].as_str() {
                Some("inet") if dynamic => i.dhcp4 = true,
                Some("inet") => i.addresses.push(format!("{local}/{plen}")),
                Some("inet6") if dynamic || a["mngtmpaddr"].as_bool().unwrap_or(false) => i.auto6 = true,
                Some("inet6") => i.addresses.push(format!("{local}/{plen}")),
                _ => {}
            }
        }
        if let Some(r) = d4.first() {
            if let Some(g) = r["gateway"].as_str() {
                let flagged = r["flags"].as_array().is_some_and(|f| f.iter().any(|x| x == "onlink"));
                let inside = i.addresses.iter().filter(|a| !a.contains(':')).any(|a| in_subnet(a, g));
                if !i.dhcp4 || !i.addresses.is_empty() {
                    i.gateway4 = Some(g.to_string());
                    i.onlink4 = flagged || !inside;
                }
            }
        }
        if let Some(r) = d6.first() {
            if let Some(g) = r["gateway"].as_str() {
                // a route learned from router advertisements comes back by itself
                let from_ra = r["protocol"].as_str() == Some("ra");
                if !from_ra || !i.auto6 {
                    i.gateway6 = Some(g.to_string());
                }
                if from_ra && i.addresses.iter().all(|a| !a.contains(':')) {
                    i.auto6 = true;
                }
            }
        }
        if i.dhcp4 {
            // DHCP brings its own name servers
            i.dns.clear();
        }
        if i.dhcp4 && i.addresses.iter().any(|a| !a.contains(':')) {
            warn.push(format!("{name} has DHCP and static IPv4 addresses; bia keeps the static ones"));
            i.dhcp4 = false;
        }
        i.validate()?;
        out.push(i);
    }
    if out.is_empty() {
        bail!("no wired interface with an address was found; give \"network\" explicitly");
    }
    Ok((out, warn))
}

fn in_subnet(cidr: &str, ip: &str) -> bool {
    let (Some((net, p)), Ok(ip)) = (cidr.split_once('/'), ip.parse::<std::net::Ipv4Addr>()) else {
        return false;
    };
    let (Ok(net), Ok(p)) = (net.parse::<std::net::Ipv4Addr>(), p.parse::<u32>()) else {
        return false;
    };
    let m = if p == 0 { 0 } else { u32::MAX << (32 - p.min(32)) };
    u32::from(net) & m == u32::from(ip) & m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Hetzner-style: a /32 with an on-link gateway, static IPv6 with fe80::1.
    #[test]
    fn hetzner_static() {
        let link = json!([{"ifname": "lo"}, {"ifname": "eth0", "link_type": "ether"}, {"ifname": "docker0", "linkinfo": {"info_kind": "bridge"}}]);
        let addr = json!([
            {"ifname": "lo", "link_type": "loopback", "addr_info": [{"family": "inet", "local": "127.0.0.1", "prefixlen": 8, "scope": "host"}]},
            {"ifname": "eth0", "link_type": "ether", "address": "96:00:01:23:45:67", "mtu": 1500, "addr_info": [
                {"family": "inet", "local": "203.0.113.7", "prefixlen": 32, "scope": "global"},
                {"family": "inet6", "local": "2a01:4f8::1", "prefixlen": 64, "scope": "global"},
                {"family": "inet6", "local": "fe80::1", "prefixlen": 64, "scope": "link"}]},
            {"ifname": "docker0", "link_type": "ether", "address": "02:42:00:00:00:01", "addr_info": [{"family": "inet", "local": "172.17.0.1", "prefixlen": 16, "scope": "global"}]}
        ]);
        let r4 = json!([{"dst": "default", "gateway": "172.31.1.1", "dev": "eth0", "flags": ["onlink"]}]);
        let r6 = json!([{"dst": "default", "gateway": "fe80::1", "dev": "eth0", "protocol": "static", "flags": []}]);
        let (ifs, warn) = capture(&link, &addr, &r4, &r6, &["185.12.64.1".into()]).unwrap();
        assert_eq!(ifs.len(), 1);
        assert!(warn.iter().any(|w| w.contains("docker0")));
        let i = &ifs[0];
        assert!(!i.dhcp4 && i.onlink4 && !i.auto6);
        assert_eq!(i.gateway6.as_deref(), Some("fe80::1"));
        let s = stanza(i);
        assert!(s.contains("iface @IF@ inet static\n    address 203.0.113.7/32\n"), "{s}");
        assert!(s.contains("post-up ip route replace 172.31.1.1 dev @IF@\n"), "{s}");
        assert!(s.contains("default via 172.31.1.1 dev @IF@ onlink"), "{s}");
        assert!(s.contains("iface @IF@ inet6 static\n    address 2a01:4f8::1/64\n"), "{s}");
        assert!(s.contains("ip -6 route replace default via fe80::1 dev @IF@"), "{s}");
        assert_eq!(resolv(&ifs).unwrap(), "nameserver 185.12.64.1\n");
    }

    /// A typical cloud VM: DHCP v4 + SLAAC v6, names from DHCP.
    #[test]
    fn cloud_dhcp() {
        let link = json!([{"ifname": "ens3", "link_type": "ether"}]);
        let addr = json!([{"ifname": "ens3", "link_type": "ether", "address": "52:54:00:12:34:56", "mtu": 9001, "addr_info": [
            {"family": "inet", "local": "10.0.2.15", "prefixlen": 24, "scope": "global", "dynamic": true},
            {"family": "inet6", "local": "2001:db8::5054:ff:fe12:3456", "prefixlen": 64, "scope": "global", "dynamic": true, "mngtmpaddr": true}]}]);
        let r4 = json!([{"dst": "default", "gateway": "10.0.2.2", "dev": "ens3", "protocol": "dhcp"}]);
        let r6 = json!([{"dst": "default", "gateway": "fe80::2", "dev": "ens3", "protocol": "ra"}]);
        let (ifs, _) = capture(&link, &addr, &r4, &r6, &["10.0.2.3".into()]).unwrap();
        let i = &ifs[0];
        assert!(i.dhcp4 && i.auto6 && i.addresses.is_empty() && i.gateway4.is_none() && i.gateway6.is_none() && i.dns.is_empty());
        assert_eq!(stanza(i), "auto @IF@\niface @IF@ inet dhcp\niface @IF@ inet6 auto\n");
    }

    #[test]
    fn default_route_over_a_bond_is_refused() {
        let link = json!([{"ifname": "bond0", "linkinfo": {"info_kind": "bond"}}]);
        let addr = json!([{"ifname": "bond0", "link_type": "ether", "address": "52:54:00:00:00:01", "addr_info": [{"family": "inet", "local": "10.0.0.2", "prefixlen": 24, "scope": "global"}]}]);
        let r4 = json!([{"dst": "default", "gateway": "10.0.0.1", "dev": "bond0"}]);
        assert!(capture(&link, &addr, &r4, &json!([]), &[]).is_err());
    }

    #[test]
    fn in_subnet_and_resolv() {
        assert!(in_subnet("10.0.0.5/24", "10.0.0.1"));
        assert!(!in_subnet("203.0.113.7/32", "172.31.1.1"));
        assert_eq!(parse_resolv("nameserver 127.0.0.53\nnameserver 1.1.1.1\n# x\n"), vec!["1.1.1.1".to_string()]);
        let bad = Iface { mac: "zz".into(), dhcp4: true, ..Default::default() };
        assert!(bad.validate().is_err());
        let noprefix = Iface { mac: "52:54:00:00:00:01".into(), addresses: vec!["10.0.0.2".into()], ..Default::default() };
        assert!(noprefix.validate().is_err());
    }
}

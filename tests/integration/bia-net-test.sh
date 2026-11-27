#!/bin/bash
# bia-net + bia-cidata on fake interfaces and fake cidata discs, in a trixie container (python3-yaml as in the image).
#   tests/integration/bia-net-test.sh
# Cases: a CubePath restore (new MAC, address only on the disc), a normal boot (the carried configuration wins),
# Proxmox's v1 network-config, no takeover with and without a disc, a broken disc.
set -u
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
if [ "${1:-}" != --inside ]; then
  exec docker run --rm -v "$ROOT":/work:ro -w /work debian:trixie-slim bash -c \
    "apt-get update -qq >/dev/null && apt-get install -y -qq --no-install-recommends python3 python3-yaml iproute2 >/dev/null && bash tests/integration/bia-net-test.sh --inside"
fi

SBIN=/work/source/os/overlay/usr/local/sbin
fails=0
ok() { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fails=$((fails + 1)); }
has() { grep -qF -- "$2" "$1" && ok "$3" || { bad "$3"; sed 's/^/       | /' "$1"; }; }
hasnt() { grep -qF -- "$2" "$1" && { bad "$3"; sed 's/^/       | /' "$1"; } || ok "$3"; }

# a fresh world: $W/sys (interfaces), $W/net (carried), $W/disc (the cidata disc's files), $W/out
world() {
  W=$(mktemp -d)
  mkdir -p "$W/sys" "$W/disc"
  echo "# shipped" > "$W/interfaces"
  export BIA_NET_SYS=$W/sys BIA_NET_DIR=$W/net BIA_NET_OUT=$W/interfaces BIA_NET_CIDATA=$W/ci \
    BIA_NET_RESOLV=$W/resolv.conf BIA_CIDATA_DIR=$W/disc BIA_CIDATA_BIN=$SBIN/bia-cidata
}
nic() { # name mac
  mkdir -p "$W/sys/$1"
  echo "$2" > "$W/sys/$1/address"
  echo 1 > "$W/sys/$1/type"
}
carried() { # mac-without-colons address gateway dns
  mkdir -p "$W/net"
  printf 'auto @IF@\niface @IF@ inet static\n    address %s\n    post-up ip route replace %s dev @IF@\n    post-up ip route replace default via %s dev @IF@ onlink\n' "$2" "$3" "$3" > "$W/net/$1.conf"
  echo "nameserver $4" > "$W/net/resolv.conf"
}
cubepath_disc() { # mac ipv4 gw (the disc CubePath gave VPS 29887, 2026-11-27)
  cat > "$W/disc/network-config" <<EOF
network:
  version: 2
  ethernets:
    public:
      match:
        macaddress: $1
      accept-ra: false
      addresses:
      - $2/32
      - 2607:a2c0::b7/128
      routes:
      - to: default
        via: $3
        on-link: true
      - to: default
        via: fe80::1
        on-link: true
      nameservers:
        addresses:
        - 1.1.1.1
        - 2606:4700:4700::1111
        - 8.8.8.8
        - 2001:4860:4860::8888
EOF
}

echo "== CubePath restore: the carried MAC is gone, the disc names the new one"
world
nic eth0 02:ff:02:24:ad:de
carried 02fb73410f40 45.90.237.105/32 45.90.237.1 9.9.9.9
cubepath_disc 02:ff:02:24:ad:de 45.90.237.200 45.90.237.1
start=$(date +%s)
bash $SBIN/bia-net
took=$(( $(date +%s) - start ))
has "$W/interfaces" "# 02ff0224adde (cidata disc)" "the disc's interface is used"
has "$W/interfaces" "address 45.90.237.200/32" "new address from the disc"
has "$W/interfaces" "post-up ip route replace default via 45.90.237.1 dev eth0 onlink" "on-link gateway on eth0"
has "$W/interfaces" "post-up ip -6 route replace default via fe80::1 dev eth0 || true" "IPv6 via fe80::1, never failing ifup"
has "$W/interfaces" "# 02fb73410f40: no interface with this address" "the old carried MAC is noted, not used"
hasnt "$W/interfaces" "45.90.237.105" "the old address is gone"
has "$W/resolv.conf" "nameserver 1.1.1.1" "name servers from the disc"
hasnt "$W/resolv.conf" "9.9.9.9" "not the carried ones"
[ "$(grep -c nameserver "$W/resolv.conf")" = 3 ] && ok "at most three name servers" || bad "at most three name servers"
[ "$took" -le 2 ] && ok "no wait for the carried MAC that will never come (${took}s)" || bad "waited ${took}s for the old MAC"

echo "== normal boot: the carried configuration wins for its own MAC"
world
nic eth0 02:fb:73:41:0f:40
carried 02fb73410f40 45.90.237.105/32 45.90.237.1 9.9.9.9
cubepath_disc 02:fb:73:41:0f:40 45.90.237.99 45.90.237.1
bash $SBIN/bia-net
has "$W/interfaces" "address 45.90.237.105/32" "carried address kept"
hasnt "$W/interfaces" "45.90.237.99" "the disc does not override a carried MAC"
has "$W/resolv.conf" "nameserver 9.9.9.9" "carried name servers kept"

echo "== stock Proxmox: v1 network-config, address + netmask, a second NIC on DHCP"
world
nic eth0 bc:24:11:aa:bb:cc
nic eth1 bc:24:11:dd:ee:ff
cat > "$W/disc/network-config" <<'EOF'
version: 1
config:
    - type: physical
      name: eth0
      mac_address: 'bc:24:11:aa:bb:cc'
      subnets:
      - type: static
        address: '192.0.2.10'
        netmask: '255.255.255.0'
        gateway: '192.0.2.1'
      - type: static6
        address: '2001:db8::10/64'
        gateway: '2001:db8::1'
    - type: physical
      name: eth1
      mac_address: 'bc:24:11:dd:ee:ff'
      subnets:
      - type: dhcp4
    - type: nameserver
      address:
      - '192.0.2.53'
      search:
      - 'example.com'
EOF
bash $SBIN/bia-net
has "$W/interfaces" "address 192.0.2.10/24" "netmask becomes a prefix"
has "$W/interfaces" "post-up ip route replace default via 192.0.2.1 dev eth0" "gateway inside the subnet"
hasnt "$W/interfaces" "via 192.0.2.1 dev eth0 onlink" "no on-link route where none is needed"
has "$W/interfaces" "address 2001:db8::10/64" "static IPv6"
has "$W/interfaces" "post-up ip -6 route replace default via 2001:db8::1 dev eth0 onlink || true" "IPv6 gateway reached first"
has "$W/interfaces" "iface eth1 inet dhcp" "the DHCP interface"
has "$W/resolv.conf" "nameserver 192.0.2.53" "global name server"

echo "== no takeover, no disc: nothing changes, no wait"
world
nic eth0 52:54:00:12:34:56
start=$(date +%s)
bash $SBIN/bia-net
took=$(( $(date +%s) - start ))
has "$W/interfaces" "# shipped" "interfaces untouched"
[ ! -e "$W/interfaces.bia-orig" ] && ok "no backup made" || bad "a backup was made"
[ "$took" -le 2 ] && ok "returned at once (${took}s)" || bad "took ${took}s"

echo "== no takeover, a disc for this machine: used"
world
nic eth0 52:54:00:12:34:56
cubepath_disc 52:54:00:12:34:56 203.0.113.7 203.0.113.1
bash $SBIN/bia-net
has "$W/interfaces" "address 203.0.113.7/32" "the disc configures a bia installed without a takeover"
has "$W/interfaces.bia-orig" "# shipped" "the shipped file is kept"

echo "== broken disc: ignored"
world
nic eth0 02:ff:02:24:ad:de
carried 02ff0224adde 45.90.237.200/32 45.90.237.1 9.9.9.9
printf 'network: [unclosed\n' > "$W/disc/network-config"
bash $SBIN/bia-net 2>"$W/err"
has "$W/interfaces" "address 45.90.237.200/32" "the carried configuration still applies"
has "$W/err" "unreadable network-config" "the problem is reported"

echo
[ "$fails" = 0 ] && echo "PASS bia-net" || { echo "FAIL bia-net: $fails"; exit 1; }

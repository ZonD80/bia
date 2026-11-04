#!/bin/bash
# bia's default venv: pip refuses the system, /home/bia/.venv is on PATH, it survives and rebuilds,
# and a venv an MCP makes for itself is not touched by it. Runs as root in Debian trixie, like the agent.
#   docker run --rm -v "$PWD":/work:ro debian:trixie-slim bash /work/tests/integration/venv-e2e.sh
set -u
apt-get update -qq >/dev/null; apt-get install -y -qq --no-install-recommends python3 python3-venv python3-gi util-linux findutils >/dev/null 2>&1
apt-get install -y -qq --no-install-recommends python3-pip >/dev/null 2>&1
useradd -m bia; R=/work/source/os/overlay
cp $R/etc/pip.conf /etc/pip.conf; cp $R/etc/profile.d/bia-venv.sh /etc/profile.d/; cp $R/usr/local/sbin/bia-venv /usr/local/sbin/; chmod +x /usr/local/sbin/bia-venv
failed=0
ck(){ if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; failed=1; fi; }
ck "first run creates it" '/usr/local/sbin/bia-venv && [ -x /home/bia/.venv/bin/python3 ]'
ck "owned by bia" '[ "$(stat -c %U /home/bia/.venv/bin/pip)" = bia ]'
ck "login sh python3 is the venv" '[ "$(sh -lc "command -v python3")" = /home/bia/.venv/bin/python3 ]'
ck "login sh as bia too" '[ "$(runuser -u bia -- sh -lc "command -v pip")" = /home/bia/.venv/bin/pip ]'
ck "sees system modules (gi)" 'sh -lc "python3 -c \"import gi\""'
ck "system pip refused" '! /usr/bin/python3 -m pip install -q six 2>/tmp/e; grep -q virtualenv /tmp/e'
ck "--break-system-packages refused" '! /usr/bin/python3 -m pip install -q --break-system-packages six 2>/tmp/e; grep -q virtualenv /tmp/e'
ck "nothing reached the system" '! ls /usr/local/lib/python3*/dist-packages/six* >/dev/null 2>&1'
ck "pip in a login shell installs into venv (root)" 'sh -lc "pip install -q six==1.16.0" 2>/dev/null; ls /home/bia/.venv/lib/python3*/site-packages/six.py >/dev/null'
/usr/local/sbin/bia-venv
ck "root install handed to bia" '[ "$(stat -c %U /home/bia/.venv/lib/python3*/site-packages/six.py)" = bia ]'
ck "second run keeps packages" '/usr/local/sbin/bia-venv; sh -lc "python3 -c \"import six\""'
echo "aarch64 3.1" > /home/bia/.venv/.bia-python
/usr/local/sbin/bia-venv
ck "other python: remade, list kept" 'grep -q "^six==1.16.0$" /home/bia/.venv/.reinstall && ! sh -lc "python3 -c \"import six\"" 2>/dev/null'
BIA_VENV_RETRY=1 /usr/local/sbin/bia-venv --reinstall
ck "reinstall brings it back" 'sh -lc "python3 -c \"import six\"" && [ ! -s /home/bia/.venv/.reinstall ]'
ck "new stamp" '! grep -q "3.1$" /home/bia/.venv/.bia-python'
echo "gone-from-pypi-xyz==1" > /home/bia/.venv/.reinstall; timeout 20 env BIA_VENV_RETRY=1 /usr/local/sbin/bia-venv --reinstall 2>/dev/null; ck "a failed name stays listed" 'grep -q gone-from /home/bia/.venv/.reinstall'
rm -rf /home/bia/.venv; mkdir -p /home/bia/.venv; ck "broken folder is rebuilt" '/usr/local/sbin/bia-venv; [ -x /home/bia/.venv/bin/python3 ]'
echo "--- an MCP's own venv next to bia's"
M=/home/bia/MCPs/my-x; mkdir -p $M; chown -R bia:bia /home/bia/MCPs
sh -lc "pip install -q six==1.16.0" 2>/dev/null
ck "made from a shell where the default venv is on PATH" "runuser -u bia -- sh -lc 'python3 -m venv $M/.venv'"
ck "it is based on the system python, not chained to bia's venv" "grep -q '^home = /usr/bin' $M/.venv/pyvenv.cfg"
ck "its own pip works under require-virtualenv, default venv still on PATH" "runuser -u bia -- sh -lc '$M/.venv/bin/pip install -q idna==3.10'"
ck "the package landed only in the MCP's venv" "ls $M/.venv/lib/python3*/site-packages/idna >/dev/null && ! ls /home/bia/.venv/lib/python3*/site-packages/idna >/dev/null 2>&1"
ck "bia's package is not visible to the MCP's venv" "! runuser -u bia -- sh -lc '$M/.venv/bin/python3 -c \"import six\"' 2>/dev/null"
ck "activating it wins over the default" "[ \"\$(runuser -u bia -- sh -lc '. $M/.venv/bin/activate; command -v python3')\" = $M/.venv/bin/python3 ]"
ck "pip after activating installs there" "runuser -u bia -- sh -lc '. $M/.venv/bin/activate; pip install -q certifi'; ls $M/.venv/lib/python3*/site-packages/certifi >/dev/null && ! ls /home/bia/.venv/lib/python3*/site-packages/certifi >/dev/null 2>&1"
ck "a VIRTUAL_ENV left in the env does not misdirect its pip" "VIRTUAL_ENV=/home/bia/.venv runuser -u bia -- env VIRTUAL_ENV=/home/bia/.venv $M/.venv/bin/pip install -q chardet; ls $M/.venv/lib/python3*/site-packages/chardet >/dev/null"
ck "bia-venv leaves the MCP venv alone" "/usr/local/sbin/bia-venv; $M/.venv/bin/python3 -c 'import idna'"
ck "a venv with system-site-packages sees system modules, not bia's" "runuser -u bia -- sh -lc 'python3 -m venv --system-site-packages $M/.v2 && $M/.v2/bin/python3 -c \"import gi\"' && ! $M/.v2/bin/python3 -c 'import six' 2>/dev/null"
exit $failed

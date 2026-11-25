#!/system/bin/sh
# bia on Android (route B): mount the Debian root in /data/bia/root and run what
# bia.service runs on a real bia (bia-venv, then bia-session with a virtual X
# screen, since the chroot has no /dev/dri). Android's kernel (4.4) has no PID/UTS/IPC/
# user namespaces, so there is no systemd: this script is the service manager.
#
# Installed as /data/adb/service.d/bia.sh (Magisk runs it as root after boot)
# and usable by hand:  bia-chroot.sh start|stop|status|shell [cmd...]
#
# It is also the phone's update step (bia-update on a PC reboots into a rescue image):
# when bia-update left /bia-update/android-apply, the new root it downloaded is copied
# over this one at boot, before anything of bia runs, then "bia-update boot" unfolds the
# agent. Its source is usr/local/share/bia/android/bia-chroot.sh in the bia image; a
# newer copy there replaces this file for the next boot.
R=/data/bia/root
LOG=/data/bia/log
mkdir -p $LOG

mounted() { grep -q " $1 " /proc/mounts; }

mount_all() {
  [ -x $R/bin/sh ] || { echo "no root at $R"; exit 1; }
  mounted $R/proc || mount -t proc proc $R/proc
  mounted $R/sys || mount -t sysfs sysfs $R/sys
  mounted $R/dev || mount -o bind /dev $R/dev
  mounted $R/dev/pts || mount -t devpts -o newinstance,ptmxmode=0666,mode=620,gid=5 devpts $R/dev/pts 2>/dev/null || mount -o bind /dev/pts $R/dev/pts
  mkdir -p $R/dev/shm; mounted $R/dev/shm || mount -t tmpfs -o mode=1777 tmpfs $R/dev/shm
  mounted $R/run || mount -t tmpfs -o mode=755 tmpfs $R/run
  # how bia-update and carry know they run on a phone
  echo android > $R/run/bia-host
  mounted $R/tmp || mount -t tmpfs -o mode=1777 tmpfs $R/tmp
  # Android has no local resolver for non-Android processes; use the network's
  # DNS when Android exposes it, else public resolvers.
  : > $R/run/resolv.conf
  for d in $(getprop net.dns1) $(getprop net.dns2) 1.1.1.1 8.8.8.8; do echo "nameserver $d" >> $R/run/resolv.conf; done
  ln -sf /run/resolv.conf $R/etc/resolv.conf
}

prepare_root() {
  # Android's kernel only lets members of AID_INET/AID_NET_RAW open sockets.
  chroot $R /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin /bin/sh -c '
    getent group aid_inet >/dev/null || groupadd -g 3003 aid_inet
    getent group aid_net_raw >/dev/null || groupadd -g 3004 aid_net_raw
    for u in root bia _apt; do id $u >/dev/null 2>&1 && usermod -aG aid_inet,aid_net_raw $u; done
    echo bia-mia2 > /etc/hostname
    # No systemd to create it at first boot; libdbus aborts without one.
    [ -s /etc/machine-id ] || dbus-uuidgen > /etc/machine-id
    mkdir -p /var/lib/dbus; [ -s /var/lib/dbus/machine-id ] || cp /etc/machine-id /var/lib/dbus/machine-id
  ' >> $LOG/prepare.log 2>&1
}

in_root() { chroot $R /usr/bin/env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin HOME=/root LANG=C.UTF-8 "$@"; }

# The staged update (bia-update start): copy /bia-update/net/root over this root while
# no bia process runs, with the rescue image's rules (android-apply, run inside the new
# root: Android has no rsync). On success /bia-update/pending is set for the unfold.
apply_update() {
  U=$R/bia-update
  [ -f $U/android-apply ] || return 0
  N=$R$(sed -n 's/^DIR=//p' $U/android-apply)
  echo "$(date) applying the staged update from $N" >> $LOG/update.log
  ok=0
  if [ -x $N/bin/bash ] && [ -f $N/etc/bia-release ]; then
    echo "copying from the downloaded release $(cat $N/etc/bia-version 2>/dev/null)" > $U/copying
    mkdir -p $N/mnt/dst $N/proc $N/dev $N/run
    mount -o bind $R $N/mnt/dst && mount -t proc proc $N/proc && mount -o bind /dev $N/dev \
      && mount -t tmpfs tmpfs $N/run
    # the script of this (old) root, outside what the copy replaces
    cp $R/usr/local/share/bia/android/android-apply $N/run/android-apply
    chroot $N /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin LANG=C.UTF-8 /bin/bash /run/android-apply /mnt/dst \
      >> $LOG/update.log 2>&1 && ok=1
    umount $N/run $N/dev $N/proc $N/mnt/dst 2>/dev/null
  else
    echo "$(date) $N is not a bia root" >> $LOG/update.log
  fi
  if [ $ok = 1 ]; then
    rm -rf $R/bia-update/net
    echo pending > $U/pending
    rm -f $U/copying $U/need-apply
    echo "$(date) copied; the agent is unfolded next" >> $LOG/update.log
  else
    # need-apply (and copying, when the copy started) stay: bia-update boot reports it
    echo "the copy at boot failed; see /data/bia/log/update.log on Android" > $U/rescue-failed
    echo "$(date) the copy failed" >> $LOG/update.log
  fi
  rm -f $U/android-apply
  # the log the agent reads
  tail -n 40 $LOG/update.log >> $U/update.log 2>/dev/null
  sync
}

# A newer launcher in the root (an update brought it) replaces this file for the next boot.
self_update() {
  new=$R/usr/local/share/bia/android/bia-chroot.sh
  me=/data/adb/service.d/bia.sh
  [ -f $new ] && [ -f $me ] || return 0
  cmp -s $new $me && return 0
  cp $new $me.new && chmod 755 $me.new && mv $me.new $me
  echo "$(date) launcher updated from the root" >> $LOG/update.log
}

start() {
  mount_all
  apply_update
  prepare_root
  # what bia.service's bia-update.service does at boot: finish an update (unfold), report,
  # clean /bia-update; before bia starts, so the unfold needs no restart
  in_root BIA_BEFORE_SESSION=1 /usr/local/sbin/bia-update boot >> $LOG/update-boot.log 2>&1
  # the unfold may have put back /etc files: the Android groups again
  prepare_root
  # what bia-swap.service and fstab do on a computer: /swapfile at half the RAM, then on
  # (bia-swap switches it on only when it resized it)
  in_root /usr/local/sbin/bia-swap >> $LOG/swap.log 2>&1
  grep -q "/swapfile " /proc/swaps || in_root swapon /swapfile >> $LOG/swap.log 2>&1
  self_update
  # bia.service: Restart=always, RestartSec=2
  (
    while [ ! -f /data/bia/stop ]; do
      chroot $R /usr/bin/env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        HOME=/root LANG=C.UTF-8 BIA_SYSTEM=1 BIA_PLATFORM=android \
        /bin/sh -c '/usr/local/sbin/bia-venv >/dev/null 2>&1; exec /usr/local/sbin/bia-session' \
        >> $LOG/bia.log 2>> $LOG/bia.err
      sleep 2
    done
  ) &
  echo $! > /data/bia/run.pid
}

stop() {
  touch /data/bia/stop
  for p in $(ls /proc | grep -E '^[0-9]+$'); do
    l=$(readlink /proc/$p/root 2>/dev/null)
    [ "$l" = "$R" ] && kill $p 2>/dev/null
  done
  sleep 3
  rm -f /data/bia/stop
}

case "${1:-start}" in
  start) rm -f /data/bia/stop
         # Magisk service.d runs early: wait for Android to finish booting.
         until [ "$(getprop sys.boot_completed)" = 1 ]; do sleep 2; done
         start ;;
  stop) stop ;;
  status) for p in $(ls /proc | grep -E '^[0-9]+$'); do
            [ "$(readlink /proc/$p/root 2>/dev/null)" = "$R" ] && echo "$p $(tr '\0' ' ' < /proc/$p/cmdline | cut -c1-100)"
          done ;;
  shell) mount_all; shift; chroot $R /usr/bin/env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin HOME=/root LANG=C.UTF-8 TERM=${TERM:-xterm} /bin/bash ${1:+-c "$*"} ;;
esac

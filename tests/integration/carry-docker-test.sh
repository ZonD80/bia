#!/bin/bash
# Docker in a carry archive (bia-mcp-carry docker.rs) against a real dockerd: containers
# (compose labels, restart policy, env, a network alias, one sharing another's network, one
# never started), a user network with its own subnet, a volume with data owned by a
# container uid; export, wipe Docker completely, import, and check everything is back.
#
#   tests/integration/carry-docker-test.sh       (builds the carry binary in bia-build:$ARCH,
#                                                  then runs itself in a privileged Debian)
set -uo pipefail
ARCH=${ARCH:-arm64}
case $ARCH in arm64) PLATFORM=linux/arm64 ;; x86_64) PLATFORM=linux/amd64 ;; *) exit 2 ;; esac
if [ "${IN_CONTAINER:-}" != 1 ]; then
  cd "$(dirname "$0")/../.."
  docker run --rm --platform "$PLATFORM" -v "$PWD":/work -v "bia-cache-$ARCH":/cache -w /work \
    -e CARGO_TARGET_DIR="/cache/target-$ARCH-test" "bia-build:$ARCH" \
    bash -lc "cargo build -q -p bia-mcp-carry && cp /cache/target-$ARCH-test/debug/bia-mcp-carry /work/target/carry-docker-test-bin" || exit 1
  docker run --rm --privileged --platform "$PLATFORM" -e IN_CONTAINER=1 \
    -v "$PWD":/work:ro -w /work debian:trixie bash /work/tests/integration/carry-docker-test.sh
  rc=$?
  rm -f target/carry-docker-test-bin
  exit $rc
fi

FAILED=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; FAILED=1; fi; }
CARRY=/work/target/carry-docker-test-bin

apt-get update -qq >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq docker.io docker-compose busybox-static >/dev/null 2>&1 \
  || { echo "FAIL apt-get install docker.io"; exit 1; }
dockerd >/var/log/dockerd.log 2>&1 &
for _ in $(seq 60); do docker info >/dev/null 2>&1 && break; sleep 1; done
check "dockerd runs" "docker info >/dev/null 2>&1"

# a tiny image from busybox
R=$(mktemp -d); mkdir -p "$R/bin"; cp /bin/busybox "$R/bin/"; for c in sh sleep cat echo chown true; do ln -s busybox "$R/bin/$c"; done
tar -C "$R" -c . | docker import -c 'CMD ["/bin/sh"]' - bia-test/tiny:1 >/dev/null
docker network create --subnet 10.77.0.0/24 --label made-by=test testnet >/dev/null
docker volume create --label kind=db data >/dev/null
docker run -d --name db --network testnet --network-alias dbalias --ip 10.77.0.50 -v data:/data \
  --restart unless-stopped -e SECRET=abc --label com.docker.compose.project=market \
  bia-test/tiny:1 sh -c 'echo hello > /data/f; chown 999:999 /data/f; sleep 100000' >/dev/null
docker run -d --name app --network container:db bia-test/tiny:1 sleep 100000 >/dev/null
docker create --name idle -v data:/data:ro bia-test/tiny:1 true >/dev/null
# a compose project, like the market's (app + db with a named volume)
mkdir -p /tmp/proj && cat > /tmp/proj/compose.yaml <<'YAML'
services:
  pdb:
    image: bia-test/tiny:1
    command: sh -c 'echo rows > /var/db/t; sleep 100000'
    volumes: [pgdata:/var/db]
    restart: unless-stopped
  papp:
    image: bia-test/tiny:1
    command: sleep 100000
    depends_on: [pdb]
volumes:
  pgdata:
YAML
(cd /tmp/proj && docker-compose -p proj up -d >/dev/null 2>&1)
sleep 2
check "the fixture runs" "[ \"\$(docker ps --format '{{.Names}}' | sort | tr '\n' ' ')\" = 'app db proj-papp-1 proj-pdb-1 ' ]"

E=/tmp/export
$CARRY --docker-export $E || FAILED=1
check "export wrote docker.json, images and the volume" "[ -s $E/docker.json ] && [ -s $E/images.tar ] && [ -s $E/volumes/data.tar ]"
check "running containers were unpaused after" "[ \"\$(docker inspect -f '{{.State.Paused}}' db)\" = false ]"

# Docker as on a fresh system
docker rm -f db app idle proj-papp-1 proj-pdb-1 >/dev/null
docker network rm testnet proj_default >/dev/null
docker volume rm data proj_pgdata >/dev/null
docker rmi bia-test/tiny:1 >/dev/null
docker system prune -af --volumes >/dev/null
check "docker is empty" "[ -z \"\$(docker ps -aq)\" ] && [ -z \"\$(docker images -q)\" ] && [ -z \"\$(docker volume ls -q)\" ]"

$CARRY --docker-import $E || FAILED=1
sleep 2
check "the image is back with its tag" "docker image inspect bia-test/tiny:1 >/dev/null 2>&1"
check "db and app run again, idle exists stopped" "[ \"\$(docker ps --format '{{.Names}}' | sort | tr '\n' ' ')\" = 'app db proj-papp-1 proj-pdb-1 ' ] && [ \"\$(docker inspect -f '{{.State.Status}}' idle)\" = created ]"
check "db keeps env, labels and restart policy" "docker inspect -f '{{.Config.Env}} {{index .Config.Labels \"com.docker.compose.project\"}} {{.HostConfig.RestartPolicy.Name}}' db | grep -q 'SECRET=abc.* market unless-stopped'"
check "db keeps its network, alias and address" "docker inspect -f '{{json .NetworkSettings.Networks.testnet}}' db | grep -q '\"dbalias\"' && [ \"\$(docker inspect -f '{{.NetworkSettings.Networks.testnet.IPAddress}}' db)\" = 10.77.0.50 ]"
check "the network has its subnet and label" "docker network inspect -f '{{(index .IPAM.Config 0).Subnet}} {{.Labels}}' testnet | grep -q '10.77.0.0/24 map\[made-by:test\]'"
check "app shares the new db's network" "[ \"\$(docker inspect -f '{{.HostConfig.NetworkMode}}' app)\" = container:\$(docker inspect -f '{{.Id}}' db) ]"
check "the volume's data and owner came back" "[ \"\$(docker exec db cat /data/f)\" = hello ] && [ \"\$(docker run --rm -v data:/d bia-test/tiny:1 sh -c 'ls -n /d/f' | awk '{print \$3}')\" = 999 ]"
check "the volume keeps its label" "[ \"\$(docker volume inspect -f '{{.Labels.kind}}' data)\" = db ]"

check "the compose volume's data came back" "[ \"\$(docker exec proj-pdb-1 cat /var/db/t)\" = rows ]"
ids=$(docker ps -q --no-trunc --filter name=proj- | sort)
up=$(cd /tmp/proj && docker-compose -p proj up -d 2>&1)
check "compose owns the containers again and keeps them (no recreate)" "[ \"\$(docker ps -q --no-trunc --filter name=proj- | sort)\" = \"\$ids\" ] && ! echo \"\$up\" | grep -qi recreat"
# again: what exists is left alone
out=$($CARRY --docker-import $E 2>&1)
check "a second import leaves existing containers alone" "echo \"\$out\" | grep -q '5 containers were already here'"

[ $FAILED = 0 ] && echo "all passed" || echo "FAILED"
exit $FAILED

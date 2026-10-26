#!/bin/sh
# aleph against a real herdr (release asset) in two containers:
#   box     sshd + `herdr server` + stub claude
#   client  aleph + herdr client; runs checks.sh:
#           doctor --json, ls --json, send, keys, rename, close
#
#   tests/docker-e2e/run.sh            build + run
#   KEEP=1 tests/docker-e2e/run.sh     leave the containers running to poke at
#   HERDR_VERSION=v0.9.5 / HERDR_URL=…  pick the herdr build
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
image=aleph-e2e
net=aleph-e2e-$$
box=aleph-e2e-box-$$

command -v docker >/dev/null || { echo "error: docker not found" >&2; exit 2; }
docker info >/dev/null 2>&1 || { echo "error: the docker daemon isn't running" >&2; exit 2; }

cleanup() {
  if [ "${KEEP:-0}" = 1 ]; then
    echo "hint: kept $box on $net — docker exec -it -u dev $box bash; docker rm -f $box; docker network rm $net"
    return
  fi
  docker rm -f "$box" >/dev/null 2>&1 || true
  docker network rm "$net" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

echo "building $image (aleph from source + herdr ${HERDR_VERSION:-v0.9.5})"
docker build -q -f "$here/Dockerfile" -t "$image" \
  --build-arg "HERDR_VERSION=${HERDR_VERSION:-v0.9.5}" \
  --build-arg "HERDR_URL=${HERDR_URL:-}" \
  "$root" >/dev/null

docker network create "$net" >/dev/null
docker run -d --name "$box" --hostname box --network "$net" --network-alias box "$image" box >/dev/null

i=0
until docker logs "$box" 2>&1 | grep -q "box ready"; do
  i=$((i + 1))
  if [ $i -gt 60 ] || [ "$(docker inspect -f '{{.State.Running}}' "$box")" != true ]; then
    echo "error: the box didn't start" >&2
    docker logs "$box" >&2 || true
    exit 1
  fi
  sleep 1
done

status=0
docker run --rm --network "$net" "$image" client || status=$?
if [ $status -ne 0 ]; then
  echo "--- box: herdr server log" >&2
  docker exec "$box" tail -n 40 /tmp/herdr-server.log >&2 || true
fi
exit $status

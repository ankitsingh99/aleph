# aleph against a real herdr, in containers.
# One image, two roles (see run.sh):
#   box     sshd + `herdr server` + a stub `claude` (the remote machine)
#   client  aleph + herdr client + ssh (your laptop)
# Build context is the repository root:
#   docker build -f tests/docker-e2e/Dockerfile -t aleph-e2e .

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release && cp target/release/aleph /aleph

FROM debian:bookworm-slim
ARG HERDR_VERSION=v0.9.5
# full URL override, e.g. for a mirror or a newer herdr
ARG HERDR_URL=
RUN apt-get update \
 && apt-get install -y --no-install-recommends openssh-server openssh-client ca-certificates curl jq procps git \
 && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    url=${HERDR_URL:-https://github.com/ogulcancelik/herdr/releases/download/${HERDR_VERSION}/herdr-linux-$(uname -m)}; \
    echo "herdr: $url"; \
    curl -fsSL -o /usr/local/bin/herdr "$url"; \
    chmod +x /usr/local/bin/herdr; \
    herdr --version
COPY --from=build /aleph /usr/local/bin/aleph
COPY tests/docker-e2e/claude-stub /usr/local/bin/claude
COPY tests/docker-e2e/entrypoint.sh /usr/local/bin/entrypoint
COPY tests/docker-e2e/checks.sh /usr/local/bin/checks

# user `dev` with one key pair that both roles share: the client logs into the box with it
RUN set -eu; \
    useradd -m -s /bin/bash dev; \
    mkdir -p /home/dev/.ssh /run/sshd; \
    ssh-keygen -q -t ed25519 -N '' -f /home/dev/.ssh/id_ed25519 -C dev@e2e; \
    cp /home/dev/.ssh/id_ed25519.pub /home/dev/.ssh/authorized_keys; \
    printf 'Host box\n  HostName box\n  User dev\n  StrictHostKeyChecking no\n  UserKnownHostsFile /dev/null\n  LogLevel ERROR\n' > /home/dev/.ssh/config; \
    mkdir -p /home/dev/.config/aleph /home/dev/project; \
    printf 'ssh_host=box\nmachine=box\nhosts=box\n' > /home/dev/.config/aleph/config; \
    chown -R dev:dev /home/dev; \
    chmod 700 /home/dev/.ssh; chmod 600 /home/dev/.ssh/*; \
    chmod +x /usr/local/bin/claude /usr/local/bin/entrypoint /usr/local/bin/checks; \
    ssh-keygen -A; \
    printf 'PasswordAuthentication no\nPermitRootLogin no\nAcceptEnv LANG LC_*\n' > /etc/ssh/sshd_config.d/e2e.conf

ENV LANG=C.UTF-8 TERM=xterm-256color
ENTRYPOINT ["/usr/local/bin/entrypoint"]
CMD ["box"]

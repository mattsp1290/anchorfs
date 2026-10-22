# Multi-stage build for the AnchorFS runtime images.
# service-base / mount-base carry only the OS layers (packages and uid/gid 10001)
# and deliberately no ENTRYPOINT, CMD or USER: scripts/oci_export.py sets those
# explicitly when it layers the cross-built binaries onto the published bases
# (scripts/publish-bases.sh, docker/bases.json). The service / mount targets are
# the same base plus exactly one binary and are what ci/image-smoke.sh checks.
FROM rust:1.95.0-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1 AS builder
ENV RUSTUP_TOOLCHAIN=1.95.0
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p anchorfs-service -p anchorfs-mount


FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8 AS service-base
LABEL org.opencontainers.image.source="https://github.com/mattsp1290/anchorfs" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
RUN groupadd --gid 10001 anchorfs && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin anchorfs

FROM service-base AS service
COPY --from=builder /src/target/release/anchorfs-service /usr/local/bin/
USER 10001:10001
ENTRYPOINT ["anchorfs-service"]
CMD ["serve"]

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8 AS mount-base
LABEL org.opencontainers.image.source="https://github.com/mattsp1290/anchorfs" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates fuse3 && rm -rf /var/lib/apt/lists/*
RUN groupadd --gid 10001 anchorfs && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin anchorfs

FROM mount-base AS mount
COPY --from=builder /src/target/release/anchorfs-mount /usr/local/bin/
USER 10001:10001
ENTRYPOINT ["anchorfs-mount"]

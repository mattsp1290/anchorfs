# Container images

Build from the repository root on the target architecture:

```sh
docker build -f docker/Dockerfile --target service -t anchorfs-service:local .
docker build -f docker/Dockerfile --target mount -t anchorfs-mount:local .
```

Both targets share one release builder, use `--locked` and digest-pinned
Rust/Debian bases, and run as uid/gid 10001. apt package versions are unpinned.
The service runtime contains ca-certificates. The mount runtime additionally
contains fuse3 and its fusermount3 helper. GitHub CI builds and smoke-tests
these Docker targets on native amd64 and arm64 runners; they are the test path,
not the deployed artifacts.

## Runtime bases and the daemonless delivery path

The Dockerfile's runtime layers are separate stages, `service-base` and
`mount-base`, that contain the packages and the uid 10001 user but no binary.
`service` and `mount` build `FROM` them. The owner publishes the two base stages
once as multi-platform images `anchorfs/service-base` and `anchorfs/mount-base`
to the apartment Zot registry with `scripts/publish-bases.sh` (workstation on
the apartment LAN only; the script writes the index and per-platform digests
into `docker/bases.json`, which is committed and checked by a provenance test).
The Forgejo delivery workflow (`docs/ops/forgejo-delivery.md`) never runs
Docker: it cross-compiles both binaries with cargo-zigbuild
(`scripts/build-release-binaries.sh`), pulls the bases by digest, and
`scripts/oci_export.py` appends one deterministic binary layer per platform
onto the base manifests, producing OCI layouts and a v1 build receipt that the
shared birbhomes publisher uploads. A base stage change requires re-running
`publish-bases.sh` and committing the new digests; the exporter refuses a
mismatch. The Docker targets and the exporter output are built from the same
stages at different times (apt is unpinned), so their package sets can differ.

The deployed images therefore come from `publication.json` of a recorded
Forgejo main run, pulled by cluster nodes as
`10.0.0.106:30001/anchorfs/<role>@sha256:<digest>`. No image is published to
any public registry; there is no signing or SBOM.

The service defaults to durable and `127.0.0.1:8080`, so the default container
listener is unreachable from outside. For a reachable durable service:

- Set `ANCHORFS_LISTEN_ADDR=0.0.0.0:8080` and map the port.
- Set both `ANCHORFS_TLS_CERT_PATH` and `ANCHORFS_TLS_KEY_PATH` to read-only
  mounted files readable by uid 10001. Never bake key material into an image.
- Supply the normal database, deployment, cursor signing and S3 configuration
  through the deployment's secret/configuration mechanism.
- Configure clients with the issuing CA (`--ca-bundle` for the mount).

The service uses TLS 1.3 and HTTP/1.1 only, without client certificates.
Certificates are loaded once at startup; rotation requires restart. Partial
or invalid TLS input exits 2 before database access; durable non-loopback
plaintext exits 7. Development permits plaintext, including binding all
addresses, for disposable local testing. PostgreSQL and S3 transport policies
are unchanged. No certificate or key example is shipped.

Mount containers additionally require `/dev/fuse`, `SYS_ADMIN` or an equivalent host/pod
policy, a writable mountpoint and mount propagation chosen for the
consumer. The image runs as uid 10001; device and mountpoint permissions must
allow that user. These runtime settings and containerized kernel mounting
are not verified by packaging smoke. No cluster, Helm, Spaces or cross-node
run is part of this milestone. Image publication and registry credentials
require a later deployment decision.

The chart in `charts/anchorfs` installs the service with these images (see
`docs/ops/cluster-proof.md`); its Helm test runs `anchorfs-service probe --url
https://anchorfs:8080 --ca-bundle … --require-tls --require-durable`, an
unauthenticated single `GET /readyz` with exit codes 0 (ready and requirements
met), 1 (not ready or requirement failed) and 2 (invalid arguments, unreadable
CA bundle or transport failure).

[Acceptance evidence](../provenance/stage3a-tls-and-images-acceptance.md)
separates kernel tests of the host binary from packaging smoke tests.

The OCI licenses label describes AnchorFS code. Debian base/runtime packages
carry their own licenses, recorded in dependencies.md. A later image publication
review must consider those packages, including the fuse3 source-availability
requirements; this milestone does not publish images.

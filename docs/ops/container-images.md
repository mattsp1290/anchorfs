# Container images

Build from the repository root on the target architecture:

```sh
docker build -f docker/Dockerfile --target service -t anchorfs-service:local .
docker build -f docker/Dockerfile --target mount -t anchorfs-mount:local .
```

Both targets share one release builder, use `--locked` and digest-pinned
Rust/Debian bases, and run as uid/gid 10001. apt package versions are unpinned.
The service runtime contains ca-certificates. The mount runtime additionally
contains fuse3 and its fusermount3 helper. CI builds and smoke-tests on native
amd64 and arm64 runners. No images are published or uploaded; there is no
multi-arch manifest list, signing or SBOM.

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

Mount containers additionally require `/dev/fuse`, appropriate host/pod
capabilities, a writable mountpoint and mount propagation chosen for the
consumer. The image runs as uid 10001; device and mountpoint permissions must
allow that user. These runtime settings and containerized kernel mounting
are not verified by packaging smoke. No cluster, Helm, Spaces or cross-node
run is part of this milestone. Image publication and registry credentials
require a later deployment decision.

[Acceptance evidence](../provenance/stage3a-tls-and-images-acceptance.md)
separates kernel tests of the host binary from packaging smoke tests.

The OCI licenses label describes AnchorFS code. Debian base/runtime packages
carry their own licenses, recorded in dependencies.md. A later image publication
review must consider those packages, including the fuse3 source-availability
requirements; this milestone does not publish images.

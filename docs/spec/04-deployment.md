# Deployment and consumer integration

## Common topology

Deploy API replicas, PostgreSQL, S3-compatible object storage, and mount clients. API readiness requires database access and backend capability checks; liveness must not restart healthy processes merely because a remote dependency is down. Client pods mount the same workspace ID through the service, never by attaching one RWO cache PVC across nodes.

Provide Helm templates and example values for local k3s and DOKS. Include requests/limits, probes, graceful termination, bounded connection pools, NetworkPolicies, TLS, Secret references, and a maintenance mode. Never place plaintext keys in values files or examples. Backend options include endpoint, region, path/virtual-host addressing, bucket, deployment prefix, CA bundle, credential source, request timeout, and retry limits. TLS verification remains enabled; plaintext endpoints are an explicit local-only profile.

Stage one mounting with a dedicated trusted FUSE client in the workload pod or a validated host mount arrangement. Before selecting a sidecar manifest, prove `/dev/fuse`, required capabilities, mount propagation and workload visibility on both clusters. Keep untrusted agent containers unprivileged. If policy disallows a sidecar mount, use a trusted CSI node plugin; do not silently grant privileged mode to agent workloads. CSI automation is a distinct deliverable after a manual multi-node mount proof.

## Local k3s

Use amd64/arm64 images and schedule only onto verified healthy nodes. Existing MinIO was Pending at inspection; the candidate service address is in the environment evidence, not a validated production endpoint. Verify MinIO health, immutable image digest, credentials, storage capacity and failure mode before live acceptance. Repair is a separate infrastructure task.

`local-path` is node-local; neither cache nor metadata on it warrants a node-loss durability claim. NFS classes exist but their server resilience and PostgreSQL guarantees were not established. For development a single database is acceptable with explicit downtime/data-loss boundaries and tested backups. Do not label this deployment highly available.

## DOKS and Spaces

Use the regional Spaces origin API endpoint, not CDN, with scoped private-bucket credentials. Create a dedicated prefix owned by one metadata authority. Do not depend on AWS STS, IAM role assumption, SSE-KMS, bucket notifications, or object conditional-write behavior being available in Spaces.

Run a provider contract suite covering PUT/GET/HEAD/delete/list pagination, range reads, checksum handling, timeouts, throttling and read-after-upload before enabling that backend. Required published chunks must be readable when a manifest is exposed; establish this experimentally and do not substitute AWS's documented guarantees for Spaces evidence. Retry missing data boundedly and surface integrity/unavailability errors, never zero-fill absent objects.

PostgreSQL deployment/managed-service selection must explicitly state failover RPO and fencing. Multiple API replicas alone do not provide database HA. Separate disposable cache storage from metadata storage. Choose same-region compute/storage and record cost and latency measurements.

## AWS later

EC2 uses the Linux mount client; AWS S3 general-purpose buckets use the standard SDK credential chain with refreshable workload credentials. Exercise IAM scoping, credential rotation, reconnect and instance loss. The provisional Lambda integration uses TLS API calls; define VPC/private connectivity or a secured public endpoint. Do not put Spaces static credentials or cluster-admin tokens into agent workloads.

## Crabber and eino-agent host adapters

For hosts executing tools locally, mount before constructing runtime/tool context. A harness that delegates all filesystem operations may use the native API without mounting; its remote tool worker mounts before constructing its execution context. Assign a stable workspace ID and mount path, authorize the caller, then supply that canonical path through the host's existing directory/workspace fields. Resume must resolve the same authorized workspace rather than whichever directory happens to exist at that path. Unmount/drain after handles and tools have stopped; forced worker shutdown requires explicit fencing.

Keep their SQL session state on their supported stores. The storage database is a separate schema/service lifecycle and must not join runtime internal tables. The Go runtime's process-local tool locks do not coordinate pods. For Git/build/patch multi-call workflows, hosts acquire a shared cross-process coordinator lease or isolate working trees. A lease that expires while a command is still running must fence filesystem mutations by lease-bound mount/session epoch before another command is admitted. Merely observing a worker as dead is insufficient.

Ordinary concurrent syscall editing remains available when no exclusive workflow holds the workspace; while held, the server rejects mutations without its token. Expose conditional file generations to API-based editing tools so they can report conflict rather than overwrite unseen changes.

## Operational evidence

Metrics: operation latency/error counts, metadata conflicts/retries, object requests/bytes, cache hit ratio, checksum failures, orphan bytes, quota consumption, open handles, upload reservations, and receipt lookups. Bound labels: no raw paths, secrets, or per-file IDs as metric dimensions. Record schema and object-format versions in health/admin output.

Runbook must cover backend outage, database outage, ambiguous commits, stale clients, full cache, credential revocation, remount after eviction, graceful drain, backup/restore, and version rollback compatibility. No destructive fault tests against existing user data; use a dedicated disposable environment.

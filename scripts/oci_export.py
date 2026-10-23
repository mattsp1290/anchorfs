#!/usr/bin/env python3
"""Daemonless OCI exporter for AnchorFS (Python >= 3.11, standard library only).

Layers the cross-built anchorfs-service / anchorfs-mount binaries onto the
digest-pinned runtime base layouts listed in docker/bases.json, packages the
Helm chart as an OCI artifact, and writes three OCI layouts (service, mount,
chart) plus a v1 build receipt (build.json) that the shared publisher accepts.
Never contacts a registry or a container daemon.

Run with: python3 -I scripts/oci_export.py --help

bases.json may carry a per-base `stage_sha256` (hash of the Dockerfile base stage at
publication). It is validated when present; with the optional `--dockerfile` the
exporter also recomputes it from that Dockerfile and refuses a mismatch (base_mismatch).

Errors print one stable code on stderr (invalid_input, base_mismatch,
elf_architecture, chart_invalid, output_exists) and exit 2. File contents are
never printed. The receipt is written last, so a failed export has no receipt.
"""
import argparse
import hashlib
import io
import json
import re
import shutil
import struct
import sys
import tarfile
from pathlib import Path

SCHEMA_VERSION = 1
OCI_INDEX = "application/vnd.oci.image.index.v1+json"
OCI_MANIFEST = "application/vnd.oci.image.manifest.v1+json"
OCI_CONFIG = "application/vnd.oci.image.config.v1+json"
OCI_LAYER = "application/vnd.oci.image.layer.v1.tar"
OCI_BASE_LAYERS = {
    "application/vnd.oci.image.layer.v1.tar",
    "application/vnd.oci.image.layer.v1.tar+gzip",
    "application/vnd.oci.image.layer.v1.tar+zstd",
}
HELM_CONFIG = "application/vnd.cncf.helm.config.v1+json"
HELM_LAYER = "application/vnd.cncf.helm.chart.content.v1.tar+gzip"
REVISION = "org.opencontainers.image.revision"
SOURCE = "org.opencontainers.image.source"

PLATFORMS = ("amd64", "arm64")  # fixed descriptor order
ELF_MACHINE = {"amd64": 0x3E, "arm64": 0xB7}
BINARIES = {"service": "anchorfs-service", "mount": "anchorfs-mount"}
CHART_SCALARS = ("apiVersion", "name", "version", "appVersion", "description", "type", "kubeVersion")
MAX_JSON = 8 * 1024 * 1024
MAX_CHART_YAML = 64 * 1024

SHA_RE = re.compile(r"[0-9a-f]{40}")
RUN_ID_RE = re.compile(r"[1-9][0-9]*")
DIGEST_RE = re.compile(r"sha256:[0-9a-f]{64}")
BUILDER_RE = re.compile(r"[a-zA-Z0-9.:/_-]+@sha256:[0-9a-f]{64}")
APP_ID_RE = re.compile(r"[a-z0-9][a-z0-9-]{0,63}")
REPO_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*")


class ExportError(Exception):
    def __init__(self, code):
        super().__init__(code)
        self.code = code


def require(condition, code):
    if not condition:
        raise ExportError(code)


# ---------------------------------------------------------------- primitives

def _no_duplicates(items):
    result = {}
    for key, value in items:
        require(key not in result, "invalid_input")
        result[key] = value
    return result


def decode_json(raw, code="invalid_input"):
    require(len(raw) <= MAX_JSON, code)
    try:
        return json.loads(raw, object_pairs_hook=_no_duplicates)
    except (ValueError, UnicodeError, RecursionError):
        raise ExportError(code) from None


def canonical(value):
    """Sorted keys, compact separators, UTF-8, one trailing newline."""
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False,
                       ensure_ascii=False) + "\n").encode("utf-8")


def sha256_hex(raw):
    return hashlib.sha256(raw).hexdigest()


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def descriptor(media_type, raw):
    return {"mediaType": media_type, "digest": "sha256:" + sha256_hex(raw), "size": len(raw)}


def match(value, pattern, code="invalid_input"):
    require(isinstance(value, str) and pattern.fullmatch(value) is not None, code)
    return value


def check_keys(value, required, optional=(), code="invalid_input"):
    require(isinstance(value, dict) and set(required) <= set(value)
            and set(value) <= set(required) | set(optional), code)


def check_no_symlinks(root):
    """Reject a symlink anywhere under (or at) an input path."""
    root = Path(root)
    require(not root.is_symlink(), "invalid_input")
    if root.is_dir():
        for path in root.rglob("*"):
            require(not path.is_symlink(), "invalid_input")


def read_regular(path, limit=None, code="invalid_input"):
    path = Path(path)
    require(path.is_file() and not path.is_symlink(), code)
    if limit is not None:
        require(path.stat().st_size <= limit, code)
    return path.read_bytes()


# ------------------------------------------------------------------ ELF check

def check_elf(path, arch):
    raw = read_regular(path)
    require(len(raw) >= 20 and raw[:4] == b"\x7fELF" and raw[4] == 2 and raw[5] == 1, "elf_architecture")
    machine = struct.unpack_from("<H", raw, 18)[0]
    require(machine == ELF_MACHINE[arch], "elf_architecture")
    return raw


# ---------------------------------------------------------------- bases.json

def load_bases(path):
    doc = decode_json(read_regular(path, MAX_JSON))
    check_keys(doc, ["schema_version", "registry", "bases"])
    require(doc["schema_version"] == 1 and type(doc["schema_version"]) is int, "invalid_input")
    require(isinstance(doc["registry"], str), "invalid_input")
    bases = doc["bases"]
    require(isinstance(bases, dict) and set(bases) == set(BINARIES), "invalid_input")
    for item in bases.values():
        check_keys(item, ["repository", "dockerfile_target", "index_digest", "platforms",
                          "published_at", "source_sha"], optional=["stage_sha256"])
        if "stage_sha256" in item:
            match(item["stage_sha256"], DIGEST_RE)
        require(isinstance(item["platforms"], dict)
                and set(item["platforms"]) == {"linux/amd64", "linux/arm64"}, "invalid_input")
        # The exporter refuses a base that has not been published yet.
        match(item["index_digest"], DIGEST_RE)
        for value in item["platforms"].values():
            match(value, DIGEST_RE)
    return doc


def dockerfile_stage_sha256(dockerfile, target):
    """SHA-256 of the exact bytes from `FROM ... AS <target>` up to the next `FROM` line.

    Same algorithm as scripts/publish-bases.sh and the provenance test."""
    lines = read_regular(dockerfile, MAX_JSON).splitlines(keepends=True)
    suffix = b" AS " + target.encode("utf-8")
    start = next((i for i, line in enumerate(lines)
                  if line.startswith(b"FROM ") and line.rstrip(b"\r\n").endswith(suffix)), None)
    require(start is not None, "base_mismatch")
    end = next((j for j in range(start + 1, len(lines)) if lines[j].startswith(b"FROM ")), len(lines))
    return "sha256:" + sha256_hex(b"".join(lines[start:end]))


def verify_stage_hashes(bases, dockerfile):
    """With --dockerfile, refuse a base whose recorded stage hash differs from the Dockerfile."""
    for item in bases["bases"].values():
        require("stage_sha256" in item, "base_mismatch")
        require(dockerfile_stage_sha256(dockerfile, item["dockerfile_target"]) == item["stage_sha256"],
                "base_mismatch")


# -------------------------------------------------------------- base layouts

class BaseLayout:
    """A verified single-image view of one platform of a pulled OCI base layout."""

    def __init__(self, root):
        self.root = Path(root)

    def blob_path(self, digest):
        match(digest, DIGEST_RE, "base_mismatch")
        return self.root / "blobs" / "sha256" / digest.split(":")[1]

    def read_blob(self, digest, limit=MAX_JSON):
        path = self.blob_path(digest)
        require(path.is_file() and not path.is_symlink(), "base_mismatch")
        require(path.stat().st_size <= limit, "base_mismatch")
        raw = path.read_bytes()
        require("sha256:" + sha256_hex(raw) == digest, "base_mismatch")
        return raw

    def verify_blob_file(self, desc):
        path = self.blob_path(desc["digest"])
        require(path.is_file() and not path.is_symlink(), "base_mismatch")
        require(type(desc["size"]) is int and path.stat().st_size == desc["size"], "base_mismatch")
        require("sha256:" + file_sha256(path) == desc["digest"], "base_mismatch")
        return path

    def platform(self, index_digest, platform_digests):
        """Return {arch: (manifest_digest, config_obj, layer_descriptors)}.

        Structurally malformed (but valid JSON) layouts are refused as base_mismatch."""
        try:
            return self._platform(index_digest, platform_digests)
        except (KeyError, TypeError, AttributeError, ValueError):
            raise ExportError("base_mismatch") from None

    def _platform(self, index_digest, platform_digests):
        require((self.root / "oci-layout").is_file(), "base_mismatch")
        try:
            layout = decode_json(read_regular(self.root / "oci-layout"), "base_mismatch")
            top = decode_json(read_regular(self.root / "index.json", MAX_JSON), "base_mismatch")
        except ExportError:
            raise ExportError("base_mismatch") from None
        require(layout.get("imageLayoutVersion") == "1.0.0", "base_mismatch")
        entries = [m for m in top.get("manifests", []) if isinstance(m, dict) and m.get("digest") == index_digest]
        require(len(entries) == 1, "base_mismatch")
        require(entries[0].get("mediaType") == OCI_INDEX, "base_mismatch")
        index = decode_json(self.read_blob(index_digest), "base_mismatch")
        require(index.get("schemaVersion") == 2 and index.get("mediaType") == OCI_INDEX, "base_mismatch")
        children = index.get("manifests")
        require(isinstance(children, list), "base_mismatch")
        result = {}
        for arch in PLATFORMS:
            wanted = platform_digests["linux/" + arch]
            found = [c for c in children if isinstance(c, dict) and c.get("digest") == wanted]
            require(len(found) == 1, "base_mismatch")
            child = found[0]
            platform = child.get("platform")
            require(isinstance(platform, dict) and platform.get("os") == "linux"
                    and platform.get("architecture") == arch, "base_mismatch")
            require(child.get("mediaType") == OCI_MANIFEST, "base_mismatch")
            manifest = decode_json(self.read_blob(wanted), "base_mismatch")
            require(manifest.get("schemaVersion") == 2 and manifest.get("mediaType") == OCI_MANIFEST,
                    "base_mismatch")
            config_desc = manifest.get("config")
            require(isinstance(config_desc, dict) and config_desc.get("mediaType") == OCI_CONFIG, "base_mismatch")
            config_raw = self.read_blob(config_desc["digest"])
            require(len(config_raw) == config_desc.get("size"), "base_mismatch")
            config = decode_json(config_raw, "base_mismatch")
            require(config.get("os") == "linux" and config.get("architecture") == arch, "base_mismatch")
            layers = manifest.get("layers")
            require(isinstance(layers, list) and 0 < len(layers) <= 127, "base_mismatch")
            clean = []
            for layer in layers:
                require(isinstance(layer, dict) and layer.get("mediaType") in OCI_BASE_LAYERS, "base_mismatch")
                self.verify_blob_file(layer)
                clean.append({"mediaType": layer["mediaType"], "digest": layer["digest"], "size": layer["size"]})
            rootfs = config.get("rootfs")
            require(isinstance(rootfs, dict) and isinstance(rootfs.get("diff_ids"), list)
                    and len(rootfs["diff_ids"]) == len(clean), "base_mismatch")
            result[arch] = (wanted, config, clean)
        return result


# ---------------------------------------------------------------- layer/image

def build_layer(name, content):
    """Deterministic USTAR tar with the single entry usr/local/bin/<name>."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        info = tarfile.TarInfo("usr/local/bin/" + name)
        info.size = len(content)
        info.mode = 0o755
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        info.mtime = 0
        info.type = tarfile.REGTYPE
        tar.addfile(info, io.BytesIO(content))
    return buffer.getvalue()


def image_config(base_config, role, diff_id, source_sha, source_url):
    config = json.loads(json.dumps(base_config))  # deep copy
    base_runtime = base_config.get("config") if isinstance(base_config.get("config"), dict) else {}
    labels = dict(base_runtime.get("Labels") or {})
    labels[REVISION] = source_sha
    labels[SOURCE] = source_url
    runtime = {
        "User": "10001:10001",
        "Entrypoint": ["/usr/local/bin/" + BINARIES[role]],
        "WorkingDir": "/",
        "Labels": labels,
    }
    if role == "service":
        runtime["Cmd"] = ["serve"]
    if base_runtime.get("Env"):
        runtime["Env"] = list(base_runtime["Env"])
    config["config"] = runtime
    config["rootfs"] = dict(config["rootfs"])
    config["rootfs"]["diff_ids"] = list(config["rootfs"]["diff_ids"]) + [diff_id]
    history = list(config.get("history") or [])
    history.append({"created_by": "scripts/oci_export.py: add /usr/local/bin/" + BINARIES[role],
                    "comment": "anchorfs binary layer"})
    config["history"] = history
    return config


class Layout:
    """Accumulates blobs in memory/on disk and writes an OCI layout directory."""

    def __init__(self, root):
        self.root = Path(root)
        (self.root / "blobs" / "sha256").mkdir(parents=True)
        self.written = set()

    def put(self, raw):
        digest = sha256_hex(raw)
        if digest not in self.written:
            (self.root / "blobs" / "sha256" / digest).write_bytes(raw)
            self.written.add(digest)
        return "sha256:" + digest

    def copy_blob(self, source, digest):
        name = digest.split(":")[1]
        if name not in self.written:
            shutil.copyfile(source, self.root / "blobs" / "sha256" / name)
            self.written.add(name)

    def finish(self, root_descriptor):
        (self.root / "oci-layout").write_bytes(canonical({"imageLayoutVersion": "1.0.0"}))
        (self.root / "index.json").write_bytes(canonical({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX,
            "manifests": [root_descriptor],
        }))


def export_image(out_dir, role, base_view, base_platforms, binaries, source_sha, source_url, evidence):
    layout = Layout(out_dir)
    children = []
    binary_digests = {}
    for arch in PLATFORMS:
        _, base_config, base_layers = base_platforms[arch]
        for layer in base_layers:
            layout.copy_blob(base_view.blob_path(layer["digest"]), layer["digest"])
        content = binaries[arch]
        binary_digests["linux/" + arch] = "sha256:" + sha256_hex(content)
        layer_raw = build_layer(BINARIES[role], content)
        layer_desc = descriptor(OCI_LAYER, layer_raw)
        layout.put(layer_raw)
        # Uncompressed layer: the diff ID equals the blob digest.
        config = image_config(base_config, role, layer_desc["digest"], source_sha, source_url)
        config_raw = canonical(config)
        config_desc = descriptor(OCI_CONFIG, config_raw)
        layout.put(config_raw)
        manifest = {
            "schemaVersion": 2,
            "mediaType": OCI_MANIFEST,
            "config": config_desc,
            "layers": base_layers + [layer_desc],
            "annotations": {REVISION: source_sha},
        }
        manifest_raw = canonical(manifest)
        manifest_desc = descriptor(OCI_MANIFEST, manifest_raw)
        layout.put(manifest_raw)
        # Platform is built from scratch: never copy variant or other base fields.
        manifest_desc["platform"] = {"os": "linux", "architecture": arch}
        children.append(manifest_desc)
    index = {
        "schemaVersion": 2,
        "mediaType": OCI_INDEX,
        "manifests": children,
        "annotations": {REVISION: source_sha},
    }
    index_raw = canonical(index)
    index_desc = descriptor(OCI_INDEX, index_raw)
    layout.put(index_raw)
    layout.finish(index_desc)
    evidence["binaries"][role] = binary_digests
    return index_desc["digest"]


# ---------------------------------------------------------------------- chart

def parse_flat_yaml(raw):
    """Parse a flat `key: value` Chart.yaml; anything nested or multi-line is rejected."""
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        raise ExportError("chart_invalid") from None
    result = {}
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        require(line == line.lstrip() and not stripped.startswith("-"), "chart_invalid")
        key, sep, value = line.partition(":")
        require(sep == ":" and re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", key) is not None, "chart_invalid")
        require(key not in result, "chart_invalid")
        value = value.strip()
        require(value != "", "chart_invalid")  # `key:` introduces a nested block
        if value[0] in "\"'":
            quote = value[0]
            require(len(value) >= 2 and value[-1] == quote and quote not in value[1:-1], "chart_invalid")
            require("\\" not in value, "chart_invalid")
            value = value[1:-1]
        else:
            require(value[0] not in "|>[{&*!%@`", "chart_invalid")
            value = re.split(r"\s#", value, maxsplit=1)[0].strip()
            require(value != "", "chart_invalid")
        result[key] = value
    return result


def read_chart(package, chart_dir):
    package = Path(package)
    raw_package = read_regular(package, 64 * 1024 * 1024, "chart_invalid")
    chart_yaml = None
    try:
        with tarfile.open(fileobj=io.BytesIO(raw_package), mode="r:gz") as tar:
            members = tar.getmembers()
            require(members, "chart_invalid")
            for member in members:
                parts = member.name.split("/")
                require(not member.name.startswith("/") and all(p not in ("", "..") for p in parts[1:])
                        and ".." not in parts, "chart_invalid")
                require(parts[0] == "anchorfs", "chart_invalid")
                require(member.isfile() or member.isdir(), "chart_invalid")
                if member.name == "anchorfs/Chart.yaml":
                    require(member.isfile() and member.size <= MAX_CHART_YAML, "chart_invalid")
                    chart_yaml = tar.extractfile(member).read()
    except (tarfile.TarError, OSError, EOFError):
        raise ExportError("chart_invalid") from None
    require(chart_yaml is not None, "chart_invalid")
    on_disk = read_regular(Path(chart_dir) / "Chart.yaml", MAX_CHART_YAML, "chart_invalid")
    fields = parse_flat_yaml(chart_yaml)
    # `helm package` re-serializes Chart.yaml (drops comments, reorders keys, requotes
    # scalars), so the packaged file is compared to the source by parsed fields, not bytes.
    require(parse_flat_yaml(on_disk) == fields, "chart_invalid")
    require("name" in fields and "version" in fields, "chart_invalid")
    config = {key: fields[key] for key in CHART_SCALARS if key in fields}
    return raw_package, config


def export_chart(out_dir, raw_package, chart_config, source_sha):
    layout = Layout(out_dir)
    config_raw = canonical(chart_config)
    config_desc = descriptor(HELM_CONFIG, config_raw)
    layout.put(config_raw)
    layer_desc = descriptor(HELM_LAYER, raw_package)
    layout.put(raw_package)
    manifest = {
        "schemaVersion": 2,
        "mediaType": OCI_MANIFEST,
        "config": config_desc,
        "layers": [layer_desc],
        "annotations": {
            REVISION: source_sha,
            "org.opencontainers.image.title": chart_config["name"],
            "org.opencontainers.image.version": chart_config["version"],
        },
    }
    manifest_raw = canonical(manifest)
    manifest_desc = descriptor(OCI_MANIFEST, manifest_raw)
    layout.put(manifest_raw)
    layout.finish(manifest_desc)
    return manifest_desc["digest"]


# --------------------------------------------------------------------- driver

def _after_layouts(output):
    """Hook between layout writing and receipt writing (tests inject failures here)."""


def write_sums(output):
    lines = []
    for path in sorted(p for p in output.rglob("*") if p.is_file()):
        rel = path.relative_to(output).as_posix()
        if rel in ("SHA256SUMS", "build.json"):
            continue
        lines.append(f"{file_sha256(path)}  {rel}\n")
    (output / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")


def export(args):
    source_sha = match(args.source_sha, SHA_RE)
    run_id = match(args.run_id, RUN_ID_RE)
    builder_image = match(args.builder_image, BUILDER_RE)
    toolchain_digest = match(args.toolchain_digest, DIGEST_RE)
    app_id = match(args.app_id, APP_ID_RE)
    source_repository = match(args.source_repository, REPO_RE)
    source_url = "https://github.com/" + source_repository

    output = Path(args.output)
    base_layouts = Path(args.base_layouts)
    binaries_dir = Path(args.binaries)
    for path in (base_layouts, binaries_dir, Path(args.chart_dir)):
        require(path.is_dir(), "invalid_input")
    require(not output.exists() and not output.is_symlink(), "output_exists")
    resolved = output.resolve()
    for guarded in (base_layouts, binaries_dir):
        require(not resolved.is_relative_to(guarded.resolve()), "invalid_input")

    for path in (Path(args.bases), base_layouts, binaries_dir, Path(args.chart_package), Path(args.chart_dir)):
        check_no_symlinks(path)

    bases = load_bases(args.bases)
    if args.dockerfile is not None:
        verify_stage_hashes(bases, args.dockerfile)

    binaries = {}
    for role, name in BINARIES.items():
        binaries[role] = {}
        for arch in PLATFORMS:
            binaries[role][arch] = check_elf(binaries_dir / f"linux-{arch}" / name, arch)

    chart_raw, chart_config = read_chart(args.chart_package, args.chart_dir)

    views = {}
    for role in BINARIES:
        view = BaseLayout(base_layouts / role)
        item = bases["bases"][role]
        views[role] = (view, view.platform(item["index_digest"], item["platforms"]))

    evidence = {
        "schema_version": 1,
        "source_sha": source_sha,
        "python": ".".join(str(v) for v in sys.version_info[:3]),
        "bases": {role: {"index_digest": bases["bases"][role]["index_digest"],
                         "platforms": bases["bases"][role]["platforms"]} for role in BINARIES},
        "binaries": {},
        "chart": {"name": chart_config["name"], "version": chart_config["version"],
                  "package_sha256": "sha256:" + sha256_hex(chart_raw)},
    }

    try:
        output.mkdir(parents=True)
    except FileExistsError:
        raise ExportError("output_exists") from None
    try:
        artifacts = {}
        for role in BINARIES:
            view, platforms = views[role]
            digest = export_image(output / role, role, view, platforms, binaries[role],
                                  source_sha, source_url, evidence)
            artifacts[role] = {"layout": role, "digest": digest}
        artifacts["chart"] = {"layout": "chart",
                              "digest": export_chart(output / "chart", chart_raw, chart_config, source_sha)}
        _after_layouts(output)
        (output / "export-evidence.json").write_bytes(canonical(evidence))
        write_sums(output)
        receipt = {
            "schema_version": SCHEMA_VERSION,
            "app_id": app_id,
            "source_repository": source_repository,
            "source_sha": source_sha,
            "run_id": run_id,
            "builder_image": builder_image,
            "toolchain_digest": toolchain_digest,
            "mode": "production",
            "artifacts": artifacts,
        }
        (output / "build.json").write_bytes(canonical(receipt))  # written last
    except BaseException:
        shutil.rmtree(output, ignore_errors=True)
        raise
    return receipt


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ExportError("invalid_input")


def parse_args(argv):
    p = _Parser(description="Export AnchorFS OCI layouts and a v1 build receipt.")
    for flag in ("--source-sha", "--run-id", "--builder-image", "--toolchain-digest", "--bases",
                 "--base-layouts", "--binaries", "--chart-package", "--chart-dir", "--output"):
        p.add_argument(flag, required=True)
    p.add_argument("--dockerfile", default=None,
                   help="optional: verify each base's recorded stage_sha256 against this Dockerfile")
    p.add_argument("--app-id", default="anchorfs")
    p.add_argument("--source-repository", default="mattsp1290/anchorfs")
    return p.parse_args(argv)


def main(argv=None):
    try:
        args = parse_args(sys.argv[1:] if argv is None else argv)
        export(args)
    except ExportError as exc:
        print(f"oci_export: {exc.code}", file=sys.stderr)
        return 2
    except (OSError, KeyError, TypeError, AttributeError, ValueError):
        print("oci_export: invalid_input", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())

"""Unit tests for scripts/oci_export.py using only synthetic fixtures.

Run: python3 -I -m unittest discover -s scripts/tests -v   (from the repo root)
Fixtures: 64-byte ELF headers, tiny two-platform OCI base layouts (the arm64
descriptor carries variant v8, like buildx output), and a tiny flat-Chart.yaml
chart tgz. No registry, daemon, network or real binary is involved.
"""
import contextlib
import hashlib
import importlib.util
import io
import json
import struct
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest import mock

MODULE_PATH = Path(__file__).resolve().parent.parent / "oci_export.py"
spec = importlib.util.spec_from_file_location("oci_export", MODULE_PATH)
oci_export = importlib.util.module_from_spec(spec)
spec.loader.exec_module(oci_export)

SHA = "a" * 40
BUILDER = "rust@sha256:" + "b" * 64
TOOLCHAIN = "sha256:" + "c" * 64
REVISION = "org.opencontainers.image.revision"
MACHINE = {"amd64": 0x3E, "arm64": 0xB7}
DOCKERFILE = (b"FROM rust:1 AS builder\nRUN true\n"
              b"FROM debian AS service-base\nRUN a\n"
              b"FROM service-base AS service\nCOPY x y\n"
              b"FROM debian AS mount-base\nRUN b\n")
CHART_YAML = (b"apiVersion: v2\nname: anchorfs\ndescription: Synthetic chart\ntype: application\n"
              b"version: 0.1.0\nappVersion: \"0.1.0\"\nkubeVersion: \">=1.32.0-0\"\n")


def sha(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def canon(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def elf(arch):
    header = bytearray(64)
    header[0:4] = b"\x7fELF"
    header[4], header[5], header[6] = 2, 1, 1
    struct.pack_into("<H", header, 18, MACHINE[arch])
    return bytes(header)


def put_blob(root, raw):
    d = sha(raw)
    (root / "blobs" / "sha256").mkdir(parents=True, exist_ok=True)
    (root / "blobs" / "sha256" / d.split(":")[1]).write_bytes(raw)
    return d


def make_base(root, name, layer_media=None):
    """Write a tiny OCI layout with a two-platform index; return the bases.json entry."""
    root.mkdir(parents=True)
    (root / "oci-layout").write_bytes(canon({"imageLayoutVersion": "1.0.0"}))
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as tar:
        info = tarfile.TarInfo("etc/synthetic")
        info.size = 4
        tar.addfile(info, io.BytesIO(b"base"))
    layer = buffer.getvalue()
    children, platforms = [], {}
    for arch in ("amd64", "arm64"):
        layer_digest = put_blob(root, layer + arch.encode())  # distinct bytes per platform
        config = {"architecture": arch, "os": "linux", "created": "2026-01-01T00:00:00Z",
                  "config": {"Env": ["PATH=/usr/bin"], "Labels": {"base": name}, "Cmd": ["bash"]},
                  "rootfs": {"type": "layers", "diff_ids": [layer_digest]},
                  "history": [{"created_by": "synthetic"}]}
        config_raw = canon(config)
        config_digest = put_blob(root, config_raw)
        manifest = {"schemaVersion": 2, "mediaType": oci_export.OCI_MANIFEST,
                    "config": {"mediaType": oci_export.OCI_CONFIG, "digest": config_digest, "size": len(config_raw)},
                    "layers": [{"mediaType": layer_media or oci_export.OCI_LAYER, "digest": layer_digest,
                                "size": len(layer) + len(arch)}]}
        manifest_raw = canon(manifest)
        manifest_digest = put_blob(root, manifest_raw)
        platform = {"os": "linux", "architecture": arch}
        if arch == "arm64":
            platform["variant"] = "v8"
        children.append({"mediaType": oci_export.OCI_MANIFEST, "digest": manifest_digest,
                         "size": len(manifest_raw), "platform": platform})
        platforms["linux/" + arch] = manifest_digest
    index_raw = canon({"schemaVersion": 2, "mediaType": oci_export.OCI_INDEX, "manifests": children})
    index_digest = put_blob(root, index_raw)
    (root / "index.json").write_bytes(canon({"schemaVersion": 2, "manifests": [
        {"mediaType": oci_export.OCI_INDEX, "digest": index_digest, "size": len(index_raw)}]}))
    return {"repository": "anchorfs/" + name + "-base", "dockerfile_target": name + "-base",
            "index_digest": index_digest, "platforms": platforms,
            "published_at": "2026-01-01T00:00:00Z", "source_sha": SHA}


def make_chart_tgz(path, chart_yaml=CHART_YAML, extra=()):
    with tarfile.open(path, "w:gz") as tar:
        for name, data in (("anchorfs/Chart.yaml", chart_yaml), ("anchorfs/values.yaml", b"a: b\n")):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        for info in extra:
            tar.addfile(info)


class Fixture:
    def __init__(self, root, chart_yaml=CHART_YAML):
        self.root = Path(root)
        self.layouts = self.root / "layouts"
        self.dist = self.root / "dist"
        self.chart_dir = self.root / "chart" / "anchorfs"
        self.chart_dir.mkdir(parents=True)
        (self.chart_dir / "Chart.yaml").write_bytes(chart_yaml)
        self.chart_tgz = self.root / "anchorfs-0.1.0.tgz"
        make_chart_tgz(self.chart_tgz, chart_yaml)
        self.bases_path = self.root / "bases.json"
        self.dockerfile_path = self.root / "Dockerfile"
        self.dockerfile_path.write_bytes(DOCKERFILE)
        bases = {"schema_version": 1, "registry": "10.0.0.106:30001",
                 "bases": {name: make_base(self.layouts / name, name) for name in ("service", "mount")}}
        for item in bases["bases"].values():
            item["stage_sha256"] = oci_export.dockerfile_stage_sha256(
                self.dockerfile_path, item["dockerfile_target"])
        self.write_bases(bases)
        for arch in ("amd64", "arm64"):
            d = self.dist / ("linux-" + arch)
            d.mkdir(parents=True)
            for name in ("anchorfs-service", "anchorfs-mount"):
                (d / name).write_bytes(elf(arch) + name.encode())

    def write_bases(self, bases):
        self.bases = bases
        self.bases_path.write_text(json.dumps(bases))

    def argv(self, output, **overrides):
        values = {"--source-sha": SHA, "--run-id": "12345", "--builder-image": BUILDER,
                  "--toolchain-digest": TOOLCHAIN, "--bases": str(self.bases_path),
                  "--base-layouts": str(self.layouts), "--binaries": str(self.dist),
                  "--chart-package": str(self.chart_tgz), "--chart-dir": str(self.chart_dir),
                  "--dockerfile": str(self.dockerfile_path), "--output": str(output)}
        values.update(overrides)
        return [item for pair in values.items() for item in pair]

    def run(self, output, **overrides):
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = oci_export.main(self.argv(output, **overrides))
        return code, err.getvalue().strip()


def read_json(path):
    return json.loads(Path(path).read_bytes())


def blob(layout, digest):
    return Path(layout) / "blobs" / "sha256" / digest.split(":")[1]


class ExportTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.fx = Fixture(self.tmp / "in")
        self.out = self.tmp / "out"

    def export_ok(self, output=None):
        code, err = self.fx.run(output or self.out)
        self.assertEqual((code, err), (0, ""))
        return read_json((output or self.out) / "build.json")

    def image_index(self, role):
        receipt = read_json(self.out / "build.json")
        layout = self.out / role
        root = read_json(layout / "index.json")
        self.assertEqual(root["manifests"][0]["digest"], receipt["artifacts"][role]["digest"])
        return layout, read_json(blob(layout, root["manifests"][0]["digest"]))

    def test_exports_two_images_and_chart(self):
        receipt = self.export_ok()
        self.assertIs(type(receipt["schema_version"]), int)
        self.assertEqual(receipt["schema_version"], 1)
        self.assertEqual(receipt["run_id"], "12345")
        self.assertEqual(receipt["mode"], "production")
        self.assertEqual(set(receipt["artifacts"]), {"service", "mount", "chart"})
        for role, item in receipt["artifacts"].items():
            self.assertEqual(item["layout"], role)
            root = read_json(self.out / role / "index.json")
            self.assertEqual(root["manifests"][0]["digest"], item["digest"])
            self.assertEqual(read_json(self.out / role / "oci-layout"), {"imageLayoutVersion": "1.0.0"})
            # every referenced blob exists with a matching digest; walk the graph
            pending = [root["manifests"][0]]
            seen = 0
            while pending:
                desc = pending.pop()
                raw = blob(self.out / role, desc["digest"]).read_bytes()
                self.assertEqual(sha(raw), desc["digest"])
                self.assertEqual(len(raw), desc["size"])
                seen += 1
                if desc["mediaType"] == oci_export.OCI_INDEX:
                    pending += json.loads(raw)["manifests"]
                elif desc["mediaType"] == oci_export.OCI_MANIFEST:
                    obj = json.loads(raw)
                    pending += [obj["config"]] + obj["layers"]
            self.assertGreaterEqual(seen, 3)
        self.assertTrue((self.out / "SHA256SUMS").is_file())
        self.assertTrue((self.out / "export-evidence.json").is_file())

    def test_image_config_and_manifests_carry_revision(self):
        self.export_ok()
        for role, entry in (("service", ["/usr/local/bin/anchorfs-service"]),
                            ("mount", ["/usr/local/bin/anchorfs-mount"])):
            layout, index = self.image_index(role)
            self.assertEqual(index["annotations"][REVISION], SHA)
            for child in index["manifests"]:
                manifest = read_json(blob(layout, child["digest"]))
                self.assertEqual(manifest["annotations"][REVISION], SHA)
                config = read_json(blob(layout, manifest["config"]["digest"]))
                runtime = config["config"]
                self.assertEqual(runtime["Labels"][REVISION], SHA)
                self.assertEqual(runtime["Labels"]["base"], role)
                self.assertEqual(runtime["User"], "10001:10001")
                self.assertEqual(runtime["Entrypoint"], entry)
                self.assertEqual(runtime["WorkingDir"], "/")
                self.assertEqual(runtime["Env"], ["PATH=/usr/bin"])
                if role == "service":
                    self.assertEqual(runtime["Cmd"], ["serve"])
                else:
                    self.assertNotIn("Cmd", runtime)
                self.assertEqual(len(config["rootfs"]["diff_ids"]), 2)
                self.assertEqual(len(config["history"]), 2)

    def test_index_has_exactly_two_platforms(self):
        # The synthetic base index carries variant v8 on arm64.
        base_index = read_json(blob(self.fx.layouts / "service", self.fx.bases["bases"]["service"]["index_digest"]))
        self.assertIn({"os": "linux", "architecture": "arm64", "variant": "v8"},
                      [m["platform"] for m in base_index["manifests"]])
        self.export_ok()
        for role in ("service", "mount"):
            layout, index = self.image_index(role)
            self.assertEqual(set(index), {"schemaVersion", "mediaType", "manifests", "annotations"})
            self.assertEqual([m["platform"] for m in index["manifests"]],
                             [{"os": "linux", "architecture": "amd64"}, {"os": "linux", "architecture": "arm64"}])
            for child in index["manifests"]:
                self.assertEqual(set(child), {"mediaType", "digest", "size", "platform"})
                manifest = read_json(blob(layout, child["digest"]))
                config = read_json(blob(layout, manifest["config"]["digest"]))
                self.assertEqual(config["architecture"], child["platform"]["architecture"])
                self.assertEqual(config["os"], "linux")

    def test_layer_is_single_deterministic_entry(self):
        self.export_ok()
        layout, index = self.image_index("service")
        for child in index["manifests"]:
            arch = child["platform"]["architecture"]
            manifest = read_json(blob(layout, child["digest"]))
            layer = manifest["layers"][-1]
            self.assertEqual(layer["mediaType"], oci_export.OCI_LAYER)
            raw = blob(layout, layer["digest"]).read_bytes()
            with tarfile.open(fileobj=io.BytesIO(raw)) as tar:
                members = tar.getmembers()
                self.assertEqual([m.name for m in members], ["usr/local/bin/anchorfs-service"])
                m = members[0]
                self.assertEqual((m.mode, m.uid, m.gid, m.mtime, m.uname, m.gname), (0o755, 0, 0, 0, "", ""))
                self.assertEqual(tar.extractfile(m).read(), elf(arch) + b"anchorfs-service")
            config = read_json(blob(layout, manifest["config"]["digest"]))
            self.assertEqual(config["rootfs"]["diff_ids"][-1], layer["digest"])
            self.assertEqual(len(manifest["layers"]), len(config["rootfs"]["diff_ids"]))

    def test_identical_inputs_produce_identical_digests(self):
        first = self.export_ok(self.tmp / "out1")
        second = self.export_ok(self.tmp / "out2")
        self.assertEqual((self.tmp / "out1" / "build.json").read_bytes(),
                         (self.tmp / "out2" / "build.json").read_bytes())
        self.assertEqual(first, second)
        self.assertEqual((self.tmp / "out1" / "SHA256SUMS").read_bytes(),
                         (self.tmp / "out2" / "SHA256SUMS").read_bytes())

    def test_rejects_wrong_architecture_binary(self):
        (self.fx.dist / "linux-arm64" / "anchorfs-mount").write_bytes(elf("amd64"))
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: elf_architecture"))
        self.assertFalse(self.out.exists())

    def test_rejects_base_digest_mismatch(self):
        bases = json.loads(json.dumps(self.fx.bases))
        bases["bases"]["mount"]["platforms"]["linux/arm64"] = "sha256:" + "d" * 64
        self.fx.write_bases(bases)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: base_mismatch"))
        bases = json.loads(json.dumps(self.fx.bases))
        bases["bases"]["mount"]["platforms"]["linux/arm64"] = self.fx.bases["bases"]["mount"]["platforms"]["linux/arm64"]
        bases["bases"]["service"]["index_digest"] = "sha256:" + "e" * 64
        self.fx.write_bases(bases)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: base_mismatch"))

    def test_rejects_null_bases(self):
        bases = json.loads(json.dumps(self.fx.bases))
        bases["bases"]["service"]["index_digest"] = None
        bases["bases"]["service"]["platforms"] = {"linux/amd64": None, "linux/arm64": None}
        self.fx.write_bases(bases)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: invalid_input"))

    def test_rejects_symlink_in_inputs(self):
        (self.fx.dist / "linux-amd64" / "extra").symlink_to(self.fx.dist / "linux-amd64" / "anchorfs-service")
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: invalid_input"))
        self.assertFalse(self.out.exists())

    def test_rejects_existing_output(self):
        self.out.mkdir()
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: output_exists"))

    def test_chart_manifest_uses_helm_media_types(self):
        receipt = self.export_ok()
        layout = self.out / "chart"
        root = read_json(layout / "index.json")
        self.assertEqual(len(root["manifests"]), 1)
        desc = root["manifests"][0]
        self.assertEqual(desc["digest"], receipt["artifacts"]["chart"]["digest"])
        self.assertEqual(desc["mediaType"], oci_export.OCI_MANIFEST)
        manifest = read_json(blob(layout, desc["digest"]))
        self.assertEqual(manifest["config"]["mediaType"], "application/vnd.cncf.helm.config.v1+json")
        self.assertEqual(len(manifest["layers"]), 1)
        self.assertEqual(manifest["layers"][0]["mediaType"], "application/vnd.cncf.helm.chart.content.v1.tar+gzip")
        self.assertEqual(blob(layout, manifest["layers"][0]["digest"]).read_bytes(), self.fx.chart_tgz.read_bytes())
        config = read_json(blob(layout, manifest["config"]["digest"]))
        self.assertEqual((config["name"], config["version"], config["appVersion"]), ("anchorfs", "0.1.0", "0.1.0"))
        self.assertEqual(config["kubeVersion"], ">=1.32.0-0")
        self.assertEqual(manifest["annotations"], {REVISION: SHA,
                                                   "org.opencontainers.image.title": "anchorfs",
                                                   "org.opencontainers.image.version": "0.1.0"})

    def test_rejects_nested_chart_yaml(self):
        nested = CHART_YAML + b"maintainers:\n  - name: someone\n"
        fx = Fixture(self.tmp / "nested", chart_yaml=nested)
        self.assertEqual(fx.run(self.tmp / "out-nested"), (2, "oci_export: chart_invalid"))
        self.assertFalse((self.tmp / "out-nested").exists())

    def test_receipt_written_last(self):
        with mock.patch.object(oci_export, "_after_layouts", side_effect=oci_export.ExportError("invalid_input")):
            code, err = self.fx.run(self.out)
        self.assertEqual((code, err), (2, "oci_export: invalid_input"))
        self.assertFalse((self.out / "build.json").exists())

    # ---- refusal paths -------------------------------------------------------

    def test_rejects_docker_media_type_in_base(self):
        import shutil
        shutil.rmtree(self.fx.layouts / "service")
        bases = json.loads(json.dumps(self.fx.bases))
        bases["bases"]["service"] = make_base(
            self.fx.layouts / "service", "service",
            layer_media="application/vnd.docker.image.rootfs.diff.tar.gzip")
        bases["bases"]["service"]["stage_sha256"] = self.fx.bases["bases"]["service"]["stage_sha256"]
        self.fx.write_bases(bases)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: base_mismatch"))

    def test_rejects_packaged_chart_yaml_differing_from_source(self):
        other = CHART_YAML.replace(b"version: 0.1.0", b"version: 0.2.0")
        make_chart_tgz(self.fx.chart_tgz, other)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: chart_invalid"))
        self.assertFalse(self.out.exists())

    def test_rejects_symlink_member_in_chart(self):
        link = tarfile.TarInfo("anchorfs/link")
        link.type = tarfile.SYMTYPE
        link.linkname = "/etc/passwd"
        make_chart_tgz(self.fx.chart_tgz, extra=[link])
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: chart_invalid"))

    def test_rejects_dotdot_member_in_chart(self):
        info = tarfile.TarInfo("anchorfs/../evil")
        info.size = 0
        make_chart_tgz(self.fx.chart_tgz, extra=[info])
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: chart_invalid"))

    def test_rejects_output_inside_binaries(self):
        inside = self.fx.dist / "export"
        self.assertEqual(self.fx.run(inside), (2, "oci_export: invalid_input"))
        self.assertFalse(inside.exists())

    def test_rejects_base_blob_whose_bytes_do_not_match_its_digest_name(self):
        item = self.fx.bases["bases"]["mount"]
        layout = self.fx.layouts / "mount"
        manifest = read_json(blob(layout, item["platforms"]["linux/amd64"]))
        victim = blob(layout, manifest["layers"][0]["digest"])
        victim.write_bytes(b"x" * len(victim.read_bytes()))  # same size, wrong content
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: base_mismatch"))

    def test_malformed_base_json_is_a_refusal_not_a_traceback(self):
        # index.json whose top level is a list
        (self.fx.layouts / "service" / "index.json").write_bytes(b"[]")
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: base_mismatch"))

    def test_manifest_without_config_digest_is_a_refusal(self):
        item = self.fx.bases["bases"]["service"]
        layout = self.fx.layouts / "service"
        bad = read_json(blob(layout, item["platforms"]["linux/amd64"]))
        del bad["config"]["digest"]
        raw = canon(bad)
        digest = put_blob(layout, raw)
        index = read_json(blob(layout, item["index_digest"]))
        for child in index["manifests"]:
            if child["platform"]["architecture"] == "amd64":
                child["digest"], child["size"] = digest, len(raw)
        index_raw = canon(index)
        index_digest = put_blob(layout, index_raw)
        (layout / "index.json").write_bytes(canon({"schemaVersion": 2, "manifests": [
            {"mediaType": oci_export.OCI_INDEX, "digest": index_digest, "size": len(index_raw)}]}))
        bases = json.loads(json.dumps(self.fx.bases))
        bases["bases"]["service"]["index_digest"] = index_digest
        bases["bases"]["service"]["platforms"]["linux/amd64"] = digest
        self.fx.write_bases(bases)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: base_mismatch"))

    # ---- base stage hash (drift detection) ----------------------------------

    def changed_dockerfile(self, extra):
        path = self.tmp / "Dockerfile.changed"
        path.write_bytes(DOCKERFILE.replace(b"RUN a\n", b"RUN a\n" + extra))
        return path

    def test_stage_hash_mismatch_is_refused(self):
        changed = self.changed_dockerfile(b"RUN added\n")
        self.assertEqual(self.fx.run(self.out, **{"--dockerfile": str(changed)}),
                         (2, "oci_export: base_mismatch"))
        self.assertFalse(self.out.exists())

    def test_missing_stage_hash_is_invalid_input(self):
        bases = json.loads(json.dumps(self.fx.bases))
        del bases["bases"]["mount"]["stage_sha256"]
        self.fx.write_bases(bases)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: invalid_input"))

    def test_malformed_stage_hash_is_invalid_input(self):
        bases = json.loads(json.dumps(self.fx.bases))
        bases["bases"]["service"]["stage_sha256"] = "sha256:short"
        self.fx.write_bases(bases)
        self.assertEqual(self.fx.run(self.out), (2, "oci_export: invalid_input"))

    def test_stage_hash_matches_python_and_stops_at_next_from(self):
        expected = "sha256:" + hashlib.sha256(b"FROM debian AS service-base\nRUN a\n").hexdigest()
        self.assertEqual(oci_export.dockerfile_stage_sha256(self.fx.dockerfile_path, "service-base"), expected)

    def test_default_dockerfile_is_the_repository_one(self):
        self.assertEqual(oci_export.DEFAULT_DOCKERFILE, MODULE_PATH.parent.parent / "docker" / "Dockerfile")


class SubcommandTests(unittest.TestCase):
    """The delivery primitives the shell scripts and the workflow call."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.fx = Fixture(self.tmp / "in")

    def call(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = oci_export.main(list(argv))
        return code, out.getvalue(), err.getvalue().strip()

    def test_stage_sha256_prints_digest(self):
        expected = oci_export.dockerfile_stage_sha256(self.fx.dockerfile_path, "mount-base")
        self.assertEqual(self.call("stage-sha256", str(self.fx.dockerfile_path), "mount-base"),
                         (0, expected + "\n", ""))

    def test_stage_sha256_unknown_stage_is_base_mismatch(self):
        self.assertEqual(self.call("stage-sha256", str(self.fx.dockerfile_path), "nope"),
                         (2, "", "oci_export: base_mismatch"))

    def test_repository_stage_hashes_match_committed_bases(self):
        root = MODULE_PATH.parent.parent
        committed = json.loads((root / "docker" / "bases.json").read_text())
        for item in committed["bases"].values():
            self.assertEqual(oci_export.dockerfile_stage_sha256(root / "docker" / "Dockerfile",
                                                                item["dockerfile_target"]),
                             item["stage_sha256"])

    def test_check_elf(self):
        good = self.fx.dist / "linux-amd64" / "anchorfs-service"
        self.assertEqual(self.call("check-elf", str(good), "amd64"), (0, "", ""))
        self.assertEqual(self.call("check-elf", str(good), "arm64"), (2, "", "oci_export: elf_architecture"))
        self.assertEqual(self.call("check-elf", str(good), "riscv"), (2, "", "oci_export: invalid_input"))
        bad = self.tmp / "text"
        bad.write_bytes(b"not an elf file at all, long enough")
        self.assertEqual(self.call("check-elf", str(bad), "amd64"), (2, "", "oci_export: elf_architecture"))
        big_endian = bytearray(elf("amd64"))
        big_endian[5] = 2
        bad.write_bytes(bytes(big_endian))
        self.assertEqual(self.call("check-elf", str(bad), "amd64"), (2, "", "oci_export: elf_architecture"))

    def test_bases_pull_refs_and_list(self):
        item = self.fx.bases["bases"]
        self.assertEqual(self.call("bases", "pull-refs", str(self.fx.bases_path)), (0, "".join(
            f"{role} 10.0.0.106:30001/anchorfs/{role}-base@{item[role]['index_digest']}\n"
            for role in ("service", "mount")), ""))
        self.assertEqual(self.call("bases", "registry", str(self.fx.bases_path)), (0, "10.0.0.106:30001\n", ""))
        self.assertEqual(self.call("bases", "list", str(self.fx.bases_path))[1].splitlines()[0],
                         f"service anchorfs/service-base service-base {item['service']['index_digest']}")

    def test_pull_refs_refuses_unpublished_but_list_reports_none(self):
        bases = json.loads(json.dumps(self.fx.bases))
        bases["bases"]["service"]["index_digest"] = None
        bases["bases"]["service"]["platforms"] = {"linux/amd64": None, "linux/arm64": None}
        self.fx.write_bases(bases)
        self.assertEqual(self.call("bases", "pull-refs", str(self.fx.bases_path)),
                         (2, "", "oci_export: invalid_input"))
        self.assertIn("service anchorfs/service-base service-base none",
                      self.call("bases", "list", str(self.fx.bases_path))[1])

    def readback_index(self, mutate=None):
        children = [{"mediaType": oci_export.OCI_MANIFEST, "digest": "sha256:" + c * 64, "size": 1,
                     "platform": {"os": "linux", "architecture": arch}}
                    for c, arch in (("1", "amd64"), ("2", "arm64"))]
        index = {"schemaVersion": 2, "manifests": children}
        if mutate:
            mutate(index)
        raw = canon(index)
        path = self.tmp / "index.manifest.json"
        path.write_bytes(raw)
        return sha(raw), path

    def test_verify_readback_index_and_platform(self):
        digest, path = self.readback_index()
        self.assertEqual(self.call("verify-readback", "index", digest, str(path)),
                         (0, f"linux/amd64 sha256:{'1' * 64}\nlinux/arm64 sha256:{'2' * 64}\n", ""))
        self.assertEqual(self.call("verify-readback", "platform", digest, str(path)), (0, "", ""))
        self.assertEqual(self.call("verify-readback", "platform", sha(b"other"), str(path)),
                         (2, "", "oci_export: readback_mismatch"))
        self.assertEqual(self.call("verify-readback", "index", sha(b"other"), str(path)),
                         (2, "", "oci_export: readback_mismatch"))
        self.assertEqual(self.call("verify-readback", "index", "sha256:xyz", str(path)),
                         (2, "", "oci_export: invalid_input"))

    def test_verify_readback_refuses_wrong_platform_set(self):
        digest, path = self.readback_index(lambda i: i["manifests"].pop())
        self.assertEqual(self.call("verify-readback", "index", digest, str(path)),
                         (2, "", "oci_export: readback_mismatch"))
        digest, path = self.readback_index(lambda i: i["manifests"][1]["platform"].update(architecture="amd64"))
        self.assertEqual(self.call("verify-readback", "index", digest, str(path)),
                         (2, "", "oci_export: readback_mismatch"))

    def test_layout_digest_requires_exactly_one_entry(self):
        layout = self.fx.layouts / "service"
        self.assertEqual(self.call("layout-digest", str(layout / "index.json")),
                         (0, self.fx.bases["bases"]["service"]["index_digest"] + "\n", ""))
        two = self.tmp / "two.json"
        two.write_bytes(canon({"manifests": [{"digest": "sha256:" + "1" * 64}, {"digest": "sha256:" + "2" * 64}]}))
        self.assertEqual(self.call("layout-digest", str(two)), (2, "", "oci_export: readback_mismatch"))

    def test_bases_record_writes_readback_results(self):
        digest, path = self.readback_index()
        stage = "sha256:" + "9" * 64
        new_sha = "b" * 40
        code, out, err = self.call("bases", "record", str(self.fx.bases_path), "mount", digest, str(path),
                                   stage, new_sha)
        self.assertEqual((code, err), (0, ""))
        self.assertIn(f"mount: old {self.fx.bases['bases']['mount']['index_digest']}", out)
        self.assertIn(f"mount: new {digest}", out)
        written = read_json(self.fx.bases_path)
        mount = written["bases"]["mount"]
        self.assertEqual(mount["index_digest"], digest)
        self.assertEqual(mount["platforms"], {"linux/amd64": "sha256:" + "1" * 64,
                                              "linux/arm64": "sha256:" + "2" * 64})
        self.assertEqual((mount["stage_sha256"], mount["source_sha"]), (stage, new_sha))
        self.assertRegex(mount["published_at"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertEqual(written["bases"]["service"], self.fx.bases["bases"]["service"])
        self.assertTrue(self.fx.bases_path.read_text().endswith("}\n"))

    def test_bases_record_refuses_unverified_readback_without_writing(self):
        digest, path = self.readback_index()
        before = self.fx.bases_path.read_bytes()
        self.assertEqual(self.call("bases", "record", str(self.fx.bases_path), "mount", sha(b"x"), str(path),
                                   "sha256:" + "9" * 64, "b" * 40), (2, "", "oci_export: readback_mismatch"))
        self.assertEqual(self.call("bases", "record", str(self.fx.bases_path), "mount", digest, str(path),
                                   "sha256:" + "9" * 64, "short"), (2, "", "oci_export: invalid_input"))
        self.assertEqual(self.fx.bases_path.read_bytes(), before)

    def test_unknown_subcommand_arity_is_invalid_input(self):
        self.assertEqual(self.call("stage-sha256", "only-one"), (2, "", "oci_export: invalid_input"))
        self.assertEqual(self.call("bases", "bogus"), (2, "", "oci_export: invalid_input"))


if __name__ == "__main__":
    unittest.main()

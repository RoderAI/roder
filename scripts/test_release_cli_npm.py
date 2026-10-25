#!/usr/bin/env python3
"""Exercise npm tarballs, native-process transport, and publication guards offline."""
from __future__ import annotations

import importlib.util
import io
import json
from pathlib import Path
import selectors
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import unittest
import urllib.error
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("release_cli_npm", Path(__file__).with_name("release_cli_npm.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
COMMIT = "a" * 40


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="roder-npm-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "source"
        self.template = self.root / "crates/roder-cli/npm"
        shutil.copytree(release.ROOT / "crates/roder-cli/npm", self.template)
        version = json.loads((self.template / "package.json").read_text())["version"]
        self.version = version
        (self.root / "crates/roder-cli/Cargo.toml").write_text(f'[package]\nname = "roder"\nversion = "{version}"\n')
        shutil.copyfile(release.ROOT / "LICENCE", self.root / "LICENCE")
        self.artifacts = self.base / "artifacts"
        self.artifacts.mkdir()
        self.payload = ('''#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
if (args[0] === '--version') console.log('roder VERSION');
else if (args[0] === '--wait') { console.log('ready'); setInterval(() => {}, 1000); }
else if (args[0] === '--exit') process.exit(7);
else console.log(JSON.stringify({args, cwd:process.cwd(), env:process.env.RODER_TEST_VALUE, stdin:fs.readFileSync(0,'utf8')}));
'''.replace('VERSION', version)).encode()
        for target in release.TARGETS:
            self.archive(target)

    def archive(self, target, *, symlink=False):
        name = f"roder-{target}.tar.gz"
        path = self.artifacts / name
        with tarfile.open(path, "w:gz") as output:
            entry = tarfile.TarInfo(f"roder-{target}/roder")
            if symlink:
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/etc/passwd"
                output.addfile(entry)
            else:
                entry.size = len(self.payload)
                entry.mode = 0o755
                output.addfile(entry, io.BytesIO(self.payload))
        data = {"product":"roder", "version":self.version, "commit_sha":COMMIT,
                "target":target, "archive":name, "sha256":release.digest(path)}
        (self.artifacts / f"roder-{target}.json").write_text(json.dumps(data))
        (self.artifacts / (name + ".sha256")).write_text(f'{data["sha256"]}  {name}\n')

    def pack(self, name="output"):
        return release.build_package(self.artifacts, self.base / name, COMMIT, self.root)

    def registry_metadata(self, tarball):
        return {"name":release.NAME, "version":self.version, "roder":{"commit_sha":COMMIT},
                "dist":{"integrity":release.integrity(tarball)}}

    def install(self, tarball):
        prefix = self.base / "installation with spaces"
        subprocess.run(["npm", "install", "--global", "--ignore-scripts", "--offline", "--no-audit", "--no-fund",
                        "--prefix", str(prefix), str(tarball)], check=True, capture_output=True, text=True)
        return prefix / "bin/roder"

    def test_package_installs_without_scripts_and_preserves_process_transport(self):
        tarball = self.pack()
        self.assertEqual(release.package_identity(tarball), (self.version, COMMIT))
        binary = self.install(tarball)
        version = subprocess.check_output([str(binary), "--version"], text=True)
        self.assertEqual(version, f"roder {self.version}\n")
        import os
        env = {**os.environ, "RODER_TEST_VALUE":"inherited"}
        result = subprocess.run([str(binary), "exec", "two words", 'a"b', "$literal"], input="prompt\n",
                                cwd=self.base, env=env, capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(result.stdout), {"args":["exec", "two words", 'a"b', "$literal"],
                         "cwd":str(self.base.resolve()), "env":"inherited", "stdin":"prompt\n"})
        self.assertEqual(subprocess.run([str(binary), "--exit"], capture_output=True).returncode, 7)
        self.assertEqual(release.integrity(tarball), release.integrity(self.pack("second-output")))

    def test_launcher_forwards_termination(self):
        binary = self.install(self.pack())
        child = subprocess.Popen([str(binary), "--wait"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(lambda: child.poll() is None and child.kill())
        with selectors.DefaultSelector() as ready:
            ready.register(child.stdout, selectors.EVENT_READ)
            self.assertTrue(ready.select(5), "child did not start")
        self.assertEqual(child.stdout.readline(), "ready\n")
        child.send_signal(signal.SIGTERM)
        self.assertEqual(child.wait(timeout=5), -signal.SIGTERM)
        child.stdout.close()
        child.stderr.close()

    def test_missing_binary_has_actionable_error(self):
        binary = self.install(self.pack())
        platform = subprocess.check_output(["node", "-p", "process.platform+'-'+process.arch"], text=True).strip()
        native = binary.resolve().parent / platform / "roder"
        native.unlink()
        result = subprocess.run([str(binary), "--version"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("Reinstall @roderai/roder", result.stderr)

    def test_launcher_supports_only_the_three_shipped_targets(self):
        launcher = self.template / "bin/roder.cjs"
        code = '''const {binaryForPlatform:f}=require(process.argv[1]);
for (const [os,arch] of [['linux','x64'],['linux','arm64'],['darwin','arm64']]) if(!f(os,arch).endsWith(os+'-'+arch+'/roder'))process.exit(1);
for (const [os,arch] of [['win32','x64'],['darwin','x64']]) {try {f(os,arch);process.exit(1)}catch(e){if(!e.message.includes('does not support'))throw e}}
'''
        subprocess.run(["node", "-e", code, str(launcher)], check=True)

    def test_mixed_commit_missing_target_and_corrupt_archive_fail_closed(self):
        target = next(iter(release.TARGETS))
        path = self.artifacts / f"roder-{target}.json"
        data = json.loads(path.read_text()); data["commit_sha"] = "b" * 40; path.write_text(json.dumps(data))
        with self.assertRaisesRegex(RuntimeError, "Wrong release identity"):
            self.pack()
        self.archive(target)
        archive = self.artifacts / f"roder-{target}.tar.gz"
        archive.write_bytes(archive.read_bytes() + b"corrupt")
        with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
            self.pack("corrupt")
        self.archive(target); path.unlink()
        with self.assertRaises(FileNotFoundError):
            self.pack("missing")

    def test_symlinked_binary_is_rejected(self):
        self.archive(next(iter(release.TARGETS)), symlink=True)
        with self.assertRaisesRegex(RuntimeError, "regular Roder binary"):
            self.pack()

    def test_template_version_and_generated_output_location_are_checked(self):
        path = self.template / "package.json"
        data = json.loads(path.read_text()); data["version"] = "9.9.9"; path.write_text(json.dumps(data))
        with self.assertRaisesRegex(RuntimeError, "version/name disagree"):
            self.pack()
        with self.assertRaisesRegex(RuntimeError, "outside the checkout"):
            release.build_package(self.artifacts, self.root / "generated", COMMIT, self.root)

    def test_manifest_checks_archive_before_stamping_identity(self):
        target = next(iter(release.TARGETS))
        with patch.object(release, "cli_version", return_value=self.version):
            release.write_manifest(self.artifacts, target, COMMIT)
            (self.artifacts / f"roder-{target}.tar.gz.sha256").write_text("bad checksum\n")
            with self.assertRaisesRegex(RuntimeError, "checksum does not match"):
                release.write_manifest(self.artifacts, target, COMMIT)

    def test_publish_rerun_verifies_existing_integrity_without_writes(self):
        tarball = self.pack(); metadata = self.registry_metadata(tarball)
        with patch.object(release, "registry_version", return_value=metadata), patch.object(release.subprocess, "run") as run:
            release.publish_package(tarball); run.assert_not_called()
            metadata["dist"]["integrity"] = "sha512-wrong"
            with self.assertRaisesRegex(RuntimeError, "differs"):
                release.publish_package(tarball)
            run.assert_not_called()

    def test_publish_refuses_latest_downgrade_and_waits_after_one_write(self):
        tarball = self.pack(); metadata = self.registry_metadata(tarball)
        with patch.object(release, "registry_version", side_effect=[None,{"version":"99.0.0"}]), patch.object(release.subprocess, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "latest backwards"):
                release.publish_package(tarball)
            run.assert_not_called()
        with patch.object(release, "registry_version", side_effect=[None,None,None,metadata]), patch.object(release.subprocess, "run") as run, patch.object(release.time, "sleep") as sleep:
            release.publish_package(tarball)
            self.assertEqual(run.call_count, 1)
            self.assertIn("--ignore-scripts", run.call_args.args[0])
            sleep.assert_called_once_with(10)

    def test_publish_source_requires_clean_landed_release_and_master_workflow(self):
        with patch.object(release.subprocess, "check_output", side_effect=[COMMIT + "\n", "dirty"]):
            with self.assertRaisesRegex(RuntimeError, "clean source"):
                release.publication_source_check(COMMIT)
        with patch.object(release.subprocess, "check_output", return_value="b" * 40):
            with self.assertRaisesRegex(RuntimeError, "source commit differs"):
                release.publication_source_check(COMMIT)
        with patch.dict(release.os.environ, {"GITHUB_ACTIONS":"true", "GITHUB_REF":"refs/heads/feature"}), patch.object(release.subprocess, "check_output", side_effect=[COMMIT + "\n", ""]):
            with self.assertRaisesRegex(RuntimeError, "master workflow"):
                release.publication_source_check(COMMIT)
        with patch.dict(release.os.environ, {"GITHUB_ACTIONS":"true", "GITHUB_REF":"refs/heads/master"}), patch.object(release.subprocess, "check_output", side_effect=[COMMIT + "\n", ""]), patch.object(release.subprocess, "run") as run:
            run.return_value.returncode = 1
            with self.assertRaisesRegex(RuntimeError, "landed release"):
                release.publication_source_check(COMMIT)

    def test_registry_errors_are_not_treated_as_missing_versions(self):
        error = urllib.error.HTTPError("https://registry.npmjs.org", 401, "Unauthorized", {}, None)
        with patch.object(release.urllib.request, "urlopen", side_effect=error):
            with self.assertRaises(urllib.error.HTTPError):
                release.registry_version("1.2.3")


if __name__ == "__main__":
    unittest.main()

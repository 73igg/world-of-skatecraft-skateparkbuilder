"""Exercise the source-only gate against forced assets and intermediate commits."""
import contextlib
import hashlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("audit_source", Path(__file__).with_name("audit_source.py"))
audit_source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit_source)


class SourceAuditTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="skatecraft-source-audit-")
        self.root = Path(self.temporary.name).resolve()
        self.assertTrue(self.root.is_relative_to(Path(tempfile.gettempdir()).resolve()))
        self.saved_root = audit_source.ROOT
        audit_source.ROOT = self.root
        self.git("init", "--quiet")
        self.git("config", "user.name", "Source audit test")
        self.git("config", "user.email", "source-audit@example.invalid")
        self.git("config", "core.autocrlf", "false")

    def tearDown(self):
        audit_source.ROOT = self.saved_root
        self.temporary.cleanup()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], stderr=subprocess.PIPE)

    def stage(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        self.git("add", "--force", "--", name)

    def check(self, revisions=None):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return audit_source.audit(revisions or [None])

    def test_source_and_target_selection_module_pass(self):
        self.stage("crates/benilla-app/src/target/example.rs", b"pub fn example() {}\n")
        self.assertEqual(self.check(), 0)

    def test_game_archive_is_rejected_even_when_forced(self):
        self.stage("crates/benilla-app/assets/game.mpq", b"MPQ\x1a\0owned-game-data")
        self.assertEqual(self.check(), 1)

    def test_text_game_export_is_rejected(self):
        self.stage("crates/benilla-app/assets/rig.json", b'{"mesh": [0, 1, 2]}')
        self.assertEqual(self.check(), 1)

    def test_binary_disguised_as_source_is_rejected(self):
        self.stage("crates/benilla-app/src/disguised.rs", b"MPQ\x1a\0binary")
        self.assertEqual(self.check(), 1)

    def test_private_key_material_is_rejected_without_printing_it(self):
        header = b"-----BEGIN " + b"OPENSSH PRIVATE KEY-----"
        self.stage("tools/secret.py", header + b"\nfixture\n")
        errors = io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(errors):
            self.assertEqual(audit_source.audit([None]), 1)
        self.assertNotIn(header.decode(), errors.getvalue())

    def test_local_state_is_rejected(self):
        self.stage(".setup/account.toml", b'account = "fixture"\n')
        self.assertEqual(self.check(), 1)

    def test_media_requires_exact_approved_content(self):
        name = "docs/media/new-abilities.png"
        data = b"\x89PNG\r\n\x1a\n\0synthetic-test-image"
        previous = audit_source.APPROVED_MEDIA.copy()
        try:
            audit_source.APPROVED_MEDIA[name] = hashlib.sha256(data).hexdigest()
            self.stage(name, data)
            self.assertEqual(self.check(), 0)
            self.stage(name, data + b"different")
            self.assertEqual(self.check(), 1)
        finally:
            audit_source.APPROVED_MEDIA.clear()
            audit_source.APPROVED_MEDIA.update(previous)

    def test_asset_added_then_deleted_in_history_is_rejected(self):
        self.stage("README.md", b"Source-only test.\n")
        self.git("commit", "--quiet", "-m", "Source baseline")
        self.stage("crates/benilla-app/assets/game.mpq", b"MPQ\x1a\0data")
        self.git("commit", "--quiet", "-m", "Forbidden intermediate asset")
        bad = self.git("rev-parse", "HEAD").decode().strip()
        self.git("rm", "--quiet", "crates/benilla-app/assets/game.mpq")
        self.git("commit", "--quiet", "-m", "Remove asset")
        tip = self.git("rev-parse", "HEAD").decode().strip()
        self.assertEqual(self.check([tip]), 0)
        self.assertEqual(self.check([bad, tip]), 1)

    def test_approved_media_cannot_be_reused_as_a_source_blob(self):
        name = "docs/media/new-abilities.png"
        data = b"\x89PNG\r\n\x1a\n\0synthetic-test-image"
        previous = audit_source.APPROVED_MEDIA.copy()
        try:
            audit_source.APPROVED_MEDIA[name] = hashlib.sha256(data).hexdigest()
            self.stage(name, data)
            self.stage("tools/disguised.py", data)
            self.assertEqual(self.check(), 1)
        finally:
            audit_source.APPROVED_MEDIA.clear()
            audit_source.APPROVED_MEDIA.update(previous)


if __name__ == "__main__":
    unittest.main()

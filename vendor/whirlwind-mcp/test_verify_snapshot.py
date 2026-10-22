"""Check snapshot provenance using fixtures with Windows-sensitive name order."""
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


class SnapshotTests(unittest.TestCase):
    def test_portable_order_and_content_integrity(self):
        verifier = pathlib.Path(__file__).with_name("verify_snapshot.py").read_bytes()
        # Explicit byte order: Windows Path ordering would put README after examples.
        entries = [
            ("Cargo.toml", b'[package]\nversion = "0.1.0"\n'),
            ("README.md", b"Snapshot fixture\n"),
            ("examples/fixture.rs", b"fn main() {}\n"),
            ("src/lib.rs", b"// fixture\n"),
            ("verify_snapshot.py", verifier),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary) / "kit"
            digest = hashlib.sha256()
            for name, content in entries:
                file = root / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(content)
                digest.update(name.encode())
                digest.update(b"\0")
                digest.update(content)
            root.with_suffix(".json").write_text(json.dumps({
                "version": "0.1.0", "sha256": digest.hexdigest(),
            }))
            # Generated build and interpreter files are not part of the source.
            for name in ["Cargo.lock", "target/output", "__pycache__/cached.pyc"]:
                file = root / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(b"generated")
            command = [sys.executable, str(root / "verify_snapshot.py")]
            checked = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            (root / "src/lib.rs").write_bytes(b"// changed\n")
            changed = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(changed.returncode, 0)
            self.assertIn("Shared MCP snapshot changed", changed.stderr)


if __name__ == "__main__":
    unittest.main()

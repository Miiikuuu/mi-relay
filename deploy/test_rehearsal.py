from contextlib import closing, redirect_stderr
import importlib.util
import io
from pathlib import Path
import sqlite3
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("rehearsal", Path(__file__).with_name("rehearse-upgrade.py"))
rehearsal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rehearsal)


class RehearsalTests(unittest.TestCase):
    def test_artifacts_are_independent_copies_including_tmp_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            old, new = root / "old", root / "new"
            old.write_bytes(b"old binary")
            new.write_bytes(b"new binary")
            new.chmod(0o755)
            rehearsal.stage_artifacts(root / "artifacts", old, new)
            new.write_bytes(b"later build")
            self.assertEqual((root / "artifacts/old-server").read_bytes(), b"old binary")
            self.assertEqual((root / "artifacts/new-server").read_bytes(), b"new binary")
            self.assertEqual((root / "artifacts/new-server").stat().st_mode & 0o777, 0o755)

    def test_default_and_explicit_upgrade_versions(self):
        paths = ["--old-binary", "/old", "--new-binary", "/new"]
        args = rehearsal.parse_args(paths)
        self.assertEqual((args.old_schema, args.new_schema), (3, 4))
        args = rehearsal.parse_args(paths + ["--old-schema", "1", "--new-schema", "2"])
        self.assertEqual((args.old_schema, args.new_schema), (1, 2))
        for extra in (["--new-schema", "2"], ["--old-schema", "4"], ["--new-schema", "1"]):
            with self.subTest(extra=extra), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                rehearsal.parse_args(paths + extra)

    def test_read_only_snapshot_and_retention_compare_exact_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "state.sqlite3"
            with closing(sqlite3.connect(path)) as db:
                db.executescript("CREATE TABLE folders(id TEXT PRIMARY KEY, credential BLOB);"
                                 "INSERT INTO folders VALUES('test',x'010203'); PRAGMA user_version=2;")
            original = path.read_bytes()
            before = rehearsal.database_snapshot(path)
            self.assertEqual(path.read_bytes(), original)
            with closing(sqlite3.connect(path)) as db:
                db.executescript("CREATE TABLE directory_versions(id TEXT); PRAGMA user_version=3;")
            after = rehearsal.database_snapshot(path)
            rehearsal.require_retained(before, after, 3)
            with self.assertRaises(RuntimeError):
                rehearsal.require_retained(before, after, 2)
            with closing(sqlite3.connect(path)) as db:
                db.execute("UPDATE folders SET credential=x'040506'")
                db.commit()
            with self.assertRaises(RuntimeError):
                rehearsal.require_retained(before, rehearsal.database_snapshot(path), 3)

    def test_missing_database_is_not_created(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "absent.sqlite3"
            with self.assertRaises(sqlite3.OperationalError):
                rehearsal.database_snapshot(path)
            self.assertFalse(path.exists())

    def test_schema_four_allows_only_default_disconnection_column(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "state.sqlite3"
            with closing(sqlite3.connect(path)) as db:
                db.executescript("CREATE TABLE folders(id TEXT PRIMARY KEY, credential BLOB);"
                                 "INSERT INTO folders VALUES('test',x'010203'); PRAGMA user_version=3;")
            before = rehearsal.database_snapshot(path)
            with closing(sqlite3.connect(path)) as db:
                db.executescript("ALTER TABLE folders ADD COLUMN disconnected INTEGER NOT NULL DEFAULT 0;"
                                 "PRAGMA user_version=4;")
            rehearsal.require_retained(before, rehearsal.database_snapshot(path), 4)
            with closing(sqlite3.connect(path)) as db:
                db.executescript("UPDATE folders SET disconnected=1;")
            with self.assertRaises(RuntimeError):
                rehearsal.require_retained(before, rehearsal.database_snapshot(path), 4)
            with closing(sqlite3.connect(path)) as db:
                db.executescript("UPDATE folders SET disconnected=0, credential=x'040506';")
            with self.assertRaises(RuntimeError):
                rehearsal.require_retained(before, rehearsal.database_snapshot(path), 4)

    def test_foreign_key_violation_blocks_acceptance(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "state.sqlite3"
            with closing(sqlite3.connect(path)) as db:
                db.executescript("CREATE TABLE parent(id INTEGER PRIMARY KEY);"
                                 "CREATE TABLE child(id INTEGER REFERENCES parent(id));"
                                 "INSERT INTO child VALUES(42);")
            with self.assertRaises(RuntimeError):
                rehearsal.database_snapshot(path)


if __name__ == "__main__":
    unittest.main()

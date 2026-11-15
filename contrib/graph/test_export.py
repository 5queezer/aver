"""Offline, invented fixtures only: python3 -m unittest discover -s contrib/graph."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sqlite3
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("aver_graph_export", Path(__file__).with_name("export.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ExportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.db = Path(self.temp.name) / "invented ? memory.sqlite"
        with contextlib.closing(sqlite3.connect(self.db)) as conn, conn:
            conn.executescript("""
                CREATE TABLE claims(id INTEGER,subject TEXT,predicate TEXT,object TEXT,
                    provenance TEXT,confidence REAL,status TEXT,source_refs TEXT,scope TEXT,
                    object_kind TEXT,event_id INTEGER);
                CREATE TABLE episodic_events(id INTEGER,session_id TEXT,kind TEXT,payload TEXT,
                    source TEXT,agent_id TEXT,agent_kind TEXT,ts INTEGER,scope TEXT);
                CREATE TABLE contradictions(id INTEGER,claim_id INTEGER,reason TEXT,new_claim_id INTEGER);
            """)

    def test_empty_store_and_readonly_input(self):
        before = self.db.read_bytes()
        data = module.export(self.db)
        self.assertEqual(data["claims"], [])
        self.assertEqual(data["entities"], [])
        self.assertEqual(self.db.read_bytes(), before)
        with self.assertRaises(sqlite3.OperationalError):
            module.export(Path(self.temp.name) / "absent.sqlite")
        self.assertFalse((Path(self.temp.name) / "absent.sqlite").exists())

    def test_literals_are_facts_and_event_provenance_survives(self):
        literal = '<script>globalThis.unwanted = true</script>'
        with contextlib.closing(sqlite3.connect(self.db)) as conn, conn:
            conn.execute("INSERT INTO claims VALUES(?,?,?,?,?,?,?,?,?,?,?)", (1,"task:invented","has_estimate","-0.18","INFERRED",0.45,"ACTIVE",'["event:3"]',"global","literal",3))
            conn.execute("INSERT INTO claims VALUES(?,?,?,?,?,?,?,?,?,?,?)", (2,"task:invented","concerns",literal,"EXTRACTED",0.9,"ACTIVE",'[]',"global","literal",None))
            conn.execute("INSERT INTO episodic_events VALUES(?,?,?,?,?,?,?,?,?)", (3,"synthetic","decision",literal,"synthetic","model","LLM",1,"global"))
        data = module.export(self.db)
        self.assertEqual(data["entities"], [{"id":"task:invented","label":"task:invented"}])
        self.assertEqual(data["claims"][0]["event_ids"], [3])
        self.assertEqual(data["claims"][1]["object"], literal)
        self.assertEqual(data["events"][0]["payload"], literal)

    def test_cli_creates_private_export_without_overwriting(self):
        output = Path(self.temp.name) / "new.json"
        with patch.object(sys, "argv", ["export.py", str(self.db), str(output)]), contextlib.redirect_stdout(io.StringIO()) as stdout:
            module.main()
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
        self.assertEqual(json.loads(stdout.getvalue())["claims"], 0)
        with patch.object(sys, "argv", ["export.py", str(self.db), str(output)]), self.assertRaises(FileExistsError):
            module.main()

    def test_matches_committed_goldens_shared_with_rust(self):
        testdata = Path(__file__).with_name("testdata")
        for name in ("synthetic", "decisions", "projects"):
            with self.subTest(name):
                db = Path(self.temp.name) / f"{name}.sqlite"
                with contextlib.closing(sqlite3.connect(db)) as conn, conn:
                    conn.executescript((testdata / f"{name}.sql").read_text())
                expected = json.loads((testdata / f"{name}.json").read_text())
                self.assertEqual(module.export(db), expected)

    def test_legacy_object_kind_remains_entity_until_migrated(self):
        with contextlib.closing(sqlite3.connect(self.db)) as conn, conn:
            conn.execute("ALTER TABLE claims DROP COLUMN object_kind")
            conn.execute("ALTER TABLE claims DROP COLUMN event_id")
            conn.execute("INSERT INTO claims VALUES(?,?,?,?,?,?,?,?,?)", (1,"sample","concerns","An invented sentence.","EXTRACTED",0.9,"ACTIVE",'[]',"global"))
        data = module.export(self.db)
        self.assertEqual(data["claims"][0]["object_kind"], "entity")
        self.assertIsNone(data["claims"][0]["created_at"])
        self.assertEqual(data["predicates"], [])
        self.assertEqual(len(data["entities"]), 2)


if __name__ == "__main__":
    unittest.main()

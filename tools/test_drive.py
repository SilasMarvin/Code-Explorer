"""Tests for tools/drive's own logic: python3 -m unittest tools/test_drive.py"""
import importlib.machinery
import importlib.util
import json
import struct
import tempfile
import unittest
import zlib
from pathlib import Path

loader = importlib.machinery.SourceFileLoader("drive", str(Path(__file__).with_name("drive")))
drive = importlib.util.module_from_spec(importlib.util.spec_from_loader("drive", loader))
loader.exec_module(drive)


def summary(ok=False):
    return {
        "dir": "/s/x", "ok": ok,
        "checks": {
            "dots-apart": {"pass": False, "failed_in": ["02-a"]},
            "labels-near": {"pass": True, "failed_in": []},
        },
        "snaps": {"02-a": {"dots-apart": {"pass": False, "count": 3,
                                          "offenders": [{"functions": ["f", "g"]}]}}},
        "expects": [{"line": 4, "expect": "expect mode journey", "ok": False,
                     "why": "mode is browse, not journey"},
                    {"line": 5, "expect": "expect depth 1", "ok": True}],
        "notes": [{"step": 5, "line": 6, "do": "key x", "note": ["the search box has the keyboard"]}],
        "failed_steps": [{"step": 2, "line": 3, "do": "click f", "trouble": ["over g"]},
                         {"step": 3, "line": 4, "do": "expect mode journey",
                          "trouble": ["mode is browse"]}],
    }


class DriveTest(unittest.TestCase):
    def test_reads_view_args_from_the_header(self):
        drive.os.environ["CODEVIZ_DATA"] = "/data"
        self.assertEqual(drive.header_args("# a\n#: view $CODEVIZ_DATA/x.json --focus 'POST /u'\nsnap"),
                         ["view", "/data/x.json", "--focus", "POST /u"])
        self.assertEqual(drive.header_args("snap"), [])

    def test_turns_single_shot_flags_into_a_script(self):
        script, rest = drive.shot_script(["view", "d.json", "--hover", "f", "--keys", "x .", "--depth", "2"])
        self.assertEqual(script, "key x\nkey .\nhover f\nsnap\n")
        self.assertEqual(rest, ["view", "d.json", "--depth", "2"])
        script, rest = drive.shot_script(["view", "d.json", "--keys=i"])
        self.assertEqual((script, rest), ("key i\nsnap\n", ["view", "d.json"]))

    def test_gives_longer_scripts_more_time(self):
        short = drive.time_limit("snap")
        self.assertEqual(drive.time_limit("snap\nwait 5000\n# no"), short + drive.STEP_SECONDS + 5)
        self.assertEqual(drive.time_limit("snap\n" * 1000), drive.MOST_SECONDS)

    def test_reports_failed_steps_expects_and_checks(self):
        text = drive.report(summary())
        self.assertIn("FAIL", text.splitlines()[0])
        self.assertIn("step 2 (line 3) `click f`: over g", text)
        self.assertNotIn("step 3", text, "a failed expect shows once, as an expect")
        self.assertIn("expect failed, line 4 `expect mode journey`: mode is browse, not journey", text)
        self.assertIn('check dots-apart failed in 02-a (3 offenders in 02-a): {"functions": ["f", "g"]}', text)
        self.assertNotIn("labels-near", text)
        self.assertIn("note, step 5 (line 6) `key x`: the search box has the keyboard", text)

    def test_tells_a_window_not_drawn_from_a_failure(self):
        blank = dict(summary(ok=True), window_not_drawn=["02-a"])
        self.assertEqual(drive.verdict(blank), "NOT DRAWN")
        self.assertEqual(drive.verdict(dict(summary(), window_not_drawn=["02-a"])), "FAIL")
        self.assertEqual(drive.verdict(summary(ok=True)), "PASS")
        text = drive.report(blank)
        self.assertTrue(text.splitlines()[0].endswith("NOT DRAWN"))
        self.assertIn("window not drawn: 1 snaps have no PNG (02-a)", text)
        table = drive.table([("a", blank), ("b", summary(ok=True))])
        self.assertTrue(table.splitlines()[2].endswith("not drawn"))
        self.assertIn("2/2 scripts pass; 1 of them with the window not drawn", table)

    def test_tables_the_suite(self):
        text = drive.table([("a", summary()), ("b", summary(ok=True)),
                            ("c", {"ok": False, "error": "killed"})])
        rows = text.splitlines()
        self.assertTrue(rows[2].startswith("a ") and rows[2].endswith("FAIL"))
        self.assertIn("1 failed", rows[2])
        self.assertIn("1/2 ok", rows[2])
        self.assertIn("dots-apart", rows[2])
        self.assertTrue(rows[4].endswith("ERROR"))
        self.assertIn("1/3 scripts pass", text)
        self.assertIn("killed", text)

    def test_judges_the_faint_lines_by_their_pixels(self):
        ground, path = (15, 18, 23), [[20.0, 50.0], [100.0, 50.0], [180.0, 50.0]]
        snap = {"window": [200, 100], "state": {"mode": "browse", "expanded": None},
                "content": {"selection": None},
                "nodes": [], "labels": [], "modules": [],
                "module_arrows": [{"on_screen": True, "drawn": True, "path": path}]}

        def judged(lift):
            with tempfile.TemporaryDirectory() as tmp:
                out = Path(tmp)
                write_png(out / "01-open.png", 200, 100,
                          lambda x, y: tuple(c + lift for c in ground) if y == 50 else ground)
                (out / "01-open.json").write_text(json.dumps(snap))
                summary = {"ok": True, "snaps": {"01-open": {}}, "checks": {}}
                drive.judge_look(out, summary)
                return summary

        bright = judged(15)
        self.assertTrue(bright["ok"])
        self.assertEqual(bright["snaps"]["01-open"]["look"]["measured"],
                         {"ground": 18.7, "faint links": 15.0})
        dark = judged(2)
        self.assertFalse(dark["ok"])
        self.assertEqual(dark["checks"]["look"]["failed_in"], ["01-open"])
        self.assertEqual(dark["snaps"]["01-open"]["look"]["offenders"][0]["what"], "faint links")


def write_png(path, width, height, color):
    """An RGB PNG whose pixel (x, y) is color(x, y)."""
    rows = b"".join(b"\0" + bytes(c for x in range(width) for c in color(x, y)) for y in range(height))

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    unittest.main()

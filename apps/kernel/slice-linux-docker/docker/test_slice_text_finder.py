import csv
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("slice-text-finder.py")
TSV_FIELDS = [
    "level",
    "page_num",
    "block_num",
    "par_num",
    "line_num",
    "word_num",
    "left",
    "top",
    "width",
    "height",
    "conf",
    "text",
]


class SliceTextFinderTests(unittest.TestCase):
    def run_finder(self, query, rows):
        with tempfile.TemporaryDirectory() as root:
            tsv_path = Path(root) / "screen.tsv"
            with tsv_path.open("w", newline="", encoding="utf-8") as handle:
                writer = csv.DictWriter(handle, fieldnames=TSV_FIELDS, delimiter="\t")
                writer.writeheader()
                writer.writerows(rows)
            return subprocess.run(
                [sys.executable, str(SCRIPT), query, str(tsv_path)],
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
            )

    def test_emits_every_occurrence_in_reading_order_and_preserves_pixel_coordinates(self):
        completed = self.run_finder(
            "Open Room",
            [
                word(1, 1, 10, 20, 40, 20, "Open"),
                word(1, 2, 55, 20, 40, 20, "Room"),
                word(2, 1, 200, 400, 80, 40, "Open"),
                word(2, 2, 290, 400, 80, 40, "Room"),
            ],
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            [json.loads(line) for line in completed.stdout.splitlines()],
            [
                {
                    "text": "Open Room",
                    "left": 10,
                    "top": 20,
                    "width": 85,
                    "height": 20,
                    "center_x": 52,
                    "center_y": 30,
                },
                {
                    "text": "Open Room",
                    "left": 200,
                    "top": 400,
                    "width": 170,
                    "height": 40,
                    "center_x": 285,
                    "center_y": 420,
                },
            ],
        )

    def test_matches_non_english_text_case_insensitively(self):
        completed = self.run_finder(
            "grüße 世界",
            [
                word(1, 1, 30, 40, 70, 24, "Grüße"),
                word(1, 2, 110, 40, 48, 24, "世界"),
            ],
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        match = json.loads(completed.stdout)
        self.assertEqual(match["text"], "Grüße 世界")
        self.assertEqual(match["left"], 30)
        self.assertEqual(match["width"], 128)

    def test_returns_null_and_failure_for_absent_text(self):
        completed = self.run_finder(
            "Missing",
            [word(1, 1, 10, 20, 40, 20, "Visible")],
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        self.assertEqual(completed.stdout, "null\n")

    def test_slice_screen_returns_every_match_from_the_text_finder(self):
        with tempfile.TemporaryDirectory() as root_value:
            root = Path(root_value)
            fixture_tsv = root / "screen.tsv"
            with fixture_tsv.open("w", newline="", encoding="utf-8") as handle:
                writer = csv.DictWriter(handle, fieldnames=TSV_FIELDS, delimiter="\t")
                writer.writeheader()
                writer.writerows(
                    [
                        word(1, 1, 10, 20, 40, 20, "Open"),
                        word(1, 2, 55, 20, 40, 20, "Room"),
                        word(2, 1, 200, 400, 80, 40, "Open"),
                        word(2, 2, 290, 400, 80, 40, "Room"),
                    ]
                )
            shutil.copy(SCRIPT, root / SCRIPT.name)
            fake_bin = root / "bin"
            fake_bin.mkdir()
            write_executable(fake_bin / "xdpyinfo", "#!/bin/sh\nexit 0\n")
            write_executable(fake_bin / "pgrep", "#!/bin/sh\nprintf '123 fixture\\n'\n")
            write_executable(
                fake_bin / "tesseract",
                "#!/bin/sh\ncat \"$CHARIOX_TEST_TSV\"\n",
            )
            image = root / "screen.png"
            image.write_bytes(b"fixture")
            environment = os.environ.copy()
            environment.update(
                {
                    "CHARIOX_SLICE_ROOT": str(root),
                    "CHARIOX_SLICE_CHROME_PROFILE": str(root / "profile"),
                    "CHARIOX_TEST_TSV": str(fixture_tsv),
                    "PATH": f"{fake_bin}{os.pathsep}{environment['PATH']}",
                }
            )

            completed = subprocess.run(
                [
                    "bash",
                    str(Path(__file__).with_name("slice-screen.sh")),
                    "find-text",
                    "Open Room",
                    str(image),
                ],
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
                env=environment,
            )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(len(completed.stdout.splitlines()), 2, completed.stdout)


def word(line, number, left, top, width, height, text):
    return {
        "level": 5,
        "page_num": 1,
        "block_num": 1,
        "par_num": 1,
        "line_num": line,
        "word_num": number,
        "left": left,
        "top": top,
        "width": width,
        "height": height,
        "conf": 95,
        "text": text,
    }


def write_executable(path, content):
    path.write_text(content, encoding="utf-8")
    path.chmod(0o755)


if __name__ == "__main__":
    unittest.main()

import csv
import tempfile
import unittest
from pathlib import Path
from unittest import mock

try:
    import tkinter  # noqa: F401
    from polykara import gui
except ImportError:  # Python built without Tk
    gui = None


@unittest.skipIf(gui is None, "tkinter is not available")
class SongListTest(unittest.TestCase):
    def test_new_ids_come_from_the_title_and_stay_unique(self):
        self.assertEqual(gui.new_id("I Know You're Hurting", set()), "i-know-you-re-hurting")
        self.assertEqual(gui.new_id("Wanna", {"wanna"}), "wanna-2")
        self.assertEqual(gui.new_id("我们的纪念", set()), "song")

    def test_songs_round_trip_and_keep_unknown_columns(self):
        with tempfile.TemporaryDirectory() as folder:
            manifest = Path(folder) / "songs.csv"
            manifest.write_text("id,url,title,extra\nabc,https://x,Song,keep\n", encoding="utf-8")
            with mock.patch.object(gui, "MANIFEST", manifest):
                fields, songs = gui.read_songs()
                self.assertIn("voice", fields)
                songs.append({"id": "new", "url": "https://y", "title": "歌", "voice": "female"})
                gui.write_songs(fields, songs)
                with manifest.open(encoding="utf-8") as handle:
                    rows = list(csv.DictReader(handle))
        self.assertEqual([row["id"] for row in rows], ["abc", "new"])
        self.assertEqual(rows[0]["extra"], "keep")
        self.assertEqual(rows[1]["title"], "歌")


if __name__ == "__main__":
    unittest.main()

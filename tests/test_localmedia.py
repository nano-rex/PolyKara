import csv
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from polykara import localmedia
from polykara.localmedia import guess_from_name, guess_language, is_local


class NameTest(unittest.TestCase):
    def test_artist_and_title_from_file_names(self):
        cases = {
            "Raye - I Know You're Hurting (Official Video)": ("Raye", "I Know You're Hurting"),
            "01. Aiman Junaidi - Janji Suci": ("Aiman Junaidi", "Janji Suci"),
            "olivia_ong_-_如燕": ("olivia ong", "如燕"),
            "王力宏-唯一": ("王力宏", "唯一"),
            "Bermuda [Lyric Video]": ("", "Bermuda"),
            "07 Takkan Terganti": ("", "Takkan Terganti"),
        }
        for stem, expected in cases.items():
            self.assertEqual(guess_from_name(stem), expected, stem)

    def test_language_from_script(self):
        self.assertEqual([guess_language(text) for text in ("唯一", "クライアイ", "사랑", "Janji Suci")], ["zh", "ja", "ko", "en"])

    def test_local_paths_and_web_addresses(self):
        self.assertTrue(is_local("C:\\\\Music\\\\song.mp3"))
        self.assertTrue(is_local("media/song.mkv"))
        self.assertTrue(is_local("file:///home/me/song.mp4"))
        self.assertFalse(is_local("https://www.youtube.com/watch?v=x"))
        self.assertFalse(is_local(""))


class AddFilesTest(unittest.TestCase):
    def test_files_become_rows_once(self):
        with tempfile.TemporaryDirectory() as folder:
            home = Path(folder)
            manifest = home / "songs.csv"
            manifest.write_text("id,url,title,artist,lyricist,composer,producer,template,language,subtitle_langs,lyrics_file,word_timing_file,logo_file,prompt_ms\n", encoding="utf-8")
            media = home / "music"
            media.mkdir()
            (media / "Raye - Escapism.mp4").write_bytes(b"")
            (media / "周杰伦-晴天.mp3").write_bytes(b"")
            with mock.patch.object(localmedia, "MANIFEST", manifest), mock.patch.object(localmedia, "HOME", home), mock.patch.object(localmedia, "probe", return_value={"duration": 0.0, "tags": {}, "video": False, "cover": False, "audio": True}):
                added = localmedia.add_files(sorted(media.iterdir()))
                again = localmedia.add_files(sorted(media.iterdir()))
            with manifest.open(encoding="utf-8") as handle:
                rows = list(csv.DictReader(handle))
        self.assertEqual(len(added), 2)
        self.assertEqual(again, [])
        by_title = {row["title"]: row for row in rows}
        self.assertEqual(by_title["Escapism"]["artist"], "Raye")
        self.assertEqual(by_title["Escapism"]["id"], "raye-escapism")
        self.assertEqual(by_title["Escapism"]["url"], str(Path("music") / "Raye - Escapism.mp4"))
        self.assertEqual(by_title["晴天"]["artist"], "周杰伦")
        self.assertEqual(by_title["晴天"]["language"], "zh")
        self.assertIn("voice", rows[0])

    def test_tags_win_over_the_file_name(self):
        with mock.patch.object(localmedia, "probe", return_value={"duration": 200.0, "tags": {"title": "Real Title", "artist": "Real Artist"}, "video": True, "cover": False, "audio": True}):
            row = localmedia.describe(Path("/music/track01.mp3"), set())
        self.assertEqual((row["artist"], row["title"], row["id"]), ("Real Artist", "Real Title", "real-artist-real-title"))


@unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "needs ffmpeg")
class BringInTest(unittest.TestCase):
    def test_file_is_placed_in_raw_with_its_length(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "song.mp3"
            subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "sine=d=2", "-metadata", "title=Song", str(source)], check=True)
            raw = Path(folder) / "raw"
            target = localmedia.bring_in({"id": "demo", "url": str(source), "title": "Song"}, raw)
            self.assertEqual(target, raw / "demo.mp3")
            self.assertTrue(target.exists())
            info = json.loads((raw / "demo.info.json").read_text(encoding="utf-8"))
            self.assertAlmostEqual(info["duration"], 2.0, delta=0.2)
            self.assertEqual(localmedia.probe(source)["tags"]["title"], "Song")

    def test_missing_file_fails_for_that_song(self):
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaises(localmedia.SongError):
                localmedia.bring_in({"id": "demo", "url": str(Path(folder) / "nope.mp3")}, Path(folder))


class DemoSongsTest(unittest.TestCase):
    def test_old_demo_songs_are_dropped_and_others_kept(self):
        from polykara import app

        with tempfile.TemporaryDirectory() as folder:
            manifest = Path(folder) / "songs.csv"
            manifest.write_text("id,url,title\nexample,,Example\namazing-grace-vocal,https://www.youtube.com/watch?v=24cWZeB4ldE,Amazing Grace\nmine,https://x,Mine\n", encoding="utf-8")
            with mock.patch.object(app, "HOME", Path(folder)):
                app.drop_demo_songs()
            self.assertEqual(manifest.read_text(encoding="utf-8"), "id,url,title\nexample,,Example\nmine,https://x,Mine\n")


if __name__ == "__main__":
    unittest.main()

import tempfile
import unittest
from pathlib import Path
from unittest import mock

from polykara import library


class LibraryTest(unittest.TestCase):
    def test_translation_lines_are_merged(self):
        lines = library.merge_translations([(1000, 3000, "你好", []), (1000, 3000, "Hello", []), (3000, 5000, "再见", [])])
        self.assertEqual(len(lines), 2)
        self.assertEqual(lines[0]["translation"], "Hello")
        self.assertEqual(lines[0]["end"], 3000)

    def test_lrc_translation_pair_keeps_file_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "x.lrc"
            path.write_text("[00:01.00]月亮\n[00:01.00]The moon\n[00:04.00]<00:04.00>你<00:04.50>好<00:05.00>\n", encoding="utf-8")
            lines = library.merge_translations(library.lyric_source_entries(path))
        self.assertEqual((lines[0]["text"], lines[0]["translation"], lines[0]["end"]), ("月亮", "The moon", 4000))
        self.assertEqual(lines[1]["text"], "你好")

    def test_language_guess(self):
        self.assertEqual(library.guess_language("こんにちは"), "ja")
        self.assertEqual(library.guess_language("안녕"), "ko")
        self.assertEqual(library.guess_language("你好"), "zh")
        self.assertEqual(library.guess_language("hello"), "")

    def test_scan_finds_sidecar_and_stores_lyrics_outside_music(self):
        with tempfile.TemporaryDirectory() as tmp:
            music, work = Path(tmp) / "music", Path(tmp) / "work"
            music.mkdir()
            (music / "a.mp3").write_bytes(b"")
            (music / "a.lrc").write_text("[00:01.00]hi\n", encoding="utf-8")
            (music / "b.flac").write_bytes(b"")
            with mock.patch.object(library, "load_config", return_value={"player": {"library": [str(music)]}}), \
                    mock.patch.object(library, "probe", return_value={"title": "x", "artist": "", "album": "", "duration": 1.0, "video": False}), \
                    mock.patch.object(library, "WORK", work), \
                    mock.patch.object(library, "LIBRARY_CACHE", work / "library.json"), \
                    mock.patch.object(library, "FETCHED_LYRICS", work / "player-lyrics"):
                tracks = {Path(t["path"]).name: t for t in library.scan(Path(tmp))}
                self.assertTrue(tracks["a.mp3"]["lyrics"])
                self.assertFalse(tracks["b.flac"]["lyrics"])
                library.store_lyrics(tracks["b.flac"]["id"], ".krc", "[0,1000]<0,1000,0>好\n".encode("utf-8"))
                self.assertEqual(sorted(p.name for p in music.iterdir()), ["a.lrc", "a.mp3", "b.flac"])
                result = library.track_lyrics(tracks["b.flac"], online=False)
                self.assertEqual(result["lines"][0]["words"], [[0, 1000, "好"]])


if __name__ == "__main__":
    unittest.main()

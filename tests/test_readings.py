import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from polykara import readings
from polykara.romanize import romanize

HAS_PINYIN = importlib.util.find_spec("pypinyin") is not None
HAS_KAKASI = importlib.util.find_spec("pykakasi") is not None


class FormatTest(unittest.TestCase):
    def test_tone_marks_and_numbers_convert_both_ways(self):
        self.assertEqual([readings.numbered(value) for value in ("chóng", "chong2", "lǜ", "de", "nǚ3")], ["chong2", "chong2", "lv4", "de5", "nv3"])
        self.assertEqual([readings.marked(value) for value in ("chong2", "gui1", "shou3", "lv4", "de5", "xue4")], ["chóng", "guī", "shǒu", "lǜ", "de", "xuè"])
        self.assertEqual(readings.styled("chóng", "zh", "none"), "chong")
        self.assertEqual(readings.styled("ngan4", "yue", "none"), "ngan")

    def test_entries_need_one_syllable_per_character(self):
        words, problems = readings.parse("# comment\n重来 chóng lái\n银行 yin\n落下 luo4 xia4  # note\n", "zh")
        self.assertEqual(words, {"重来": ["chóng", "lái"], "落下": ["luo4", "xia4"]})
        self.assertEqual(len(problems), 1)
        self.assertEqual(readings.parse("君 きみ\n", "ja")[0], {"君": ["きみ"]})

    def test_longest_word_wins(self):
        words = {"重": ["zhong4"], "重来": ["chong2", "lai2"]}
        self.assertEqual(readings.matches("再重来", words), [(1, 3, ["chong2", "lai2"])])

    def test_builtin_lists_are_valid(self):
        for language in readings.LANGUAGES:
            text = (readings.BUILTIN / f"{language}.txt").read_text(encoding="utf-8")
            self.assertEqual(readings.parse(text, language)[1], [], language)


@unittest.skipUnless(HAS_PINYIN, "pypinyin is not installed")
class MandarinLibraryTest(unittest.TestCase):
    def test_library_words_fix_the_engine(self):
        self.assertEqual(romanize("我们重来", "zh", {"tone": "marks"}), "wǒ men chóng lái")
        self.assertEqual(romanize("慢慢地走", "zh", {"tone": "none"}), "man man de zou")

    def test_user_words_replace_builtin_ones(self):
        with tempfile.TemporaryDirectory() as folder:
            Path(folder, "readings").mkdir()
            Path(folder, "readings", "zh.txt").write_text("重来 zhong4 lai2\n", encoding="utf-8")
            with mock.patch.object(readings, "HOME", Path(folder)):
                self.assertEqual(romanize("重来", "zh", {"tone": "numbers"}), "zhong4 lai2")


@unittest.skipUnless(HAS_KAKASI, "pykakasi is not installed")
class JapaneseLibraryTest(unittest.TestCase):
    def test_kana_readings_and_particles(self):
        self.assertEqual(romanize("人は温もりを", "ja", {}), "hito wa nukumori o")
        self.assertTrue(romanize("君に会いたい", "ja", {}).startswith("kimi ni"))


if __name__ == "__main__":
    unittest.main()

import importlib.util
import unittest

from polykara.ass import ass
from polykara.romanize import engine_for, romanize, romanize_words
from polykara.subtitle import tokenise

SETTINGS = {"enabled": True, "languages": "auto", "tone": "numbers"}
ROW = {"id": "demo", "title": "Demo", "artist": "Artist", "lyricist": "L", "composer": "C", "producer": "P"}


def timed(text, step=500):
    return [(index * step, index * step + step - 100, token) for index, token in enumerate(tokenise(text))]


class EngineChoiceTest(unittest.TestCase):
    def test_script_decides_the_engine(self):
        self.assertEqual(engine_for("ありがとう", "en"), "ja")
        self.assertEqual(engine_for("사랑해", "en"), "ko")
        self.assertEqual(engine_for("你好", "zh"), "zh")
        self.assertEqual(engine_for("你好", "yue"), "yue")
        self.assertEqual(engine_for("東京", "ja"), "ja")
        self.assertEqual(engine_for("Привет", "ru"), "other")
        self.assertIsNone(engine_for("Hello world", "zh"))

    def test_disabled_or_unlisted_language_gives_one_row(self):
        self.assertIsNone(romanize("사랑해", "ko", {**SETTINGS, "enabled": False}))
        self.assertIsNone(romanize("사랑해", "ko", {**SETTINGS, "languages": ["zh"]}))
        self.assertEqual(romanize("사랑해", "ko", {**SETTINGS, "languages": ["ko"]}), "saranghae")


class KoreanTest(unittest.TestCase):
    def test_revised_romanization_with_carried_consonants(self):
        self.assertEqual(romanize("사랑해요 baby", "ko", SETTINGS), "saranghaeyo baby")
        self.assertEqual(romanize("먹어요 좋아", "ko", SETTINGS), "meogeoyo joa")
        self.assertEqual(romanize("한국", "ko", SETTINGS), "hanguk")

    def test_each_syllable_is_timed_from_its_character(self):
        words = timed("사랑 baby")
        self.assertEqual(romanize_words(words, "ko", SETTINGS), [(0, 400, "sa"), (500, 900, "rang "), (1000, 1400, "baby")])


@unittest.skipUnless(importlib.util.find_spec("pypinyin"), "pypinyin is not installed")
class MandarinTest(unittest.TestCase):
    def test_pinyin_per_character_with_latin_words_kept(self):
        self.assertEqual(romanize("我们, Hello 朋友！", "zh", SETTINGS), "wo3 men5, Hello peng2 you3!")
        self.assertEqual(romanize("我们长大了，女朋友", "zh", {**SETTINGS, "tone": "marks"}), "wǒ men zhǎng dà le, nǚ péng yǒu")
        words = timed("你好 世界")
        self.assertEqual([word for _, _, word in romanize_words(words, "zh", SETTINGS)], ["ni3 ", "hao3 ", "shi4 ", "jie4"])


@unittest.skipUnless(importlib.util.find_spec("pykakasi"), "pykakasi is not installed")
class JapaneseTest(unittest.TestCase):
    def test_a_word_spanning_several_characters_takes_their_whole_time(self):
        words = timed("東京へ")
        result = romanize_words(words, "ja", SETTINGS)
        self.assertEqual(result[0], (0, 900, "toukyou "))
        self.assertEqual(result[1][2], "e")


@unittest.skipUnless(importlib.util.find_spec("anyascii"), "anyascii is not installed")
class OtherScriptsTest(unittest.TestCase):
    def test_cyrillic(self):
        self.assertEqual(romanize("Привет мир", "ru", SETTINGS), "Privet mir")


def dialogue(text, style):
    return [line for line in text.splitlines() if line.startswith("Dialogue") and f",{style}," in line]


def position(line):
    return line.split("\\pos(")[1].split(")")[0].split(",")


class RubyLayoutTest(unittest.TestCase):
    ENTRY = [(20000, 24000, "사랑해", [(20000, 20500, "사"), (20500, 21000, "랑"), (21000, 21500, "해")])]

    def test_each_reading_is_centred_above_its_character(self):
        text = ass({**ROW, "language": "ko"}, self.ENTRY)
        base, roman = dialogue(text, "Lyric"), dialogue(text, "Romanization")
        self.assertEqual(len(base), 3)
        self.assertEqual(len(roman), 3)
        for character, reading in zip(base, roman):
            self.assertEqual(position(character)[0], position(reading)[0])
            self.assertLess(int(position(reading)[1]), int(position(character)[1]))
        xs = [int(position(line)[0]) for line in base]
        self.assertEqual(xs, sorted(xs))
        # Reading and character wait the same time and then fill over the same 0.5 s.
        self.assertTrue(roman[1].endswith("{\\kf50}rang"))
        self.assertTrue(base[1].endswith("{\\kf50}랑"))
        self.assertEqual(roman[1].split("}", 1)[1].rsplit("{", 1)[0], base[1].split("}", 1)[1].rsplit("{", 1)[0])

    def test_words_that_need_no_reading_get_none(self):
        text = ass({**ROW, "language": "ko"}, [(20000, 24000, "사랑 baby", [(20000, 20500, "사"), (20500, 21000, "랑 "), (21000, 21500, "baby")])])
        self.assertEqual(len(dialogue(text, "Lyric")), 3)
        self.assertEqual([line.rsplit("}", 1)[1] for line in dialogue(text, "Romanization")], ["sa", "rang"])

    def test_rows_layout_keeps_one_romanized_line(self):
        import polykara.ass as module
        original = module.load_config

        def rows_config():
            config = original()
            return {**config, "romanization": {**config["romanization"], "layout": "rows"}}

        module.load_config = rows_config
        try:
            text = ass({**ROW, "language": "ko"}, self.ENTRY)
        finally:
            module.load_config = original
        roman = dialogue(text, "Romanization")
        self.assertEqual(len(roman), 1)
        self.assertTrue(roman[0].endswith("{\\kf50}sa{\\kf50}rang{\\kf50}hae"))
        self.assertEqual(len(dialogue(text, "Lyric")), 1)

    def test_latin_lyrics_have_a_single_row(self):
        text = ass({**ROW, "language": "en"}, [(20000, 24000, "Hello", [(20000, 21000, "Hello")])])
        self.assertFalse(dialogue(text, "Romanization"))
        self.assertEqual(len(dialogue(text, "Lyric")), 1)


if __name__ == "__main__":
    unittest.main()

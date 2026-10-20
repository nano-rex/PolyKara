import tomllib
import unittest

from polykara import settings

SAMPLE = """# PolyKara settings
[layout]
# How lines are shown.
mode = "pairs"
lead_ms = 2000

[lyric]
size = 120
"""


class SettingsFileTest(unittest.TestCase):
    def test_values_change_in_place_and_comments_stay(self):
        text = settings.set_value(SAMPLE, "layout", "mode", "alternate")
        text = settings.set_value(text, "lyric", "bold", True)
        text = settings.set_value(text, "card", "title_size", 100)
        self.assertIn('# How lines are shown.\nmode = "alternate"\nlead_ms = 2000', text)
        self.assertIn("size = 120\nbold = true", text)
        data = tomllib.loads(text)
        self.assertEqual((data["layout"]["mode"], data["lyric"]["bold"], data["card"]["title_size"]), ("alternate", True, 100))

    def test_literals(self):
        self.assertEqual(settings.literal(['my', 'jp']), '["my", "jp"]')
        self.assertEqual(settings.literal('say "hi"'), '"say \\"hi\\""')
        self.assertEqual(settings.literal(0.6), "0.6")

    def test_comments_become_help(self):
        help_text = settings.comments(SAMPLE)
        self.assertEqual(help_text[("layout", "mode")], "How lines are shown.")
        self.assertNotIn(("layout", "lead_ms"), help_text)

    def test_ass_colours_convert(self):
        self.assertEqual(settings.ass_to_hex("&H000000FC"), "#fc0000")
        self.assertEqual(settings.hex_to_ass("#142d88", "80"), "&H80882D14")
        self.assertIsNone(settings.ass_to_hex("voice"))

    def test_shipped_file_reads_with_defaults(self):
        from polykara.config import ROOT

        values = settings.current_values((ROOT / "polykara.toml").read_text(encoding="utf-8"))
        self.assertIn("mode", values["layout"])


if __name__ == "__main__":
    unittest.main()

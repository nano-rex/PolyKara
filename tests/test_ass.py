import copy
import unittest
from unittest import mock

from polykara import ass as ass_module
from polykara.config import DEFAULT_CONFIG

ROW = {"id": "t", "title": "T", "artist": "A", "lyricist": "", "composer": "", "producer": "", "language": "en"}


def render(entries, **overrides):
    config = copy.deepcopy(DEFAULT_CONFIG)
    config["title"]["enabled"] = config["credit"]["enabled"] = False
    config["video"]["extend_intro"] = False
    for section, values in overrides.items():
        config[section].update(values)
    with mock.patch.object(ass_module, "load_config", return_value=config):
        return ass_module.ass(ROW, entries)


def lyric_lines(text: str) -> list[str]:
    return [line for line in text.splitlines() if line.startswith("Dialogue: 0,") and ",Lyric," in line]


class KaraokeTextTest(unittest.TestCase):
    def test_gap_between_words_becomes_silent_k(self):
        text = ass_module.karaoke_text([(1000, 1200, "a"), (1500, 1700, "b")], 1000, "kf", "", {})
        self.assertEqual(text, "{\\kf20}a{\\k30} {\\kf20}b")

    def test_cjk_words_are_not_space_separated(self):
        text = ass_module.karaoke_text([(0, 100, "你"), (100, 200, "好")], 0, "kf", "", {})
        self.assertEqual(text, "{\\kf10}你{\\kf10}好")

    def test_syllable_template_uses_relative_times(self):
        template, _ = ass_module.EFFECT_PRESETS["pop"]["syllable"], None
        text = ass_module.karaoke_text([(2000, 2400, "x")], 2000, "kf", template, {"scale": 130})
        self.assertIn("\\t(0,200,\\fscx130\\fscy130)\\t(200,400,\\fscx100\\fscy100)", text)


class AssLayoutTest(unittest.TestCase):
    entries = [
        (1000, 2000, "one", [(1000, 2000, "one")]),
        (2500, 3500, "two", [(2500, 3500, "two")]),
        (4000, 5000, "three", [(4000, 5000, "three")]),
    ]

    def test_single_mode_keeps_existing_margin(self):
        lines = lyric_lines(render(self.entries))
        self.assertTrue(all(",Lyric,,0,0,150,," in line for line in lines))
        self.assertTrue(lines[0].startswith("Dialogue: 0,0:00:01.00,"))

    def test_alternate_mode_rows_and_lead_in(self):
        lines = lyric_lines(render(self.entries, layout={"mode": "alternate", "lead_in_ms": 1500}))
        self.assertIn("{\\an1}", lines[0])
        self.assertIn("{\\an3}", lines[1])
        # line two is shown early (lead-in) and karaoke waits with a silent \k
        self.assertTrue(lines[1].startswith("Dialogue: 0,0:00:01.00,0:00:03.50"))
        self.assertIn("{\\k150}", lines[1])
        # line three shares row 0 with line one and must wait until line one ends
        self.assertTrue(lines[2].startswith("Dialogue: 0,0:00:02.50,"))

    def test_fade_preset_and_unknown_preset(self):
        lines = lyric_lines(render(self.entries, effects={"preset": "fade", "params": {"fade_in": 100}}))
        self.assertIn("{\\fad(100,200)}", lines[0])
        with self.assertRaises(ValueError):
            render(self.entries, effects={"preset": "nope"})

    def test_bad_layout_mode(self):
        with self.assertRaises(ValueError):
            render(self.entries, layout={"mode": "diagonal"})


if __name__ == "__main__":
    unittest.main()

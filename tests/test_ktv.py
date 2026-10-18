import copy
import re
import unittest

from polykara.ass import ass
from polykara.config import load_config
from polykara.ktv import decorate, placements, voice_of, wrap_credit
from polykara.layout import measured_font

ROW = {"id": "demo", "title": "Demo", "artist": "Singer", "lyricist": "L", "composer": "C", "producer": "P", "language": "en"}


def entry(start, text, step=500):
    words, cursor = [], start
    parts = text.split(" ")
    for index, part in enumerate(parts):
        words.append((cursor, cursor + step, part + (" " if index < len(parts) - 1 else "")))
        cursor += step
    return (start, cursor, text, words)


def events(text, style):
    return [line for line in text.splitlines() if line.startswith("Dialogue:") and f",{style}," in line]


def config_with(**layout):
    config = copy.deepcopy(load_config())
    config["layout"].update(layout)
    return config


class PlacementTest(unittest.TestCase):
    LINES = [entry(20000, "one two"), entry(21500, "three four"), entry(23000, "five six"), entry(24500, "seven eight")]

    def test_pairs_share_their_time_on_screen(self):
        placed = placements(self.LINES, config_with(mode="pairs"))
        self.assertEqual([item["slot"] for item in placed], [0, 1, 0, 1])
        self.assertEqual((placed[0]["show"], placed[0]["hide"]), (placed[1]["show"], placed[1]["hide"]))
        # The second pair appears once the first has gone, and before it is sung.
        self.assertGreaterEqual(placed[2]["show"], placed[1]["hide"])
        self.assertLessEqual(placed[2]["show"], placed[2]["start"])
        for item in placed:
            self.assertLessEqual(item["show"], item["start"])
            self.assertGreaterEqual(item["hide"], item["end"])

    def test_alternate_rows_free_up_one_at_a_time(self):
        placed = placements(self.LINES, config_with(mode="alternate"))
        self.assertEqual([item["slot"] for item in placed], [0, 1, 0, 1])
        self.assertEqual(placed[0]["show"], placed[1]["show"])
        self.assertGreaterEqual(placed[2]["show"], placed[0]["hide"])
        self.assertLess(placed[2]["show"], placed[1]["hide"])

    def test_long_gap_starts_again_in_the_upper_row_with_a_countdown(self):
        lines = [entry(20000, "one"), entry(21000, "two"), entry(22000, "three"), entry(60000, "four")]
        placed = placements(lines, config_with(mode="pairs"))
        self.assertEqual([item["slot"] for item in placed], [0, 1, 0, 0])
        self.assertEqual([item["countdown"] for item in placed], [True, False, False, True])
        self.assertLessEqual(placed[3]["show"], 60000 - 4000)


class DocumentTest(unittest.TestCase):
    def test_rows_are_left_and_right_aligned(self):
        text = ass(ROW, [entry(20000, "first line"), entry(22000, "second line")])
        top, bottom = events(text, "Lyric")
        self.assertIn("\\an1\\pos(105,", top)
        x, y = map(float, re.search(r"\\pos\(([\d.]+),([\d.]+)\)", bottom).groups())
        self.assertGreater(x, 400)
        self.assertGreater(y, float(re.search(r"\\pos\([\d.]+,([\d.]+)\)", top).group(1)))
        self.assertIn("; PolyKaraLayout: ktv", text)

    def test_credits_card_uses_the_song_language(self):
        text = ass({**ROW, "language": "zh", "artist": "歌星"}, [entry(20000, "hello")])
        credits = events(text, "CardCredit")
        self.assertTrue(credits[0].endswith("歌手 : 歌星"))
        self.assertTrue(any(line.endswith("字幕制作 : P") for line in credits))
        self.assertTrue(events(text, "CardTitle")[0].endswith("}Demo"))

    def test_countdown_dots_count_down_before_the_first_line(self):
        dots = [line.rsplit("}", 1)[1] for line in events(ass(ROW, [entry(20000, "hello")]), "Prompt")]
        self.assertEqual(dots, ["●●●●", "●●●", "●●", "●"])

    def test_first_line_too_early_for_the_countdown_delays_the_video(self):
        text = ass(ROW, [entry(1000, "hello")])
        self.assertIn("; PolyKaraIntroPaddingMs: 3500", text)

    def test_voice_column_and_speaker_tags_choose_colours(self):
        config = load_config()
        tagged = entry(20000, "hello")
        text = ass({**ROW, "voice": "male"}, [(tagged[0], tagged[1], "[singer: 女] hello", tagged[3]), entry(22000, "again")])
        lyric_style = next(line for line in text.splitlines() if line.startswith("Style: Lyric,"))
        self.assertIn(config["singers"]["male_color"], lyric_style)
        female, male = events(text, "Lyric")
        self.assertIn(f"\\1c{config['singers']['female_color']}", female)
        self.assertNotIn("\\1c", male)
        label = events(text, "Label")[0]
        self.assertTrue(label.endswith("女："))

    def test_voice_names(self):
        self.assertEqual([voice_of(name) for name in ("女", "Male", "合", "duet", "Jennie")], ["female", "male", "both", "both", None])

    def test_long_credit_continues_under_the_first_name(self):
        style = {"font": "Arial", "size": 50, "bold": True}
        rows = wrap_credit("Composer : ", "Alpha One、Bravo Two、Charlie Three、Delta Four、Echo Five", style, 700)
        self.assertGreater(len(rows), 1)
        self.assertTrue(rows[0][0].startswith("Composer : Alpha One、"))
        self.assertTrue(all(indented for _, indented in rows[1:]))
        self.assertEqual("".join(text for text, _ in rows).replace("Composer : ", ""), "Alpha One、Bravo Two、Charlie Three、Delta Four、Echo Five")


@unittest.skipUnless(measured_font("Arial", False), "needs Pillow and fontconfig to measure fonts")
class DecorateTest(unittest.TestCase):
    def test_each_karaoke_event_gets_a_clipped_sung_copy(self):
        text = ass(ROW, [entry(20000, "one two three")])
        decorated, done = decorate(text, load_config())
        self.assertTrue(done)
        base = events(text, "Lyric")
        overlay = [line for line in events(decorated, "Lyric") if line.startswith("Dialogue: 5,")]
        self.assertEqual(len(overlay), len(base))
        self.assertNotIn("\\k", overlay[0])
        self.assertIn("\\3c&H00FFFFFF", overlay[0])
        # One clip step per word, ending at the time the karaoke ends.
        steps = re.findall(r"\\t\((\d+),(\d+),\\clip\(([-\d]+),0,([-\d]+),1080\)\)", overlay[0])
        self.assertEqual(len(steps), 3)
        edges = [int(right) for _, _, _, right in steps]
        self.assertEqual(edges, sorted(edges))
        lead = int(re.search(r"\{\\k(\d+)\}", base[0]).group(1)) * 10
        self.assertEqual(int(steps[0][0]), lead)
        self.assertEqual(int(steps[-1][1]), lead + 1500)

    def test_single_layout_is_left_alone(self):
        text = "[Script Info]\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Lyric,,0,0,0,,{\\kf50}hi\n"
        self.assertEqual(decorate(text, load_config()), (text, False))


if __name__ == "__main__":
    unittest.main()

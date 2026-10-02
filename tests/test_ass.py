import unittest

from polykara.ass import ass, at, esc, karaoke_text

ROW = {"id": "demo", "title": "Demo", "artist": "Artist", "lyricist": "L", "composer": "C", "producer": "P", "language": "en"}


def lyric_lines(text):
    return [line for line in text.splitlines() if line.startswith("Dialogue:") and ",Lyric," in line]


class KaraokeTextTest(unittest.TestCase):
    def test_wait_before_first_word_and_gaps_are_kept(self):
        words = [(10500, 11000, "Amazing "), (11400, 12000, "grace")]
        self.assertEqual(karaoke_text(words, 10000, "kf"), "{\\k50}{\\kf50}Amazing {\\k40}{\\kf60}grace")

    def test_durations_add_up_to_the_last_word_end(self):
        words = [(1003, 1337, "a "), (1337, 1998, "b "), (2511, 3004, "c")]
        text = karaoke_text(words, 1000, "k")
        total = sum(int(value) for value in __import__("re").findall(r"\\k(\d+)", text))
        self.assertEqual(total, (3004 - 1000) // 10)

    def test_cjk_words_are_not_separated_by_spaces(self):
        self.assertEqual(karaoke_text([(0, 500, "你"), (500, 1000, "好")], 0, "kf"), "{\\kf50}你{\\kf50}好")

    def test_escape(self):
        self.assertEqual(esc("a{b}\\c"), "a\\{b\\}/c")

    def test_timestamp(self):
        self.assertEqual(at(3723450), "1:02:03.45")


class AssDocumentTest(unittest.TestCase):
    def test_document_has_one_karaoke_line_per_entry(self):
        entries = [(20000, 24000, "Hello world", [(20500, 21000, "Hello "), (21000, 22000, "world")]), (24000, 28000, "[speaker:A] Again", [(24000, 25000, "Again")])]
        text = ass(ROW, entries)
        lines = lyric_lines(text)
        self.assertEqual(len(lines), 2)
        self.assertTrue(all("{\\k" in line for line in lines))
        self.assertIn("Hello ", lines[0])
        self.assertIn("{\\c&H", lines[1])
        self.assertIn("; PolyKaraIntroPaddingMs: 0", text)

    def test_line_is_shown_early_but_highlight_starts_on_time(self):
        entries = [(20000, 24000, "Hello", [(20000, 21000, "Hello")])]
        line = lyric_lines(ass(ROW, entries))[0]
        start = line.split(",")[1]
        # Whatever lead-in is configured, display start plus the leading wait equals the sung start.
        lead = 20000 - (int(start[2:4]) * 60000 + int(start[5:7]) * 1000 + int(start[8:10]) * 10)
        self.assertGreaterEqual(lead, 0)
        self.assertTrue(line.endswith("{\\kf100}Hello"))
        if lead:
            self.assertIn(f"{{\\k{lead // 10}}}{{\\kf100}}Hello", line)

    def test_early_lyrics_are_pushed_after_the_title_card(self):
        text = ass(ROW, [(1000, 3000, "Hi", [(1000, 2000, "Hi")])])
        padding = int(text.split("; PolyKaraIntroPaddingMs:")[1].splitlines()[0])
        line = lyric_lines(text)[0]
        self.assertGreaterEqual(padding, 0)
        self.assertIn("{\\kf100}Hi", line)


if __name__ == "__main__":
    unittest.main()

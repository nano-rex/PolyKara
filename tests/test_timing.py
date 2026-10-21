import tempfile
import unittest
from pathlib import Path
from unittest import mock

from polykara import timing, verify
from polykara.subtitle import parse_lrc
from polykara.timing import Line, Token


def timed_line(start, pieces, speaker=""):
    return Line([Token(piece, start + 300 * index, start + 300 * index + 250) for index, piece in enumerate(pieces)], speaker)


class SaveTest(unittest.TestCase):
    def test_saved_timing_reads_back_word_for_word(self):
        lines = [timed_line(1000, ["Hello ", "PolyKara ", "world"]), timed_line(5000, list("唤不醒"), "女")]
        entries, estimated = timing.finished(lines)
        back = parse_lrc(timing.to_lrc(entries, "Title", "Singer"))
        self.assertEqual(estimated, 0)
        self.assertEqual([words for _, _, _, words in back], [words for _, _, _, words in entries])
        self.assertEqual(back[0][2], "Hello PolyKara world")
        self.assertEqual(back[1][2], "[speaker:女] 唤不醒")
        # The empty mark after a line gives its real end, not the next line's start.
        self.assertEqual(back[0][1], 1850)
        self.assertEqual(timing.lines_from_entries(back)[1].speaker, "女")

    def test_untimed_words_and_lines_are_filled_in(self):
        partial = timed_line(1000, list("一二三四"))
        partial.tokens[1].start = partial.tokens[1].end = None
        lines = [partial, Line([Token("没"), Token("有")]), timed_line(9000, list("五六"))]
        entries, estimated = timing.finished(lines)
        self.assertEqual(estimated, 1)
        first = entries[0][3]
        self.assertEqual((first[1][0], first[1][1]), (first[0][1], first[2][0]))
        self.assertTrue(entries[0][1] <= entries[1][0] < entries[2][0])
        starts = [left for _, _, _, words in entries for left, _, _ in words]
        self.assertEqual(starts, sorted(starts))

    def test_editor_file_is_the_lyric_source(self):
        with tempfile.TemporaryDirectory() as folder:
            lyrics = Path(folder)
            row = {"id": "song", "title": "T", "artist": "A", "lyrics_file": ""}
            with mock.patch.object(timing, "EXTERNAL_LYRICS", lyrics), mock.patch.object(verify, "EXTERNAL_LYRICS", lyrics), mock.patch.object(verify, "provider_files", return_value=[]), mock.patch.object(verify, "downloaded_subtitle", return_value=None):
                path, _ = timing.save(row, [timed_line(1000, list("一二三四五六")), timed_line(4000, list("七八九十百千"))])
                verify._check.cache_clear()
                report = verify.check_lyrics(row)
                lines, label = timing.load(row)
        self.assertEqual(path.name, "song.timed.lrc")
        self.assertEqual(report.primary.name, "editor")
        self.assertTrue(all(words for _, _, _, words in report.primary.entries))
        self.assertIn("saved timing", label)
        self.assertEqual(lines[1].tokens[0].start, 4000)

    def test_words_keep_their_spaces_when_read_back(self):
        lines = timing.lines_from_entries(parse_lrc("[00:05.000]<00:05.000>Hello <00:05.500>PolyKara <00:06.200>world<00:06.800>\n[00:08.000]<00:08.000>[singer: 女] 唤<00:08.300>醒<00:08.600>\n"))
        self.assertEqual([token.text for token in lines[0].tokens], ["Hello ", "PolyKara ", "world"])
        self.assertEqual((lines[1].speaker, lines[1].tokens[0].text), ("女", "唤"))


class RetextTest(unittest.TestCase):
    def test_unchanged_lines_keep_timing_and_new_lines_are_untimed(self):
        lines = [timed_line(1000, ["one ", "two"]), timed_line(3000, ["three ", "four"])]
        fresh = timing.retext(lines, "one two\nnew line here\n[singer: 男] three fore")
        self.assertEqual(fresh[0].tokens[1].start, 1300)
        self.assertFalse(any(token.timed for token in fresh[1].tokens))
        # Same number of words: a corrected typo keeps its times.
        self.assertEqual((fresh[2].speaker, fresh[2].tokens[1].text, fresh[2].tokens[1].start), ("男", "fore", 3300))
        self.assertEqual(timing.lines_to_text(fresh).splitlines()[2], "[singer: 男] three fore")


class PlayerClockTest(unittest.TestCase):
    def test_position_follows_the_reported_clock(self):
        player = timing.Player()
        player.requested = 10.0
        self.assertEqual(player.position(), 10000)
        with mock.patch.object(timing.time, "monotonic", return_value=100.1):
            player.speed, player.first, player.clock = 1.0, 9.5, (12.0, 100.0)
            self.assertEqual(player.position(), 12100)
            # Half speed: ffplay's clock runs in played time from where it started.
            player.speed = 0.5
            self.assertEqual(player.position(), 9500 + (2500 + 100) // 2)

    def test_status_lines_are_recognised(self):
        self.assertEqual(timing.STATUS.match("  25.56 M-A:  0.000 fd=   0 aq=   17KB").group(1), "25.56")
        self.assertIsNone(timing.STATUS.match("    nan M-A:    nan fd=   0"))


if __name__ == "__main__":
    unittest.main()

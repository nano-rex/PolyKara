import unittest
from pathlib import Path

from polykara.providers import CREDIT_LINE
from polykara.verify import LyricFile, agreement, check_files, comparison_tokens

VERSE = "Amazing grace how sweet the sound that saved a wretch like me I once was lost but now am found was blind but now I see"
OTHER = "Silent night holy night all is calm all is bright round yon virgin mother and child holy infant so tender and mild"


def lyric_file(name, text, kind="lrc"):
    return LyricFile(name, name, kind, Path(f"{name}.txt"), [], comparison_tokens(text))


class AgreementTest(unittest.TestCase):
    def test_case_and_punctuation_are_ignored(self):
        self.assertEqual(agreement(comparison_tokens("Amazing grace, how sweet!"), comparison_tokens("amazing GRACE how sweet")), 1.0)

    def test_a_file_without_the_repeated_chorus_still_agrees(self):
        self.assertEqual(agreement(comparison_tokens(VERSE + " " + VERSE), comparison_tokens(VERSE)), 1.0)

    def test_different_songs_disagree(self):
        self.assertLess(agreement(comparison_tokens(VERSE), comparison_tokens(OTHER)), 0.3)


class CheckFilesTest(unittest.TestCase):
    def test_three_agreeing_files_are_verified(self):
        report = check_files([lyric_file("subtitle", VERSE), lyric_file("lrclib", VERSE), lyric_file("lyrics-ovh", VERSE, "plain")], 3, 0.6)
        self.assertTrue(report.verified)
        self.assertEqual(report.primary.name, "subtitle")

    def test_two_files_are_not_enough(self):
        report = check_files([lyric_file("lrclib", VERSE), lyric_file("netease", VERSE)], 3, 0.6)
        self.assertFalse(report.verified)
        self.assertIn("only 2 lyric file(s)", report.problem())

    def test_the_file_confirmed_by_the_others_is_chosen(self):
        files = [lyric_file("subtitle", OTHER), lyric_file("lrclib", VERSE), lyric_file("netease", VERSE), lyric_file("lyrics-ovh", VERSE, "plain")]
        report = check_files(files, 3, 0.6)
        self.assertEqual(report.primary.name, "lrclib")
        self.assertTrue(report.verified)
        self.assertEqual([item.agrees for item in report.files], [False, True, True, True])

    def test_manifest_file_stays_primary_but_is_flagged_when_others_differ(self):
        files = [lyric_file("manifest", OTHER), lyric_file("lrclib", VERSE), lyric_file("netease", VERSE)]
        report = check_files(files, 3, 0.6, fixed_primary=True)
        self.assertEqual(report.primary.name, "manifest")
        self.assertFalse(report.verified)
        self.assertIn("only 1 of 3 lyric files agree", report.problem())

    def test_plain_text_is_only_used_when_nothing_is_timed(self):
        report = check_files([lyric_file("lyrics-ovh", VERSE, "plain"), lyric_file("webpage", VERSE, "plain")], 3, 0.6)
        self.assertEqual(report.primary.name, "lyrics-ovh")

    def test_no_files(self):
        report = check_files([], 3, 0.6)
        self.assertIsNone(report.primary)
        self.assertFalse(report.verified)


class NeteaseCreditTest(unittest.TestCase):
    def test_credit_lines_are_recognised_but_lyrics_are_not(self):
        self.assertTrue(CREDIT_LINE.match("[00:00.000] 作词 : 周杰伦"))
        self.assertTrue(CREDIT_LINE.match("[00:01.00]Composer : Someone"))
        self.assertFalse(CREDIT_LINE.match("[00:28.950]故事的小黄花"))
        self.assertFalse(CREDIT_LINE.match("[00:30.00]Listen: the wind is calling"))


if __name__ == "__main__":
    unittest.main()

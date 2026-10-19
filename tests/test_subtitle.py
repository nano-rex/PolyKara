import unittest

from polykara.subtitle import fill_gaps, match_tokens, parse_lrc, parse_timed_subtitle, plain_entries, time_entries, timed_units, tokenise


def units(*words):
    return timed_units({"segments": [{"words": [{"start": start, "end": end, "word": word} for start, end, word in words]}]})


class TokeniseTest(unittest.TestCase):
    def test_latin_words_keep_spacing_and_punctuation(self):
        tokens = tokenise("Amazing grace, how sweet - the sound!")
        self.assertEqual(tokens, ["Amazing ", "grace, ", "how ", "sweet - ", "the ", "sound!"])
        self.assertEqual("".join(tokens), "Amazing grace, how sweet - the sound!")

    def test_cjk_is_split_per_character_without_spaces(self):
        self.assertEqual(tokenise("你好，世界"), ["你", "好，", "世", "界"])

    def test_mixed_script_keeps_latin_words_whole(self):
        self.assertEqual(tokenise("Hello 世界 baby"), ["Hello ", "世", "界 ", "baby"])

    def test_leading_punctuation_joins_the_first_word(self):
        self.assertEqual(tokenise("¿Qué pasa?"), ["¿Qué ", "pasa?"])


class WordTimingTest(unittest.TestCase):
    def test_words_are_matched_by_text_not_position(self):
        # The recogniser heard an extra word first; positional mapping would shift every word.
        timing = units((10.0, 10.2, "oh"), (10.5, 11.0, "Amazing"), (11.0, 11.6, "grace"))
        (start, end, _, words), = time_entries([(10000, 14000, "Amazing grace", [])], timing)
        self.assertEqual(words, [(10500, 11000, "Amazing "), (11000, 11600, "grace")])

    def test_unrecognised_word_is_placed_between_its_neighbours(self):
        timing = units((1.0, 1.5, "I"), (3.0, 3.5, "lost"))
        (_, _, _, words), = time_entries([(1000, 5000, "I was lost", [])], timing)
        self.assertEqual([word for _, _, word in words], ["I ", "was ", "lost"])
        self.assertEqual(words[1][:2], (1500, 2700))
        self.assertEqual(words[2][:2], (3000, 3500))

    def test_times_never_run_backwards_or_leave_the_line(self):
        timing = units((0.5, 0.9, "one"), (2.0, 2.4, "two"), (2.2, 9.0, "three"))
        (start, end, _, words), = time_entries([(1000, 3000, "one two three four", [])], timing)
        cursor = start
        for left, right, _ in words:
            self.assertGreaterEqual(left, cursor)
            self.assertGreaterEqual(right, left)
            self.assertLessEqual(right, end)
            cursor = right

    def test_multi_character_cjk_word_is_shared_between_characters(self):
        timing = units((1.0, 2.0, "你好"), (2.0, 3.0, "世界"))
        (_, _, _, words), = time_entries([(1000, 3000, "你好世界", [])], timing)
        self.assertEqual(words, [(1000, 1500, "你"), (1500, 2000, "好"), (2000, 2500, "世"), (2500, 3000, "界")])

    def test_homophone_substitution_keeps_position(self):
        timing = units((1.0, 1.4, "他"), (1.4, 1.8, "们"))
        (_, _, _, words), = time_entries([(1000, 3000, "她們", [])], timing)
        self.assertEqual(words, [(1000, 1400, "她"), (1400, 1800, "們")])

    def test_line_without_recognised_words_is_reported(self):
        estimated = []
        (_, _, _, words), = time_entries([(60000, 64000, "la la la la", [])], units((1.0, 1.5, "hello")), estimated=estimated)
        self.assertEqual(estimated, [60000])
        self.assertEqual([left for left, _, _ in words], [60000, 61000, 62000, 63000])

    def test_existing_word_timing_is_kept(self):
        entry = (1000, 3000, "Hi there", [(1000, 1500, "Hi "), (1500, 3000, "there")])
        self.assertEqual(time_entries([entry], units((1.2, 1.3, "Hi"))), [entry])

    def test_long_tail_is_trimmed_after_the_last_word(self):
        timing = units((1.0, 1.5, "hello"), (1.5, 2.0, "world"))
        (start, end, _, _), = time_entries([(1000, 40000, "hello world", [])], timing, max_tail_ms=5000, tail_hold_ms=1000)
        self.assertEqual((start, end), (1000, 3000))

    def test_a_recognised_word_is_used_by_one_line_only(self):
        timing = units((1.0, 1.5, "la"), (4.9, 5.4, "la"))
        first, second = time_entries([(1000, 5000, "la", []), (5000, 9000, "la", [])], timing)
        self.assertEqual(first[3], [(1000, 1500, "la")])
        self.assertEqual(second[3][0][0], 5000)

    def test_fill_gaps_whole_line(self):
        self.assertEqual(fill_gaps([None, None], 0, 1000), [(0, 500), (500, 1000)])

    def test_match_tokens_without_units(self):
        self.assertEqual(match_tokens(["a"], []), [None])

    def test_plain_lyrics_are_timed_from_recognised_words(self):
        timing = units((1.0, 1.4, "hello"), (1.4, 2.0, "world"), (5.0, 5.5, "good"), (5.5, 6.0, "bye"))
        entries = plain_entries("Hello world\n\nGood bye\n", timing)
        self.assertEqual([(start, end, text) for start, end, text, _ in entries], [(1000, 2000, "Hello world"), (5000, 6000, "Good bye")])
        self.assertEqual(entries[1][3], [(5000, 5500, "Good "), (5500, 6000, "bye")])


class LrcTest(unittest.TestCase):
    def test_line_ends_at_the_next_timestamp(self):
        entries = parse_lrc("[ar:Someone]\n[00:01.00]First\n[00:05.50]Second\n")
        self.assertEqual(entries[0][:3], (1000, 5500, "First"))
        self.assertEqual(entries[1][0], 5500)

    def test_empty_timestamp_ends_the_previous_line(self):
        entries = parse_lrc("[00:01.00]First\n[00:03.00]\n[00:40.00]Second\n")
        self.assertEqual([entry[:3] for entry in entries], [(1000, 3000, "First"), (40000, 42000, "Second")])

    def test_millisecond_precision_and_repeated_timestamps(self):
        entries = parse_lrc("[00:01.123][00:10.5]Chorus\n")
        self.assertEqual([entry[0] for entry in entries], [1123, 10500])

    def test_offset_tag_shifts_lyrics_earlier(self):
        self.assertEqual(parse_lrc("[offset:500]\n[00:02.00]Line\n")[0][0], 1500)

    def test_enhanced_lrc_marks_are_removed_from_text(self):
        (start, end, text, words), = parse_lrc("[00:12.00]<00:12.00>Hello <00:12.80>world<00:13.50>\n")
        self.assertEqual(text, "Hello world")
        self.assertEqual(words, [(12000, 12800, "Hello "), (12800, 13500, "world")])

    def test_enhanced_lrc_without_spaces(self):
        (_, _, text, words), = parse_lrc("[00:12.00]<00:12.00>ありがとう<00:12.80>ございます\n[00:15.00]\n")
        self.assertEqual(text, "ありがとうございます")
        self.assertEqual(words, [(12000, 12800, "ありがとう"), (12800, 15000, "ございます")])

    def test_singer_tag_is_preserved(self):
        self.assertEqual(parse_lrc("[00:12.00][singer:A]First line\n")[0][2], "[speaker:A] First line")


class TimedSubtitleTest(unittest.TestCase):
    def test_srt(self):
        entries = parse_timed_subtitle("1\n00:00:01,000 --> 00:00:03,500\nHello <i>there</i>\n\n2\n00:00:04,000 --> 00:00:05,000\nAgain\n")
        self.assertEqual([entry[:3] for entry in entries], [(1000, 3500, "Hello there"), (4000, 5000, "Again")])

    def test_youtube_rolling_captions_are_deduplicated(self):
        content = "\n".join([
            "WEBVTT", "",
            "00:00:01.000 --> 00:00:03.000 align:start position:0%", " ", "hello<00:00:01.500><c> world</c>", "",
            "00:00:03.000 --> 00:00:03.010 align:start position:0%", "hello world", " ", "",
            "00:00:03.010 --> 00:00:05.000 align:start position:0%", "hello world", "second<00:00:04.000><c> line</c>", "",
            "00:00:05.000 --> 00:00:08.000", "[Music]", "",
        ])
        self.assertEqual([entry[2] for entry in parse_timed_subtitle(content)], ["hello world", "second line"])

    def test_repeated_lyric_lines_are_kept(self):
        content = "00:01.000 --> 00:02.000\nla la\n\n00:02.000 --> 00:03.000\nla la\n"
        self.assertEqual(len(parse_timed_subtitle(content)), 2)


if __name__ == "__main__":
    unittest.main()

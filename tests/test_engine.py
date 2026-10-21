import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from polykara import engine, readings, timing, verify


def ask(*requests):
    out = io.StringIO()
    engine.serve(io.StringIO("".join(json.dumps(item) + "\n" for item in requests)), out)
    return [json.loads(line) for line in out.getvalue().splitlines()]


class EngineTest(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.home = Path(folder.name)
        for target, name in ((engine, "MANIFEST"), (timing, "EXTERNAL_LYRICS"), (verify, "EXTERNAL_LYRICS")):
            value = self.home / "songs.csv" if name == "MANIFEST" else self.home / "lyrics"
            patcher = mock.patch.object(target, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        engine.write_songs(engine.COLUMNS, [{"id": "song", "title": "T", "artist": "A", "language": "zh", "url": "https://example.com/v"}])

    def test_answers_match_requests_and_errors_do_not_stop_the_engine(self):
        answers = ask({"id": 1, "cmd": "songs"}, {"id": 2, "cmd": "nope"}, {"id": 3, "cmd": "new_id", "args": {"title": "Hello World", "taken": ["hello-world"]}})
        self.assertEqual([answer["id"] for answer in answers], [1, 2, 3])
        self.assertEqual(answers[0]["result"]["songs"][0]["id"], "song")
        self.assertFalse(answers[1]["ok"])
        self.assertEqual(answers[2]["result"]["id"], "hello-world-2")

    def test_printing_while_answering_does_not_break_the_answers(self):
        with mock.patch.dict(engine.COMMANDS, {"noisy": lambda args: print("chatter") or {"fine": True}}):
            with mock.patch("sys.stderr", io.StringIO()) as stderr:
                answers = ask({"id": 1, "cmd": "noisy"})
        self.assertEqual(answers, [{"id": 1, "ok": True, "result": {"fine": True}}])
        self.assertIn("chatter", stderr.getvalue())

    def test_timing_saved_by_the_app_loads_back(self):
        lines = [{"speaker": "女", "start": None, "end": None, "tokens": [{"text": ch, "start": 1000 + 300 * n, "end": 1250 + 300 * n} for n, ch in enumerate("唤不醒")]}]
        with mock.patch.object(verify, "provider_files", return_value=[]), mock.patch.object(verify, "downloaded_subtitle", return_value=None):
            saved, loaded = ask({"id": 1, "cmd": "save", "args": {"id": "song", "lines": lines}}, {"id": 2, "cmd": "load", "args": {"id": "song"}})
        self.assertTrue(saved["ok"], saved)
        self.assertEqual(saved["result"]["name"], "song.timed.lrc")
        result = loaded["result"]
        self.assertEqual(result["lines"][0]["speaker"], "女")
        self.assertEqual([token["start"] for token in result["lines"][0]["tokens"]], [1000, 1300, 1600])
        self.assertEqual(result["language"], "zh")

    def test_retext_keeps_timing(self):
        lines = [{"speaker": "", "tokens": [{"text": "one ", "start": 1000, "end": 1200}, {"text": "two", "start": 1300, "end": 1500}]}]
        answer = ask({"id": 1, "cmd": "retext", "args": {"lines": lines, "text": "one two\nthree"}})[0]["result"]
        self.assertEqual(answer["lines"][0]["tokens"][1]["start"], 1300)
        self.assertIsNone(answer["lines"][1]["tokens"][0]["start"])

    def test_readings_follow_the_word_library(self):
        try:
            import pypinyin  # noqa: F401
        except ImportError:
            self.skipTest("pypinyin is not installed")
        with mock.patch.object(readings, "user_file", lambda language: self.home / f"{language}.txt"):
            first = ask({"id": 1, "cmd": "readings", "args": {"language": "zh", "lines": [["重", "来"], ["Hi"]]}})[0]["result"]["readings"]
        # 重来 is chóng lái in the built-in library, not zhòng lái; Latin text needs no reading.
        self.assertEqual([item.strip("1234") for item in first[0]], ["chong", "lai"])
        self.assertEqual(first[1], [None])


class SettingsFormTest(unittest.TestCase):
    def test_form_describes_fields_and_changes_are_written_in_place(self):
        text = '[karaoke]\n# The colour of sung words.\nactive_color = "&H00FF0000"\n\n[layout]\nmode = "single"\n'
        tabs = engine.settings_form({"text": text})["tabs"]
        fields = {(section["id"], field["key"]): field for tab in tabs for section in tab["sections"] for field in section["fields"]}
        self.assertEqual(fields[("karaoke", "active_color")]["kind"], "colour")
        self.assertIn("sung words", fields[("karaoke", "active_color")]["help"])
        self.assertEqual(fields[("layout", "mode")]["choices"], ["single", "alternate"])
        self.assertEqual(fields[("lyric", "bold")]["kind"], "bool")
        changed = engine.settings_apply({"text": text, "changes": [["layout", "mode", "alternate"], ["lyric", "size", "60"], ["karaoke", "active_color", "&H00FF0000"]]})["text"]
        self.assertIn('mode = "alternate"', changed)
        self.assertIn("[lyric]\nsize = 60", changed)
        self.assertIn("# The colour of sung words.", changed)

    def test_broken_file_is_reported(self):
        answer = engine.settings_form({"text": "[layout\n"})
        self.assertEqual(answer["tabs"], [])
        self.assertIn("mistake", answer["error"])

    def test_number_fields_must_be_numbers(self):
        with self.assertRaises(ValueError):
            engine.settings_apply({"text": "", "changes": [["lyric", "size", "big"]]})


if __name__ == "__main__":
    unittest.main()

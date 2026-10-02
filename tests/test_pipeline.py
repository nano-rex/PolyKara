import contextlib
import io
import subprocess
import unittest

import pipeline
from polykara.verify import LyricCheck, LyricFile


def lyric_check(count):
    files = [LyricFile(f"source{index}", "label", "lrc", pipeline.EXTERNAL_LYRICS / f"demo.source{index}.lrc") for index in range(count)]
    return LyricCheck(files, files[0] if files else None, 3)


class BatchIsolationTest(unittest.TestCase):
    def setUp(self):
        pipeline.FAILED.clear()

    def tearDown(self):
        pipeline.FAILED.clear()

    def test_one_failing_song_does_not_stop_the_others(self):
        done = []

        def action(row):
            if row["id"] == "b":
                raise subprocess.CalledProcessError(1, ["ffmpeg", "-i", "x"])
            if row["id"] == "c":
                raise SystemExit("No downloaded media found for c")
            done.append(row["id"])

        songs = [{"id": name} for name in "abcd"]
        with contextlib.redirect_stdout(io.StringIO()):
            pipeline.each("render", songs, action)
            pipeline.each("later", songs, lambda row: done.append(row["id"] + "2"))
            status = pipeline.summary()
        self.assertEqual(done, ["a", "d", "a2", "d2"])
        self.assertEqual(pipeline.FAILED, {"b": "render: ffmpeg exited with status 1", "c": "render: No downloaded media found for c"})
        self.assertEqual(status, 1)

    def test_summary_is_zero_without_failures(self):
        self.assertEqual(pipeline.summary(), 0)


class Marker:
    def __init__(self, present):
        self.present = present

    def exists(self):
        return self.present


class DownloadSkipTest(unittest.TestCase):
    def setUp(self):
        self.commands = []
        self.saved = {name: getattr(pipeline, name) for name in ("run", "find_source", "media_problem", "require_current_ytdlp", "check_lyrics", "lyrics_marker")}
        pipeline.run = lambda cmd, dry_run=False, cwd=None: self.commands.append("subtitles" if "--skip-download" in cmd else "media")
        pipeline.require_current_ytdlp = lambda: None
        pipeline.find_source = lambda sid: pipeline.RAW / "demo.mp4"
        pipeline.media_problem = lambda path: None
        pipeline.check_lyrics = lambda row: lyric_check(3)
        pipeline.lyrics_marker = lambda sid: Marker(False)
        self.row = {"id": "demo", "url": "https://example.invalid/watch", "language": "en", "subtitle_langs": ""}

    def tearDown(self):
        for name, value in self.saved.items():
            setattr(pipeline, name, value)

    def download(self, **options):
        with contextlib.redirect_stdout(io.StringIO()):
            pipeline.download_song(self.row, True, **options)
        return self.commands

    def test_existing_media_and_lyrics_run_nothing(self):
        self.assertEqual(self.download(), [])

    def test_missing_media_is_downloaded_but_existing_lyrics_are_kept(self):
        pipeline.find_source = lambda sid: None
        self.assertEqual(self.download(), ["media"])

    def test_unplayable_media_is_downloaded_again(self):
        pipeline.media_problem = lambda path: "it has no audio track"
        self.assertEqual(self.download(), ["media"])

    def test_missing_lyrics_are_fetched_without_touching_media(self):
        pipeline.check_lyrics = lambda row: lyric_check(0)
        self.assertEqual(set(self.download()), {"subtitles"})

    def test_fewer_than_three_lyric_files_keeps_looking(self):
        pipeline.check_lyrics = lambda row: lyric_check(2)
        self.assertEqual(set(self.download()), {"subtitles"})

    def test_lyrics_that_were_unavailable_are_not_requested_again(self):
        pipeline.check_lyrics = lambda row: lyric_check(1)
        pipeline.lyrics_marker = lambda sid: Marker(True)
        self.assertEqual(self.download(), [])

    def test_reprocess_retries_lyrics_only(self):
        pipeline.check_lyrics = lambda row: lyric_check(1)
        pipeline.lyrics_marker = lambda sid: Marker(True)
        self.assertEqual(set(self.download(force=True)), {"subtitles"})


class MediaProblemTest(unittest.TestCase):
    def probe(self, stdout, returncode=0):
        saved = pipeline.subprocess.run
        pipeline.subprocess.run = lambda *args, **kwargs: subprocess.CompletedProcess(args, returncode, stdout, "")
        try:
            return pipeline.media_problem(pipeline.RAW / "demo.mp4")
        finally:
            pipeline.subprocess.run = saved

    def test_video_with_audio_is_usable(self):
        self.assertIsNone(self.probe('{"streams": [{"codec_type": "video"}, {"codec_type": "audio"}], "format": {"duration": "200.5"}}'))

    def test_video_only_file_is_rejected(self):
        self.assertEqual(self.probe('{"streams": [{"codec_type": "video"}], "format": {"duration": "200.5"}}'), "it has no audio track")

    def test_unreadable_file_is_rejected(self):
        self.assertEqual(self.probe("", returncode=1), "it cannot be read")


if __name__ == "__main__":
    unittest.main()

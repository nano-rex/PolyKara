import contextlib
import io
import subprocess
import unittest

import pipeline


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


if __name__ == "__main__":
    unittest.main()

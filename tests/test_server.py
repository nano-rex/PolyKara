import copy
import json
import tempfile
import threading
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path
from unittest import mock
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from polykara import ass as ass_module
from polykara import project, server
from polykara.config import DEFAULT_CONFIG

ROW = {"id": "song1", "title": "T", "artist": "A", "lyricist": "", "composer": "", "producer": "", "language": "en", "lyrics_file": ""}


class ServerTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        root = Path(cls.tmp.name)
        cls.media = root / "song1.mp4"
        cls.media.write_bytes(bytes(range(256)) * 4)
        config = copy.deepcopy(DEFAULT_CONFIG)
        config["title"]["enabled"] = config["credit"]["enabled"] = False
        cls.patches = [
            mock.patch.object(server, "rows", return_value=[ROW]),
            mock.patch.object(server, "song_media", return_value=cls.media),
            mock.patch.object(server, "peaks", return_value=[1, 2, 3]),
            mock.patch.object(project, "PROJECTS", root / "project"),
            mock.patch.object(project, "ASS", root / "ass"),
            mock.patch.object(server, "WORK", root),
            mock.patch.object(ass_module, "load_config", return_value=config),
        ]
        for patch in cls.patches:
            patch.start()
        cls.httpd = ThreadingHTTPServer(("127.0.0.1", 0), server.Handler)
        cls.base = f"http://127.0.0.1:{cls.httpd.server_address[1]}"
        threading.Thread(target=cls.httpd.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()
        cls.httpd.server_close()
        for patch in cls.patches:
            patch.stop()
        cls.tmp.cleanup()

    def get(self, path, headers=None):
        return urlopen(Request(self.base + path, headers=headers or {}), timeout=5)

    def test_pages_and_static(self):
        self.assertIn(b"PolyKara Studio", self.get("/").read())
        self.assertIn(b"PolyKara Player", self.get("/player").read())
        self.assertEqual(self.get("/static/karaoke.js").headers["Content-Type"], "text/javascript")
        with self.assertRaises(HTTPError) as error:
            self.get("/static/../server.py")
        self.assertEqual(error.exception.code, 404)

    def test_save_load_export(self):
        lines = {"lines": [{"start": 1000, "end": 2000, "text": "hi you", "words": [[1000, 1400, "hi"], [1500, 1900, "you"]]}]}
        request = Request(self.base + "/api/song/song1", data=json.dumps(lines).encode(), method="PUT", headers={"Content-Type": "application/json"})
        self.assertEqual(json.load(urlopen(request, timeout=5))["lines"], 1)
        song = json.load(self.get("/api/song/song1"))
        self.assertEqual((song["source"], song["lines"][0]["words"][1]), ("project", [1500, 1900, "you"]))
        self.assertTrue(song["video"])
        self.assertIn("<00:01.50>you", self.get("/api/song/song1/export/lrc").read().decode())
        self.assertIn("{\\kf40}hi", self.get("/api/song/song1/export/ass").read().decode())
        self.assertEqual(json.load(self.get("/api/song/song1/peaks"))["peaks"], [1, 2, 3])

    def test_unknown_song_and_bad_body(self):
        with self.assertRaises(HTTPError) as error:
            self.get("/api/song/nope")
        self.assertEqual(error.exception.code, 404)
        request = Request(self.base + "/api/song/song1", data=b"{not json", method="PUT")
        with self.assertRaises(HTTPError) as error:
            urlopen(request, timeout=5)
        self.assertEqual(error.exception.code, 400)

    def test_media_range(self):
        response = self.get("/media/song/song1", {"Range": "bytes=10-19"})
        self.assertEqual(response.status, 206)
        self.assertEqual(response.headers["Content-Range"], "bytes 10-19/1024")
        self.assertEqual(response.read(), bytes(range(10, 20)))
        self.assertEqual(len(self.get("/media/song/song1", {"Range": "bytes=-4"}).read()), 4)


if __name__ == "__main__":
    unittest.main()

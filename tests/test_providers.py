import unittest

from polykara.providers import clean_title, extract_lyrics_page, metadata_matches


class ProvidersTest(unittest.TestCase):
    def test_video_decorations_are_removed_from_titles(self):
        self.assertEqual(clean_title("Shape of You (Official Music Video)"), "Shape of You")
        self.assertEqual(clean_title("告白氣球 [Official MV]"), "告白氣球")
        self.assertEqual(clean_title("Alive (Acoustic)"), "Alive (Acoustic)")

    def test_all_lyric_containers_are_joined(self):
        html = '<html><head><meta charset="utf-8"><link rel="x"></head><body><img src="a.png">' \
               '<div data-lyrics-container="true">Line one<br>Line two<br/><a href="#">Line three</a></div>' \
               '<div class="ad">Buy now</div>' \
               '<div data-lyrics-container="true">Line four</div></body></html>'
        self.assertEqual(extract_lyrics_page(html), "Line one\nLine two\nLine three\nLine four")

    def test_metadata_match_accepts_one_of_several_artists(self):
        candidate = {"trackName": "Die With A Smile", "artistName": "Lady Gaga & Bruno Mars"}
        self.assertTrue(metadata_matches("Die With A Smile (Official Music Video)", "Lady Gaga, Bruno Mars", candidate))
        self.assertFalse(metadata_matches("Another Song", "Lady Gaga", candidate))
        self.assertFalse(metadata_matches("Die With A Smile", "Someone Else", candidate))


if __name__ == "__main__":
    unittest.main()

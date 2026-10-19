import tempfile
import unittest
from pathlib import Path

from polykara.lyricformats import krc, ksc, ksc_units, qrc


def write(text: str, suffix: str, encoding: str = "utf-8") -> Path:
    handle = tempfile.NamedTemporaryFile("wb", suffix=suffix, delete=False)
    handle.write(text.encode(encoding))
    handle.close()
    return Path(handle.name)


class KscTest(unittest.TestCase):
    def test_cjk_units_and_durations(self):
        path = write("karaoke := CreateKaraokeObject;\nkaraoke.add('00:12.000', '00:13.000', '你好世界', '200,300,250,250');\n", ".ksc")
        [(start, end, text, words)] = ksc(path)
        self.assertEqual((start, end, text), (12000, 13000, "你好世界"))
        self.assertEqual(words, [(12000, 12200, "你"), (12200, 12500, "好"), (12500, 12750, "世"), (12750, 13000, "界")])

    def test_bracketed_latin_words(self):
        path = write("karaoke.add('01:02.50', '01:04.00', '[Hello] [world]', '700,800');", ".ksc")
        [(start, _, text, words)] = ksc(path)
        self.assertEqual(start, 62500)
        self.assertEqual(text, "Hello world")
        self.assertEqual([word for _, _, word in words], ["Hello", "world"])

    def test_gbk_encoded_file(self):
        path = write("karaoke.add('00:01.000', '00:02.000', '爱', '1000');", ".ksc", "gb18030")
        self.assertEqual(ksc(path)[0][2], "爱")

    def test_unbracketed_latin_falls_back_to_words(self):
        self.assertEqual(ksc_units("Hi there", 2), ["Hi", "there"])

    def test_mismatched_durations_are_spread(self):
        path = write("karaoke.add('00:00.000', '00:01.000', '一二', '1000');", ".ksc")
        words = ksc(path)[0][3]
        self.assertEqual(len(words), 2)


class KrcQrcTest(unittest.TestCase):
    def test_krc_offsets_are_relative_to_line(self):
        path = write("[ti:Song]\n[1000,900]<0,300,0>我<300,600,0>们\n", ".krc")
        [(start, end, text, words)] = krc(path)
        self.assertEqual((start, end, text), (1000, 1900, "我们"))
        self.assertEqual(words, [(1000, 1300, "我"), (1300, 1900, "们")])

    def test_qrc_xml_content_absolute_times(self):
        path = write('<?xml version="1.0"?><QrcInfos><LyricInfo><Lyric_1 LyricType="1" LyricContent="[5000,1000]Hello (5000,400)world(5400,600)&#10;"/></LyricInfo></QrcInfos>', ".qrc")
        [(start, end, text, words)] = qrc(path)
        self.assertEqual((start, end, text), (5000, 6000, "Hello world"))
        self.assertEqual(words[1], (5400, 6000, "world"))

    def test_binary_file_is_rejected(self):
        path = Path(tempfile.NamedTemporaryFile(suffix=".krc", delete=False).name)
        path.write_bytes(b"krc1\x00\x01\x02\x03")
        with self.assertRaises(SystemExit):
            krc(path)


if __name__ == "__main__":
    unittest.main()

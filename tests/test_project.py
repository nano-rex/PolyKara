import copy
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from polykara import ass as ass_module
from polykara import project
from polykara.config import DEFAULT_CONFIG
from polykara.lyricformats import krc, ksc
from polykara.subtitle import lrc

ROW = {"id": "t", "title": "T", "artist": "A", "lyricist": "", "composer": "", "producer": "", "language": "en"}
ENTRIES = [
    (1000, 2600, "你好世界", [(1000, 1300, "你"), (1300, 1700, "好"), (1800, 2100, "世"), (2100, 2500, "界")]),
    (3000, 4500, "hello there", [(3000, 3500, "hello"), (3600, 4400, "there")]),
]


def config(**overrides):
    value = copy.deepcopy(DEFAULT_CONFIG)
    value["title"]["enabled"] = value["credit"]["enabled"] = False
    for section, values in overrides.items():
        value[section].update(values)
    return value


def write(text: str, suffix: str) -> Path:
    handle = tempfile.NamedTemporaryFile("w", suffix=suffix, delete=False, encoding="utf-8")
    handle.write(text)
    handle.close()
    return Path(handle.name)


class AssRoundTripTest(unittest.TestCase):
    def roundtrip(self, **overrides):
        with mock.patch.object(ass_module, "load_config", return_value=config(**overrides)):
            return project.parse_ass(ass_module.ass(ROW, ENTRIES))

    def test_single_layout_words_survive(self):
        entries = self.roundtrip(video={"extend_intro": False})
        self.assertEqual([text for _, _, text, _ in entries], ["你好世界", "hello there"])
        self.assertEqual(entries[0][3], ENTRIES[0][3])
        self.assertEqual(entries[1][3], ENTRIES[1][3])

    def test_alternate_layout_lead_in_and_effects_are_ignored(self):
        entries = self.roundtrip(video={"extend_intro": False}, layout={"mode": "alternate"}, effects={"preset": "pop"})
        self.assertEqual(entries[1][3], ENTRIES[1][3])
        self.assertEqual(entries[1][0], 3000)

    def test_intro_padding_is_removed(self):
        with mock.patch.object(ass_module, "load_config", return_value=config(title={"enabled": True, "duration_ms": 5000})):
            text = ass_module.ass(ROW, ENTRIES)
        self.assertIn("PolyKaraIntroPaddingMs: 4000", text)
        self.assertEqual(project.parse_ass(text)[0][3][0], (1000, 1300, "你"))


class ExportTest(unittest.TestCase):
    def test_lrc_export_reads_back(self):
        entries = lrc(write(project.export_lrc(ENTRIES), ".lrc"))
        self.assertEqual([word for _, _, word in entries[0][3]], ["你", "好", "世", "界"])
        self.assertEqual(entries[1][3][1][:2], (3600, 4400))

    def test_krc_export_reads_back(self):
        self.assertEqual(krc(write(project.export_krc(ENTRIES), ".krc"))[0][3], ENTRIES[0][3])

    def test_ksc_export_folds_gaps_into_previous_word(self):
        entries = ksc(write(project.export_ksc(ENTRIES), ".ksc"))
        self.assertEqual(entries[0][3][1], (1300, 1800, "好"))
        self.assertEqual([word for _, _, word in entries[1][3]], ["hello", "there"])

    def test_srt_export(self):
        self.assertIn("00:00:03,000 --> 00:00:04,500\nhello there", project.export_srt(ENTRIES))

    def test_json_roundtrip_sorts_and_drops_empty(self):
        data = project.to_json(list(reversed(ENTRIES)) + [(0, 10, "", [])])
        self.assertEqual(project.from_json(data), ENTRIES)

    def test_split_units(self):
        self.assertEqual(project.split_units("我爱 you|so much"), ["我", "爱", "you", "so", "much"])


class StorageTest(unittest.TestCase):
    def test_save_then_load_prefers_project(self):
        with tempfile.TemporaryDirectory() as tmp, \
                mock.patch.object(project, "PROJECTS", Path(tmp) / "project"), \
                mock.patch.object(project, "ASS", Path(tmp) / "ass"), \
                mock.patch.object(ass_module, "load_config", return_value=config()):
            edited = project.save_entries(ROW, ENTRIES)
            self.assertTrue(edited.name.endswith(".edited.ass"))
            entries, source = project.load_entries(ROW, Path(tmp))
            self.assertEqual((entries, source), (ENTRIES, "project"))
            (Path(tmp) / "project" / "t.json").unlink()
            entries, source = project.load_entries(ROW, Path(tmp))
            self.assertEqual(source, "edited ASS")
            self.assertEqual(entries[1][3], ENTRIES[1][3])


if __name__ == "__main__":
    unittest.main()

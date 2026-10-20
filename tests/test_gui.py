import csv
import tempfile
import unittest
from pathlib import Path
from unittest import mock

try:
    import tkinter  # noqa: F401
    from polykara import gui
except ImportError:  # Python built without Tk
    gui = None


@unittest.skipIf(gui is None, "tkinter is not available")
class SongListTest(unittest.TestCase):
    def test_new_ids_come_from_the_title_and_stay_unique(self):
        self.assertEqual(gui.new_id("I Know You're Hurting", set()), "i-know-you-re-hurting")
        self.assertEqual(gui.new_id("Wanna", {"wanna"}), "wanna-2")
        # Romanized when pypinyin is installed, otherwise a placeholder.
        self.assertIn(gui.new_id("我们的纪念", set()), {"wo-men-de-ji-nian", "song"})

    def test_songs_round_trip_and_keep_unknown_columns(self):
        with tempfile.TemporaryDirectory() as folder:
            manifest = Path(folder) / "songs.csv"
            manifest.write_text("id,url,title,extra\nabc,https://x,Song,keep\n", encoding="utf-8")
            with mock.patch.object(gui, "MANIFEST", manifest):
                fields, songs = gui.read_songs()
                self.assertIn("voice", fields)
                songs.append({"id": "new", "url": "https://y", "title": "歌", "voice": "female"})
                gui.write_songs(fields, songs)
                with manifest.open(encoding="utf-8") as handle:
                    rows = list(csv.DictReader(handle))
        self.assertEqual([row["id"] for row in rows], ["abc", "new"])
        self.assertEqual(rows[0]["extra"], "keep")
        self.assertEqual(rows[1]["title"], "歌")


def display_available() -> bool:
    if gui is None:
        return False
    try:
        tkinter.Tk().destroy()
        return True
    except tkinter.TclError:
        return False


class FakePlayer:
    def __init__(self):
        self.playing, self.now = True, 0

    def position(self):
        return self.now

    def play(self, path, at, speed=1.0):
        self.playing, self.now = True, at

    def stop(self):
        self.playing = False


@unittest.skipUnless(display_available(), "no display")
class TimingEditorTest(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        home = Path(self.folder.name)
        (home / "songs.csv").write_text("id,url,title,artist,language,lyrics_file\ndemo,https://x,Demo,Singer,zh,demo.lrc\n", encoding="utf-8")
        (home / "demo.lrc").write_text("[00:01.00]一二三四五六\n[00:05.00]七八九十百千\n", encoding="utf-8")
        patches = [mock.patch.object(gui, "MANIFEST", home / "songs.csv"), mock.patch("polykara.manifest.HOME", home), mock.patch("polykara.timing.EXTERNAL_LYRICS", home / "lyrics"),
                   mock.patch("polykara.verify.EXTERNAL_LYRICS", home / "lyrics"), mock.patch("polykara.verify.provider_files", return_value=[]), mock.patch("polykara.verify.downloaded_subtitle", return_value=None),
                   mock.patch("polykara.timing.word_json", return_value=None), mock.patch("polykara.timing.media_for", return_value=None), mock.patch("polykara.editor.messagebox")]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        self.home = home
        self.app = gui.App()
        self.addCleanup(self.app.destroy)
        self.editor = self.app.editor
        self.app.tabs.select(self.editor)
        self.editor.open("demo")
        self.editor.player = FakePlayer()
        self.editor.media = home / "demo.lrc"
        self.app.update()

    def tearDown(self):
        self.folder.cleanup()

    def hold(self, start, end):
        self.editor.player.now = start
        self.editor.key_down()
        self.editor.player.now = end
        self.editor.key_up()
        self.editor.release(end)

    def test_holding_space_times_each_character_in_turn(self):
        editor = self.editor
        self.assertEqual([line.text for line in editor.lines], ["一二三四五六", "七八九十百千"])
        self.assertEqual(editor.cursor, (0, 0))
        self.hold(1000, 1200)
        self.hold(1300, 1500)
        self.assertEqual([(token.start, token.end) for token in editor.lines[0].tokens[:2]], [(1000, 1200), (1300, 1500)])
        self.assertEqual(editor.cursor, (0, 2))
        editor.step_back()
        self.assertIsNone(editor.lines[0].tokens[1].start)
        self.assertEqual(editor.cursor, (0, 1))
        editor.undo()
        self.assertEqual(editor.lines[0].tokens[1].start, 1300)

    def test_tap_mode_ends_each_word_where_the_next_starts(self):
        editor = self.editor
        editor.mode.set(list(gui_editor().MODES)[1])
        for at in (1000, 1300, 1600, 1900, 2200):
            editor.player.now = at
            editor.key_down()
        self.hold(2500, 2800)
        self.assertEqual([(token.start, token.end) for token in editor.lines[0].tokens], [(1000, 1300), (1300, 1600), (1600, 1900), (1900, 2200), (2200, 2500), (2500, 2800)])
        self.assertEqual(editor.cursor, (1, 0))

    def test_dragging_a_word_on_the_timeline_and_saving(self):
        editor = self.editor
        self.hold(1000, 1200)
        self.hold(1300, 1500)
        # 1.4 s must be on screen whatever size the window manager gives the window.
        editor.pps = 200.0
        self.app.update()
        editor.timeline.xview_moveto(0)
        editor.draw_timeline()
        x = editor.x_of(1400) - editor.timeline.canvasx(0)
        y = (gui_editor().WORD_TOP + gui_editor().WORD_BOTTOM) / 2
        editor.press_timeline(mock.Mock(x=x, y=y))
        editor.drag_timeline(mock.Mock(x=x + 10, y=y))
        editor.release_timeline(None)
        self.assertEqual((editor.lines[0].tokens[1].start, editor.lines[0].tokens[1].end), (1350, 1550))
        editor.drag_timeline(mock.Mock(x=x - 500, y=y))  # no drag in progress: nothing moves
        self.assertTrue(editor.save())
        saved = (self.home / "lyrics" / "demo.timed.lrc").read_text(encoding="utf-8")
        self.assertIn("<00:01.350>二<00:01.550>", saved)
        self.assertFalse(editor.dirty)

    def test_progress_lines_show_the_song_being_worked_on(self):
        self.app.progress(">> download demo\n")
        self.assertEqual(self.app.tree.set("demo", "state"), "downloading…")
        self.app.progress(">> download other\n")
        self.assertNotEqual(self.app.tree.set("demo", "state"), "downloading…")


def gui_editor():
    from polykara import editor

    return editor


if __name__ == "__main__":
    unittest.main()

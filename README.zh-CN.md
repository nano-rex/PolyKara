# PolyKara

多语言 KTV 字幕自动化与可编辑制作工具。

[English README](README.md)

## Windows 安装程序

从 Releases 页面（或 Actions 中 **Windows installer** 的产物 `PolyKara-Setup`）下载 `PolyKara-Setup-<版本>.exe` 并运行即可，无需另外安装任何依赖：程序自带 ffmpeg、ffprobe、ffplay、yt-dlp、faster-whisper（CPU 对轴）、各语言罗马音引擎和 Pillow。安装到当前用户的 `%LOCALAPPDATA%\Programs\PolyKara`，不需要管理员权限。

- 开始菜单中的 **PolyKara** 打开图形界面：用 **Add song** 添加歌曲（视频网址、歌名、歌手、作曲、作词、语言、声部颜色），选择 **Make videos (all steps)** 后按 **Run**。日志与命令行输出相同。运行时，歌曲列表的状态栏会把待处理的歌曲标为 queued，并显示每首歌正在进行的步骤（下载、对轴、渲染……）。**Time lyrics** 在 **Timing editor** 页中打开选中的歌曲（见下文），**Edit ASS file** 用 Aegisub（或关联 `.ass` 的程序）打开字幕，**Settings** 以表单显示 `polykara.toml`（歌词、读音、布局和特效、标题和制作人员、视频、歌词搜索、播放器等分页，提供勾选框、下拉选项、字体列表和取色器，鼠标指向设置时显示说明；保存时只改动修改过的值并保留注释，**Advanced (file)** 页仍可直接编辑文本），YouTube 下载失败时用 **Update yt-dlp** 更新。
- 窗口是用 Rust 编写的原生 Windows 程序（`src/`，不使用任何第三方库），启动快、播放时不卡顿；歌词、读音和设置文件仍由与命令行相同的 Python 代码处理（`polykara-cli.exe engine`）。开始菜单中的 **PolyKara (classic window)** 打开旧的 Tk 窗口，便于对比试用。
- 歌曲列表、设置和工作文件保存在 `文档\PolyKara`，完成的视频在 `文档\PolyKara\work\output`。
- **PolyKara command line** 打开 `polykara-cli.exe` 命令行，命令与 `pipeline.py` 相同；`polykara-cli.exe selftest` 用自带工具渲染测试画面并检查缺失项。
- 第一次对轴时会下载一次 Whisper 模型（`small` 约 0.5 GB）。

自行构建：在 Windows 上运行 `packaging\build-windows.ps1`（需要 Python 3.11+ 和 Rust）。

## 快速开始

1. 编辑 `songs.csv`，填写歌曲 URL、资料、语言和歌词来源。
2. 检查依赖：

   ```bash
   python3 pipeline.py check
   ```

3. 运行完整流程：

   ```bash
   python3 pipeline.py process
   ```

   该命令依次下载视频和字幕、获取外部歌词、标准化音频、进行 word-level 对轴、生成 ASS 并渲染 MP4。

   已完成的工作会被复用：已有 MP4 的歌曲会跳过，已下载的视频和已有的对轴结果不会重复处理；如果 `.edited.ass` 比 MP4 新，会自动重新渲染。使用 `--reprocess` 可强制重新生成 ASS 和 MP4，加上 `--realign` 可同时重新对轴。

下载步骤分别检查视频和歌词，只获取缺少的部分。`work/raw` 中已有且可以播放的视频（用 `ffprobe` 在本地检查）不会重新下载；无法播放或没有音轨的文件（例如中断的下载或合并失败）会被删除并重新下载。下载步骤会一直收集歌词，直到歌曲至少有 3 个可用的歌词文件（见下方“歌词准确性检查”）；已满足的歌曲不会再请求字幕或歌词来源。所有来源都尝试过后会被记录，避免每次运行都重复查询；`python3 pipeline.py download --reprocess` 会重新尝试。

### 使用电脑上的音视频文件

可以直接使用电脑上的视频或音频文件代替网址：在图形界面按 **Add files** 选择文件，或在命令行运行 `python3 pipeline.py add 文件...`。歌名和歌手取自文件自带的标签，没有标签时从文件名读取（`歌手 - 歌名.mp4`、`01. Artist - Title.mkv`、`歌手-歌名.mp3`），并用于搜索歌词；语言按歌名文字推测，处理前请检查语言、歌手和声部。下载步骤会把文件链接或复制到 `work/raw`，不运行 yt-dlp。纯音频文件以内嵌封面图（没有时为纯色背景，`[render] background_color`）作为画面。

### 单首歌曲出错不会中断整批

每个步骤都逐首歌曲执行。某首歌在某一步失败时（下载错误、歌词文件缺失、对轴或 FFmpeg 报错），PolyKara 会输出 `SKIP <id>: <步骤> failed (<原因>)`，该歌曲不再进入后续步骤，其余歌曲继续处理。结束时会列出所有被跳过的歌曲及原因，并以状态码 1 退出。修正问题后重新运行同一命令即可，只会处理未完成的歌曲。

找不到任何歌词的歌曲也会被跳过，而且是在下载视频之前：流程先查找歌词，没有任何歌词文件时该歌曲会被搁置，并在结束时列出以便之后补全，单独运行后续步骤时也一样：`normalize`、`align`、`lyrics` 和 `render` 会跳过有视频但没有歌词文件的歌曲，继续处理下一首。这不算错误。为它添加 `lyrics_file` 或 `lyric_pages` 后重新运行即可。

只有影响全部歌曲的问题才会中止运行：缺少 `yt-dlp`/`ffmpeg`，或 `songs.csv`、`polykara.toml` 格式错误。

### 只处理指定歌曲

所有命令都可以在后面加歌曲 id，或使用 `--only`：

```bash
python3 pipeline.py process my-song another-song
python3 pipeline.py render --reprocess --only my-song
python3 pipeline.py edit my-song
```

## 区域热门歌曲

从 YouTube 区域 Top Songs 添加歌曲：

```bash
python3 pipeline.py download --trending
```

默认添加所有配置地区每区前 10 首歌曲。手动选择使用：

```bash
python3 pipeline.py download --trending --pick
```

## 对轴和歌词

```bash
pip install -r requirements-align.txt
python3 pipeline.py normalize
python3 pipeline.py align
python3 pipeline.py lyrics
python3 pipeline.py render --reprocess
```

### 歌词准确性检查

每个来源的歌词都单独保存为一个文件，以便互相比对。可信度顺序：在 Timing editor 中保存的时间轴（`work/lyrics/<id>.timed.lrc`）→ `songs.csv` 中的 `lyrics_file`（`.lrc`、`.srt`、`.vtt` 或纯文本 `.txt`）→ 上传者制作的 YouTube 字幕 → LRCLIB 同步歌词（`work/lyrics/<id>.lrclib.lrc`）→ 网易云音乐同步歌词（`<id>.netease.lrc`）→ 歌曲语言的 YouTube 自动字幕 → LRCLIB、Lyrics.ovh 和各个 `lyric_pages` 网页的纯文本歌词（`<id>.<来源>.txt`）。

一首歌至少需要 3 个可用的歌词文件才能进行可靠的准确性检查。下载步骤先查询歌词来源，文件仍不足时才请求 YouTube 字幕。如果仍然不足，请在 `songs.csv` 中添加 `lyrics_file` 或 `lyric_pages`（多个网址用 `|` 分隔）。

检查在 `download` 结束时、`lyrics` 开始时进行，也可以用 `python3 pipeline.py verify` 单独运行；它只读取歌词文件，不需要对轴。检查会逐词比对这些文件：包括用于渲染的文件在内，至少 3 个文件一致时歌词为“已验证”。没有 `lyrics_file` 时，会选用被其他文件确认的、可信度最高的带时间轴文件，因此单个错误来源会被否决；`lyrics_file` 始终会被使用，与其他文件矛盾时会给出警告。检查会输出每个文件的一致程度，并把结果保存到 `work/lyrics/<id>.check.json`；`qa` 会显示每首歌的结论。对轴完成后，`lyrics` 步骤还会输出歌词在音频识别结果中出现的比例。

未通过验证的歌曲仍会生成，但会给出警告并在运行结束时列出。在 `polykara.toml` 的 `[lyrics]` 中设置 `require_verified = true` 可改为跳过这些歌曲；`min_sources` 和 `agreement_threshold` 可调整要求。安装 `zhconv`（已包含在 `requirements-romanization.txt` 中）后，繁体和简体的同一份歌词会被视为一致。LRCLIB 查询会去掉标题中的 `(Official Video)` 等修饰，并选择时长最接近视频的版本；时长相差超过 5 秒时会给出警告。歌词来源可在 `polykara.toml` 的 `[lyrics] sources` 中设置，也可在 `songs.csv` 的 `lyric_sources` 中为单首歌曲覆盖。支持 LRCLIB、网易云音乐、Lyrics.ovh 和公开歌词网页容器。没有 word-level timing 时不会生成假高亮视频。

`align_model=auto` 会检测系统内存，默认固定预留 2 GiB 给系统；例如 8 GiB 内存会给对轴预算 6 GiB。可在 `polykara.toml` 或 `songs.csv` 中调整模型和阈值。

增强 LRC 示例：

```text
[00:12.00]<00:12.00>ありがとう<00:12.80>ございます
```

带逐字标记的行直接使用标记作为逐字时间，不需要对轴；同时支持 `[offset:±ms]` 和空时间戳行（表示上一句结束）。

识别出的词会与歌词文本进行匹配，而不是只按数量对应，多识别或漏识别一个词不会让整句错位；漏识别的词会放在前后已匹配的词之间。中文、日文、韩文按字高亮。完全没有识别结果的行使用平均时间，并在 `lyrics` 步骤后列出，方便在编辑器中检查。高亮从实际开唱时开始，句首等待和词间停顿都会写入 ASS。`[timing]` 中的 `lead_in_ms` 控制歌词提前出现的时间，`max_tail_ms`/`tail_hold_ms` 控制长间奏前歌词何时消失。

Logo 可在 `polykara.toml` 的 `[logo]` 中统一设置，或在 `songs.csv` 的 `logo_file` 中为单首歌曲设置；编码参数在 `[render]` 中设置。

非拉丁文字的歌词会在原文上方显示一行读音：中文为拼音，日文为罗马字，其他文字同理；`[romanization] margin_v` 设置这一行的高度。中文普通话使用带声调符号的拼音，如 `nǐ hǎo`（`pypinyin`；设置 `tone = "numbers"` 可改为 `ni3 hao3`），粤语（`language` 为 `yue` 或 `zh-hk`）使用粤拼（`ToJyutping`），日文使用平文式罗马字（`pykakasi`），韩文使用内置的韩语罗马字标记法，印度文字使用 `indic-transliteration`，西里尔、希腊、泰文等其他文字使用 `anyascii`。安装方法：`pip install -r requirements-romanization.txt`。缺少某个引擎时，`lyrics` 步骤会提示需要安装的包，该行只显示原文。`pykakasi` 对人名和部分汉字的读音可能有误，可以加入下面的读音词库。

**读音词库**：引擎会读错部分词（pypinyin 把“重来”读成 zhòng lái、“睡不着”读成 shuì bù zhe，pykakasi 把“君”读成 kun）。PolyKara 自带常见易错词的词库（`polykara/readings/zh.txt`、`ja.txt`、`yue.txt`），你自己的词放在 `<PolyKara 文件夹>/readings/<语言>.txt`（Windows 程序为 `文档\PolyKara\readings`），同一个词以你的为准；长词优先，所以“重来”可以是 chóng lái 而单独的“重”仍由引擎判断；“慢慢地”这类叠词后的“地”读 de。格式：普通话每个字一个音节（`重来 chóng lái` 或 `重来 chong2 lai2`），粤语每字一个粤拼（`銀行 ngan4 hong4`），日文写整个词的假名或罗马字（`君 きみ`）。在 Timing editor 页按 **Word readings…** 可以查看本曲每个字的读音并一键加入新词；编辑器预览会把读音显示在字的上方，下次 **Make subtitles** 时生效。请为这些歌曲使用含中日韩字形的字体，例如 `Noto Sans CJK SC`。

长于 30 秒的无人声间隔会在最后三秒显示 `.`, `..`, `...`，每秒一个状态，位置在下一句歌词上方并左对齐。未标记歌词默认为蓝色；最多支持 16 位演唱者：

```text
[00:12.00][singer:A]First singer line
[00:18.00][singer:B]Second singer line
[00:24.00][singer:duet]Together line
```

## 布局和特效

`[layout] mode = "single"`（默认）每次显示一行。`"alternate"` 为 KTV 式上下两行交替：歌词在上行（`top_alignment`）和下行（`bottom_alignment`）之间交替，并提前 `lead_in_ms` 出现，方便提前看词。每个字在演唱时以 `[karaoke] active_color` 填色（`tag = "kf"` 为平滑填色）；标记了 `[singer: 名字]` 的行按歌手使用不同颜色。

`[effects] preset` 为每个字加特效：`none`、`fade`、`pop` 或 `glow`；也可以在 `syllable` 和 `line` 中写自己的 ASS 标签，`{start}` `{mid}` `{end}` `{dur}` `{index}` 为相对该行的毫秒数，`[effects.params]` 中的键也可使用。其他比例的视频会用模糊放大的画面填满 1920x1080（`[render] fit = "blur"`，`"pad"` 为黑边，`"none"` 保持原样）。

## 手动对轴（Timing editor）

窗口中的 **Timing editor** 页可以边播放歌曲边手动对轴，用法类似傻丫头字幕精灵（Sayatoo）。自动对轴不准，或网上找不到歌词时（用 **Lyrics text…** 粘贴歌词）都可以使用。

1. 在 **Songs** 页选中一首已下载的歌曲，按 **Time lyrics**（或在编辑器的歌曲列表中选择）。编辑器会先提取一次音频并画出波形。
2. 按 **Enter** 播放。高亮的字（中日韩文为一个字，其他语言为一个词）就是下一个要对的字。唱到它时**按住空格**：按下为开始，松开为结束。**Tap** 模式下每按一次就开始下一个字，同时结束上一个字。唱得太快时可以用 **Speed** 放慢；总是按晚的话可以设置 **Reaction delay**。
3. **Backspace** 撤销上一个字并回退 2 秒；**↑/↓** 跳到上一行或下一行；**←/→** 前后跳 3 秒；点击歌词中的字可以从那里重新开始。
4. 时间轴上拖动一个字可以移动它，拖动边缘可以改开始或结束时间；点击空白处移动播放位置，用 Ctrl+滚轮或缩放按钮缩放。**Shift+←/→** 把选中的字前后微调 10 毫秒，**Ctrl+Z** 撤销，**Clear line timing** 清除一行重新对。右侧预览按视频中的颜色显示正在唱的行。
5. **Save**（Ctrl+S）保存为 `work/lyrics/<id>.timed.lrc`（每个字都带时间的增强 LRC）。之后这个文件就是这首歌的歌词来源，优先于其他所有来源，也不再需要自动对轴。**Save and make subtitles** 同时重新生成字幕；**Preview on video** 用 ffplay 从播放位置前一点开始播放带字幕的视频，无需渲染。然后用 **Render videos only** 渲染：字幕在上次渲染后有改动的歌曲会自动重新渲染，不必勾选 **Redo finished work**。

还没有对的行会保存在前后两行之间的估计位置，编辑器会提示有几行。手动修改过的 `.edited.ass` 会覆盖编辑器的时间轴，所以编辑器会提出把它改名为 `.edited.ass.bak`。

## GUI 修改和清理

```bash
python3 pipeline.py edit
python3 pipeline.py qa
python3 pipeline.py cleanup --dry-run
python3 pipeline.py cleanup
```

`work/ass/*.edited.ass` 不会被自动流程覆盖；保存修改后运行 `python3 pipeline.py render <id>` 即可重新渲染；`python3 pipeline.py preview <id> --at 60` 从第 60 秒开始播放带字幕的视频，无需渲染。`qa` 会显示每首歌的素材、歌词来源、对轴、ASS 和 MP4 状态。

运行单元测试（不需要素材、网络或额外依赖）：

```bash
python3 -m unittest discover -s tests -t .
cargo test
```
确认有最终 MP4 和编辑后的 ASS 后，才使用 `cleanup --drop-source` 删除原始视频。请遵守下载平台条款和相关版权许可。

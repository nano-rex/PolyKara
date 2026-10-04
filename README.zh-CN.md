# PolyKara

多语言 KTV 字幕自动化与可编辑制作工具。

[English README](README.md)

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

每个来源的歌词都单独保存为一个文件，以便互相比对。可信度顺序：`songs.csv` 中的 `lyrics_file`（`.lrc`、`.srt`、`.vtt` 或纯文本 `.txt`）→ 上传者制作的 YouTube 字幕 → LRCLIB 同步歌词（`work/lyrics/<id>.lrclib.lrc`）→ 网易云音乐同步歌词（`<id>.netease.lrc`）→ 歌曲语言的 YouTube 自动字幕 → LRCLIB、Lyrics.ovh 和各个 `lyric_pages` 网页的纯文本歌词（`<id>.<来源>.txt`）。

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

非拉丁文字的歌词会把读音直接标在对应的字上方：每个汉字上方是拼音，日文每个词上方是罗马字，韩文每个音节上方是罗马字，其他文字同理；读音和原字同步高亮，字距会自动调整，避免较长的拼音与相邻的字重叠。布局根据 libass 实际使用的字体宽度计算（安装 Pillow 和 fontconfig 时精确测量，否则使用安全的估算值，字距会稍宽）；过长的行会自动缩小。`romanization.ruby_spacing` 调整字距；`layout = "rows"` 可改为在原文上方显示一整行读音，在 Aegisub 中更便于编辑。中文普通话使用带声调符号的拼音，如 `nǐ hǎo`（`pypinyin`；设置 `tone = "numbers"` 可改为 `ni3 hao3`），粤语（`language` 为 `yue` 或 `zh-hk`）使用粤拼（`ToJyutping`），日文使用平文式罗马字（`pykakasi`），韩文使用内置的韩语罗马字标记法，印度文字使用 `indic-transliteration`，西里尔、希腊、泰文等其他文字使用 `anyascii`。安装方法：`pip install -r requirements-romanization.txt`。缺少某个引擎时，`lyrics` 步骤会提示需要安装的包，该行只显示原文。`pykakasi` 对人名和部分汉字的读音可能有误，可在编辑后的 ASS 中修正。拼音行的位置会根据 `[lyric]` 的字号和边距自动放在歌词上方（`romanization.margin_v = "auto"`），也可设为具体数值；请为这些歌曲使用含中日韩字形的字体，例如 `Noto Sans CJK SC`。

长于 30 秒的无人声间隔会在最后三秒显示 `.`, `..`, `...`，每秒一个状态，位置在下一句歌词上方并左对齐。未标记歌词默认为蓝色；最多支持 16 位演唱者：

```text
[00:12.00][singer:A]First singer line
[00:18.00][singer:B]Second singer line
[00:24.00][singer:duet]Together line
```

## GUI 修改和清理

```bash
python3 pipeline.py edit
python3 pipeline.py qa
python3 pipeline.py cleanup --dry-run
python3 pipeline.py cleanup
```

`work/ass/*.edited.ass` 不会被自动流程覆盖；保存修改后运行 `python3 pipeline.py render <id>` 即可重新渲染。`qa` 会显示每首歌的素材、歌词来源、对轴、ASS 和 MP4 状态。

运行单元测试（不需要素材、网络或额外依赖）：

```bash
python3 -m unittest discover -s tests -t .
```
确认有最终 MP4 和编辑后的 ASS 后，才使用 `cleanup --drop-source` 删除原始视频。请遵守下载平台条款和相关版权许可。

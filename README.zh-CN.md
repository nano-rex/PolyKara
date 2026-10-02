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

### 单首歌曲出错不会中断整批

每个步骤都逐首歌曲执行。某首歌在某一步失败时（下载错误、歌词文件缺失、对轴或 FFmpeg 报错），PolyKara 会输出 `SKIP <id>: <步骤> failed (<原因>)`，该歌曲不再进入后续步骤，其余歌曲继续处理。结束时会列出所有被跳过的歌曲及原因，并以状态码 1 退出。修正问题后重新运行同一命令即可，只会处理未完成的歌曲。

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

歌词优先级：`songs.csv` 中的 `lyrics_file`（`.lrc`、`.srt`、`.vtt`，或由对轴结果计时的纯文本 `.txt`）→ 上传者制作的 YouTube 字幕 → 经过标题、歌手和跨来源文本验证的同步歌词 → 歌曲语言的 YouTube 自动字幕（可用 `lyrics.auto_captions = false` 关闭）→ `lyric_pages` 网页歌词。某个来源没有可用歌词行时会自动尝试下一个。LRCLIB 查询会去掉标题中的 `(Official Video)` 等修饰，并选择时长最接近视频的版本；时长相差超过 5 秒时会给出警告。歌词来源可在 `polykara.toml` 的 `[lyrics] sources` 中设置，也可在 `songs.csv` 的 `lyric_sources` 中为单首歌曲覆盖。支持 LRCLIB、Lyrics.ovh 和公开歌词网页容器。没有 word-level timing 时不会生成假高亮视频。

`align_model=auto` 会检测系统内存，默认固定预留 2 GiB 给系统；例如 8 GiB 内存会给对轴预算 6 GiB。可在 `polykara.toml` 或 `songs.csv` 中调整模型和阈值。

增强 LRC 示例：

```text
[00:12.00]<00:12.00>ありがとう<00:12.80>ございます
```

带逐字标记的行直接使用标记作为逐字时间，不需要对轴；同时支持 `[offset:±ms]` 和空时间戳行（表示上一句结束）。

识别出的词会与歌词文本进行匹配，而不是只按数量对应，多识别或漏识别一个词不会让整句错位；漏识别的词会放在前后已匹配的词之间。中文、日文、韩文按字高亮。完全没有识别结果的行使用平均时间，并在 `lyrics` 步骤后列出，方便在编辑器中检查。高亮从实际开唱时开始，句首等待和词间停顿都会写入 ASS。`[timing]` 中的 `lead_in_ms` 控制歌词提前出现的时间，`max_tail_ms`/`tail_hold_ms` 控制长间奏前歌词何时消失。

Logo 可在 `polykara.toml` 的 `[logo]` 中统一设置，或在 `songs.csv` 的 `logo_file` 中为单首歌曲设置；编码参数在 `[render]` 中设置。

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

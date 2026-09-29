# PolyKara

多语言 KTV 字幕自动化与可编辑制作工具

这个目录实现 CLI 批处理和 GUI 手工修正的两层流程。ASS 是可编辑项目文件；KAX/KAS 仍由 Sayatoo 自己保存。

## 使用方式

1. 编辑 `songs.csv`。填写 `id`、`url`、歌曲资料和 `lyrics_file`。`id` 只能使用英文字母、数字、短横线或下划线。
2. 检查依赖：

   ```bash
   python3 pipeline.py check
   ```

   `yt-dlp` 必须是较新的版本。若出现 YouTube `HTTP 400`、`Precondition check failed` 或 `Only images are available`，先更新它：

   - Windows：`winget upgrade yt-dlp`，或从 [yt-dlp Releases](https://github.com/yt-dlp/yt-dlp/releases) 下载最新 `yt-dlp.exe`。
   - Debian/Ubuntu：发行版的 apt 版本可能过旧，优先使用官方 standalone binary 或发行版 backports。
   - 更新后确认：`yt-dlp --version`。

3. 先预览命令，不下载：

   ```bash
   python3 pipeline.py download --dry-run
   ```

4. 下载视频：

   ```bash
   python3 pipeline.py download
   ```

5. 提取统一的 48 kHz FLAC，供后续歌词对轴使用。FLAC 比 PCM WAV 更省空间且无损：

   ```bash
   python3 pipeline.py normalize
   ```

6. 运行自动 word-level 对轴：

   ```bash
   pip install -r requirements-align.txt
   python3 pipeline.py align
   ```

   结果保存在 `work/align/<id>.json`。如果 WhisperX 不可用，PolyKara 会自动使用较轻量的 Faster-Whisper，并保留逐词时间戳；生产环境应先按语言抽样验证，再批量处理。

   `align_model=auto` 会检测系统总内存，并预留一半给操作系统和其他程序：8 GB 总内存会得到约 4 GB 对齐预算并使用 `small`，16 GB 使用 `medium`，32 GB 左右才会使用 `large-v3`。如需固定模型，可在 `songs.csv` 中改为 `tiny`、`base`、`small`、`medium` 或 `large-v3`。

   下载阶段同时会保存 YouTube 提供的人工字幕和自动字幕到 `work/subtitles/`，并从 `lyric_sources` 指定的来源获取同步歌词到 `work/lyrics/`。当前内置 `lrclib` 和 `lyrics.ovh`：LRCLIB 提供同步 LRC，Lyrics.ovh 用于交叉核对歌词文本；`webpage` 会解析 `lyric_pages` 中网页的歌词容器（例如 Genius 的 `data-lyrics-container`）。用 `subtitle_langs` 控制 YouTube 字幕语言，例如 `zh.*,yue,ja,ko,ta,en.*`。

   歌词优先级为：YouTube 字幕 → 通过标题/歌手元数据和跨来源文本相似度验证的外部同步歌词 → `lyrics_file`。如果外部来源互相矛盾，PolyKara 会拒绝自动采用并提示人工审核，避免静默生成错误歌词。外部结果会缓存，避免重复请求。

   网页解析只读取公开 HTML 中明确标记的歌词区域，不绕过登录、付费墙、验证码、反爬措施或访问限制；每首歌可在 `lyric_pages` 中用 `|` 分隔多个公开页面 URL。

   如果网页只提供普通文本而没有 LRC，PolyKara 会缓存提取结果，并在已有 word-level 对轴后按识别出的词时间为网页歌词建立行边界，再生成 karaoke ASS。

7. 将匹配实际音频版本的带时间 LRC 放在 `lyrics_file` 指定位置，然后生成自动 ASS：

   ```bash
   python3 pipeline.py lyrics
   ```

8. 创建可编辑副本并在 Aegisub 或 Subtitle Edit 中修改：

   ```bash
   python3 pipeline.py edit
   ```

   这会创建 `work/ass/<id>.edited.ass`，不会覆盖自动生成的 `.auto.ass`。

9. 渲染最终视频。若存在 edited 文件，自动优先使用它：

   ```bash
   python3 pipeline.py render
   ```

   If a final MP4 already exists, the command asks whether that song should be
   rendered again. Existing outputs are skipped automatically in non-interactive
   runs. Use `--reprocess` to render every song again without prompting:

   ```bash
   python3 pipeline.py render --reprocess
   ```

   正式渲染要求 ASS 内含 `\\k` 逐词/逐音节时间标签。若 word timing 不存在，`lyrics` 会自动启动 WhisperX 对轴；如果 WhisperX 未安装，流程会明确失败，不会生成假 karaoke 高亮。

   增强 LRC 的格式示例：

   ```text
   [00:12.00]<00:12.00>ありがとう<00:12.80>ございます
   ```

   这些尖括号时间必须来自实际音频对轴；仅把一行平均切成几个词不能保证可唱。

   只有当下一句歌词前有至少 30 秒无人声空档时，才会显示长暂停提示：在最后 3 秒依次显示 `.`, `..`, `...`，每个状态持续 1 秒。提示位于歌词上方并左对齐；普通歌词间隔不会显示点号。

   歌词最多支持 16 位演唱者。歌词行可选用 `[singer:1]` 到 `[singer:16]`、`[singer:A]`、`[singer:B]`、`[singer:duet]` 或 `[singer:backing]` 标记演唱者。未标记歌词默认使用蓝色；演唱者按首次出现顺序分配稳定颜色，同时保留逐词高亮。例如：

   ```text
   [00:12.00][singer:A]First singer line
   [00:18.00][singer:B]Second singer line
   [00:24.00][singer:duet]Together line
   ```

10. 查看处理状态：

   ```bash
   python3 pipeline.py qa
   ```

   也可以用一个命令运行下载、音频标准化、对齐、歌词生成和渲染：

   ```bash
   python3 pipeline.py process
   ```

   已存在的最终 MP4 会逐首询问是否重新处理；无人值守运行会跳过它们。需要全部重新渲染时使用 `python3 pipeline.py process --reprocess`。GUI 编辑和清理文件仍需显式执行。

11. 清理可重复生成的中间音频：

   ```bash
   python3 pipeline.py cleanup --dry-run
   python3 pipeline.py cleanup
   ```

   默认只删除 FLAC 中间文件。确认已经有最终 MP4 和 `*.edited.ass` 后，如需释放原始下载视频，再明确使用 `--drop-source`：

   ```bash
   python3 pipeline.py cleanup --drop-source
   ```

   原始视频删除后不能从本地重新渲染，必须重新下载；字幕、ASS、对轴 JSON 和最终 MP4 会保留。

文件会放在：

- `work/raw/`：原始下载文件
- `work/audio/`：统一音频
- `work/download-archive.txt`：yt-dlp 下载记录，避免重复下载
- `work/metadata/`：歌曲资料
- `work/ass/*.auto.ass`：自动生成、可以重新生成
- `work/ass/*.edited.ass`：人工修正后的项目文件，自动流程不会覆盖
- `work/output/`：最终 MP4

仓库内的两个 LRC 是用于流程冒烟测试的粗略行时间轴，不代表最终对轴结果，也不会产生逐词高亮。正式制作应优先使用带逐字/逐音节时间的增强 LRC；若没有，应使用 `utasub`、`song`、WhisperX 或其他本地强制对齐工具生成 word-level timing，再导入 PolyKara。请确保下载和歌词使用符合相应平台条款及版权许可。

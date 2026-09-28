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
   pip install whisperx
   python3 pipeline.py align
   ```

   结果保存在 `work/align/<id>.json`。WhisperX 需要较大的模型和足够的内存；生产环境应先按语言抽样验证，再批量处理。

   下载阶段同时会保存 YouTube 提供的人工字幕和自动字幕到 `work/subtitles/`。用 `subtitle_langs` 控制语言，例如 `zh.*,yue,ja,ko,ta,en.*`；下载到的字幕优先于 `lyrics_file`。

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

   正式渲染要求 ASS 内含 `\\k` 逐词/逐音节时间标签。只有需要检查片头、Logo 或版式时，才使用 `--allow-line-only` 生成无 karaoke 高亮的预览。

   增强 LRC 的格式示例：

   ```text
   [00:12.00]<00:12.00>ありがとう<00:12.80>ございます
   ```

   这些尖括号时间必须来自实际音频对轴；仅把一行平均切成几个词不能保证可唱。

   每句歌词默认会在开始前显示 `...` 1 秒。可在 `songs.csv` 的 `prompt_ms` 中调整，例如 `500` 表示提前半秒，`0` 表示关闭。

10. 查看处理状态：

   ```bash
   python3 pipeline.py qa
   ```

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

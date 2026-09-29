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

   该命令依次下载视频和字幕、获取外部歌词、标准化音频、进行 word-level 对轴、生成 ASS 并渲染 MP4。已有输出会询问是否重新处理；无人值守运行会跳过，使用 `--reprocess` 可强制重新渲染。

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

PolyKara 优先使用 YouTube 字幕，其次使用经过标题、歌手和跨来源文本验证的同步歌词，最后使用 `lyrics_file`。歌词来源可在 `polykara.toml` 的 `[lyrics] sources` 中设置，也可在 `songs.csv` 的 `lyric_sources` 中为单首歌曲覆盖。支持 LRCLIB、Lyrics.ovh 和公开歌词网页容器。没有 word-level timing 时不会生成假高亮视频。

`align_model=auto` 会检测系统内存，默认固定预留 2 GiB 给系统；例如 8 GiB 内存会给对轴预算 6 GiB。可在 `polykara.toml` 或 `songs.csv` 中调整模型和阈值。

增强 LRC 示例：

```text
[00:12.00]<00:12.00>ありがとう<00:12.80>ございます
```

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

`work/ass/*.edited.ass` 不会被自动流程覆盖。确认有最终 MP4 和编辑后的 ASS 后，才使用 `cleanup --drop-source` 删除原始视频。请遵守下载平台条款和相关版权许可。

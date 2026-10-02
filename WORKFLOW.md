# PolyKara CLI + GUI workflow

The project has two deliberately separate layers:

```text
CLI input/download/alignment
        |
        v
  song.auto.ass  ---->  Aegisub or Subtitle Edit
                              |
                              v
                        song.edited.ass
                              |
                              v
                         final MP4
```

## CLI responsibilities

The command-line pipeline handles repeatable work:

- download and cache source media
- normalize audio
- obtain or import lyrics
- calculate initial line/word timings
- create `song.auto.ass`
- run timing and media checks
- render the final MP4

## GUI responsibilities

Open `song.auto.ass` in Aegisub for detailed adjustments. Save the result as
`song.edited.ass`; never save over the automatic file. Aegisub is useful for
waveform timing, styling, karaoke tags, and real-time preview. Subtitle Edit is
an alternative when waveform correction and format conversion are the priority.

Typical manual changes include:

- shifting one line or one word
- correcting a repeated chorus
- changing male/female/duet colors
- changing subtitle height or prompt-light effect
- correcting line breaks and spelling
- adjusting Logo and credit placement

## Non-destructive rules

1. `song.auto.ass` may be regenerated at any time.
2. `song.edited.ass` must never be overwritten automatically.
3. Rendering uses `song.edited.ass` when it exists; otherwise it uses `song.auto.ass`.
   An edited file that is newer than the MP4 is rendered again automatically.
4. Final MP4 files are derivatives and can always be recreated.
5. Keep the original LRC/KRC and timing JSON beside the ASS files.

This means a small correction requires only opening the edited ASS file and
rerunning the render step for that song (`python3 pipeline.py render <id>`);
downloading and alignment do not need to run again.

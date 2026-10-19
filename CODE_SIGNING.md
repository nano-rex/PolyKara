# Code signing policy

Free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

## What is signed

Windows releases are built from this repository by the GitHub Actions workflow [`.github/workflows/windows.yml`](.github/workflows/windows.yml) on GitHub-hosted runners. The workflow submits the build to SignPath, which signs:

- `PolyKara.exe` and `polykara-cli.exe`, PolyKara's own programs ([`.signpath/artifact-configuration/program.xml`](.signpath/artifact-configuration/program.xml))
- `PolyKara-Setup-<version>.exe`, the installer ([`.signpath/artifact-configuration/installer.xml`](.signpath/artifact-configuration/installer.xml))

Third-party components bundled in the installer (ffmpeg, ffprobe, yt-dlp, the Python runtime and libraries) are included as their publishers distribute them and are not signed with this certificate.

Only builds from `v*` release tags are signed with the release certificate. Every release signing request is approved by hand.

## Team roles

- Committers and reviewers: [nano-rex](https://github.com/nano-rex)
- Approvers: [nano-rex](https://github.com/nano-rex)

## Privacy

PolyKara does not send any information to other networked systems unless the user asks for it. Network connections are made only for the work the user starts: downloading the videos and subtitles listed in `songs.csv` (yt-dlp), looking up lyrics from the providers configured in `polykara.toml`, downloading the Whisper speech model the first time word timing runs, and updating yt-dlp when the user presses **Update yt-dlp**.

## Setting up signing (maintainers)

1. Apply for the SignPath Foundation open-source program at <https://signpath.org/apply> with this repository.
2. After approval, in SignPath: create the project with slug `PolyKara`, link it to this GitHub repository, add the artifact configurations `program` and `installer` from `.signpath/artifact-configuration/`, and use the signing policies `test-signing` and `release-signing`.
3. In SignPath, create a CI user and an API token for it, and give it submitter rights on both signing policies.
4. In GitHub, under Settings → Secrets and variables → Actions:
   - secret `SIGNPATH_API_TOKEN`: the CI user's API token
   - variable `SIGNPATH_ORGANIZATION_ID`: the organization id from SignPath
   - variable `SIGNPATH_PROJECT_SLUG` (optional): only if the project slug is not `PolyKara`

The workflow signs as soon as the secret and the organization id exist; until then it builds unsigned installers.

# helloim.ai — a voice assistant for Windows and Linux that runs on your machine

helloim.ai is a personal AI voice assistant for Windows and Linux. It talks,
it listens, and it uses **your** AI key — your chat goes straight to the AI provider you choose, and no server of ours ever sees it.

- **Your chat and key go straight to your AI, not through us.** Bring your own brain (Claude, OpenAI, Gemini, or a local model); your chat and key go straight to it. Pick a local model and the model call stays on your machine. (The optional Google connector routes Gmail/Calendar tokens through us — the one exception.)
- **Good to know.** Sign-in uses a helloim.ai account (handles login and your plan, never your chat). Voice is transcribed by your device's own speech service, not by us; playback is on-device.
- **Free to use.** The assistant, its faces and its voices are free.
- **Source-available, so "private" is something you can verify** — not just
  trust. The code is here to read.
- **Windows** and a separate **Linux (amd64)** installer.

## License — free to use, not for resale

This project is **source-available and free to use**, released under the MIT
license **with the Commons Clause**. In plain terms: you may use, run, read,
and modify it freely — you may **not sell it or resell it** as a product or
service. See [LICENSE](LICENSE) for the exact terms.

## Repository layout

- [Windows](Windows/): the published source in [windows-source](Windows/windows-source/).
- [linux](linux/): Linux installation instructions.

## Download

- [Windows installer](https://github.com/mdalton24/helloim.ai/releases/latest/download/helloim.ai-Setup.exe) — v1.6.1, from the latest GitHub release.
- [Linux AppImage](https://github.com/mdalton24/helloim.ai/releases/latest/download/helloim.ai-x86_64.AppImage) — v1.6.1, amd64. Recommended; runs on most distributions.
- [Linux .deb](https://github.com/mdalton24/helloim.ai/releases/latest/download/helloim.ai-amd64.deb) — v1.6.1, amd64, for Debian/Ubuntu; see [Linux instructions](linux/README.md).


**v1.6.1 is out.** Get the installers from the [latest release](https://github.com/mdalton24/helloim.ai/releases/latest). It's an unsigned build, so Windows shows a SmartScreen warning on first run — the release notes explain exactly what to click and include a SHA-256 checksum to verify the download.

## Status

Released — v1.6.1, free. Support and community: Discord.

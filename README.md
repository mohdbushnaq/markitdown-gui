<p align="center">
  <img src="assets/icon.png" width="96" alt="MarkItDown GUI icon">
</p>

<h1 align="center">MarkItDown GUI</h1>

<p align="center">
  A small, portable Windows app for converting documents to Markdown with
  <a href="https://github.com/microsoft/markitdown">Microsoft MarkItDown</a>.<br>
  Drag files in or paste a web page URL, click Convert, get clean <code>.md</code> files. No installer, no command line.
</p>

<p align="center">
  <a href="https://github.com/mohdbushnaq/markitdown-gui/releases/latest"><img src="https://img.shields.io/github/v/release/mohdbushnaq/markitdown-gui?label=download" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6" alt="Windows 10 and 11">
  <img src="https://img.shields.io/badge/built%20with-Rust-B7410E" alt="Built with Rust">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/mohdbushnaq/markitdown-gui" alt="MIT license"></a>
</p>

<p align="center">
  <img src="docs/screenshots/main.png" width="820" alt="MarkItDown GUI after converting a batch of files">
</p>

> **Unofficial project.** MarkItDown GUI is an independent front-end. It is not made by or affiliated with Microsoft.
> All document conversion is done by the official, unmodified MarkItDown.

## Contents

- [Download](#download)
- [Install on this PC (optional)](#install-on-this-pc-optional)
- [First run](#first-run)
- [How to use](#how-to-use)
- [Features](#features)
- [Options](#options)
- [Updates](#updates)
- [Supported formats](#supported-formats)
- [Where files are stored](#where-files-are-stored)
- [Troubleshooting](#troubleshooting)
- [Build from source](#build-from-source)
- [Credits](#credits)

## Download

1. Open the [latest release](https://github.com/mohdbushnaq/markitdown-gui/releases/latest).
2. Download **`MarkItDown-GUI.exe`** (about 7 MB).
3. Put it in any folder you like and double-click it. You can rename it if you like.

That's it. The app is a single portable `.exe` that runs on any 64-bit Windows 10 or 11 PC
without extra DLLs or admin rights. If you prefer a regular installed app, see the next section.

> Windows may show a SmartScreen warning the first time because the app is not code-signed.
> Click **More info**, then **Run anyway**.

## Install on this PC (optional)

Portable is the default, but the app can also install itself like a normal Windows program:

1. Open **Options** and, under **App**, click **Install on this PC** (optionally with a desktop shortcut).
2. The app is copied to `%LOCALAPPDATA%\Programs\MarkItDown GUI`, a **Start menu** shortcut is added, and
   **MarkItDown GUI** appears in **Settings > Apps > Installed apps**.
3. Your existing MarkItDown runtime and settings are moved along, so nothing is downloaded again.
4. Click **Open installed app** to switch to the installed copy.

No admin rights are needed because it installs for your Windows user only.
To remove it, use **Uninstall** under **Options > App**, or uninstall it from Windows Settings like any other app.

## First run

On first launch the app sets everything up by itself. You do **not** need to install Python.

1. It looks for an existing MarkItDown on your PC.
2. If there is none, it downloads the official embeddable Python from python.org into a private
   `MarkItDown GUI runtime` folder next to the `.exe`.
3. It installs `markitdown[all]` into that private Python and checks that it starts.

Progress is shown in a log window. This takes a few minutes and about 500 MB of disk space, once.
Nothing is installed system-wide, nothing is added to PATH, and no admin rights are needed.

<p align="center">
  <img src="docs/screenshots/update.png" width="700" alt="Setup and update log window">
</p>

## How to use

<p align="center">
  <img src="docs/screenshots/empty.png" width="700" alt="Empty window with drop zone">
</p>

1. **Add files or web pages.** Drag files or whole folders onto the window, click the drop zone, or use **Add files** / **Add folder**.
   You can also drop files onto the `.exe` itself.
   **Or** paste a web page address into the URL field (under the drop zone, or at the top of the list) and press **Enter** or **Add URL**.
2. **Choose where to save.** By default each `.md` is saved **next to its original file**, and web pages go to your
   **Downloads** folder. Pick **Folder:** to send everything to one folder instead.
3. **Click Convert.** Each file shows its status: Ready, Queued, a spinner while converting, then **Done** or the error.
4. **Open the result.** Use **Open** to open the Markdown file, or **Folder** to show it in File Explorer.
   Failed files have a **Retry** button, and hovering the error shows the full message.

Click **Stop** at any time to cancel. Running conversions are stopped immediately.

## Features

- Batch conversion with drag and drop of files and folders
- Convert web pages by URL (saved as `site - page.md`)
- Saves next to each input file by default, or to a folder you choose
- Never overwrites existing files unless you allow it (uses `name (1).md` instead)
- Converts several files in parallel (1 to 8 at a time)
- Per-file Open, Show in folder, Retry and full error details
- Every MarkItDown command-line option is available as a setting (see below)
- Automatic first-run setup with a private Python runtime
- Portable by default, or one-click install with Start menu shortcut and a normal Windows uninstaller
- Checks for new versions of the app and of MarkItDown, and updates either with one click
- Remembers your settings between sessions
- Light on resources: a native Rust app (egui), about 7 MB

## Options

Click **Options** in the top-right corner.

<p align="center">
  <img src="docs/screenshots/options.png" width="700" alt="Options panel">
</p>

| Setting | What it does | MarkItDown flag |
|---|---|---|
| Overwrite existing .md files | Replace files with the same name instead of creating `name (1).md` | |
| Keep embedded images (data URIs) | Keep images inside the Markdown as base64 instead of placeholders | `--keep-data-uris` |
| Open output folder when done | Opens the folder in File Explorer after a batch finishes | |
| Parallel jobs | How many files convert at the same time | |
| Use 3rd-party plugins / List | Enable installed MarkItDown plugins and list them | `-p`, `--list-plugins` |
| Extension, MIME type, Charset | Hints for files with missing or wrong extensions | `-x`, `-m`, `-c` |
| Azure Document Intelligence | Higher quality conversion of PDFs and scans with Azure | `-d`, `-e` |
| Azure Content Understanding | Analyze documents, images, audio and video with Azure | `--use-cu`, `--cu-endpoint`, `--cu-analyzer`, `--cu-file-types` |
| Azure API key | Optional key for the Azure options. Never saved to disk | `AZURE_API_KEY` |
| markitdown command | Use a specific `markitdown.exe` or `python.exe` instead of auto-detect | |

## Updates

<p align="center">
  <img src="docs/screenshots/updates.png" width="300" alt="App and Updates sections of the Options panel">
</p>

**Options > Updates** has a **Check for updates** button and checks automatically on startup
(turn this off with **Check for updates on startup**). It covers two things:

- **MarkItDown GUI (this app).** Compares your version with the [latest release here](https://github.com/mohdbushnaq/markitdown-gui/releases/latest).
  When a newer one exists, click **Update app** (or the green **App x.y.z available** button in the title bar).
  The new version is downloaded, swapped in and restarted automatically, for both portable and installed copies.
- **MarkItDown (converter).** Compares your MarkItDown with the [latest Microsoft release](https://github.com/microsoft/markitdown/releases)
  (PyPI as a fallback). **Upgrade now**, or the **Update to x.y.z** button in the title bar, runs
  `python -m pip install --upgrade "markitdown[all]"` and shows the output live.

## Supported formats

Anything MarkItDown supports, including:

PDF, Word (`.docx`), PowerPoint (`.pptx`), Excel (`.xlsx`, `.xls`), HTML, CSV, JSON, XML, EPUB, ZIP archives,
Outlook messages (`.msg`), images (EXIF metadata), audio (speech transcription) and web pages by URL
(including Wikipedia articles, YouTube video pages with transcripts, RSS feeds and Bing search results).

## Where files are stored

| What | Where |
|---|---|
| The app (portable) | Wherever you put the `.exe` |
| The app (installed) | `%LOCALAPPDATA%\Programs\MarkItDown GUI` |
| Settings | `MarkItDown GUI.settings.ron` next to the `.exe` |
| Private Python and MarkItDown | `MarkItDown GUI runtime\` next to the `.exe` |

If the `.exe` is in a folder you cannot write to (for example Program Files), settings and the runtime go to
`%LOCALAPPDATA%\MarkItDown GUI` instead.

To move the app to another PC without downloading again, copy the `.exe` together with the
`MarkItDown GUI runtime` folder. To remove it completely, delete the `.exe`, the settings file and the runtime folder.

## Troubleshooting

**Setup says it could not download Python.**
The PC needs internet access to python.org and pypi.org on first run. Check proxies or firewalls and press **Install** again.

**"A Windows runtime library is missing".**
Install the [Microsoft Visual C++ Redistributable (x64)](https://aka.ms/vs/17/release/vc_redist.x64.exe), then press **Install** again.

**I already have MarkItDown installed.**
The app finds it automatically on PATH, in pip's user Scripts folders, or via `python -m markitdown`.
You can also point **Options > markitdown command** at a specific `markitdown.exe` or `python.exe`.

**Images from Word files are missing.**
By default MarkItDown replaces images with placeholders. Turn on **Keep embedded images (data URIs)**.

## Build from source

Requirements: [Rust](https://rustup.rs) (stable).

```bat
git clone https://github.com/mohdbushnaq/markitdown-gui.git
cd markitdown-gui
build.bat
```

`build.bat` builds a release with the C runtime linked statically and writes `MarkItDown GUI.exe` next to it.
Every push to `main` is also built by GitHub Actions; the `.exe` is available under the run's artifacts.

### How it works

The app is written in Rust with [egui](https://github.com/emilk/egui). It does not contain any MarkItDown code.
It runs the `markitdown` command line in the background with no console window, one process per file,
and reads the result and any errors back into the interface.

| File | Purpose |
|---|---|
| `src/main.rs` | User interface, settings and conversion queue |
| `src/runner.rs` | Finding and running MarkItDown, parallel workers, cancellation |
| `src/update.rs` | Release checks, private Python setup, pip updates and app self-update |
| `src/install.rs` | Optional per-user install, shortcuts, Windows uninstall entry |

## Credits

- Conversion by [Microsoft MarkItDown](https://github.com/microsoft/markitdown) (MIT License).
- Interface built with [egui / eframe](https://github.com/emilk/egui) and [rfd](https://github.com/PolyMeilex/rfd).

Created by **[Mohammed Bushnaq](https://github.com/mohdbushnaq)**.

## License

[MIT](LICENSE) © 2026 Mohammed Bushnaq

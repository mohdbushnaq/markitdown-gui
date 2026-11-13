## What's new in 1.2.1

- **Fixed: web pages converting to an empty file.** Many sites build their content with JavaScript, or block automated downloads. When a page comes back empty or blocked, the app now loads it in a hidden Microsoft Edge (built into Windows; Chrome also works), waits for it to finish, and converts the fully loaded page. Pages that still have no text (for example behind a sign-in) now show an error instead of saving an empty file.

## New in 1.2.0

- **Install on this PC.** Portable is still the default, but **Options > App > Install on this PC** now installs the app for your Windows user: Start menu (and optional desktop) shortcut, an entry in **Settings > Apps > Installed apps** with a normal uninstaller, and your MarkItDown runtime is moved along so nothing downloads again. No admin rights needed.
- **App updates.** **Options > Updates > Check for updates** now also checks for new versions of MarkItDown GUI itself. **Update app** downloads the new version, swaps it in and restarts, for portable and installed copies. A green **App x.y.z available** button appears in the title bar when there is one.
- The Updates section moved to the top of Options, with separate lines for the app and for the MarkItDown converter.

## Also in 1.1.x

- Convert web pages by URL (field under the drop zone, or at the top of the list).
- Fixed: Ctrl+V paste in text fields, and links in the app now open in your browser.

## Download

Download **`MarkItDown-GUI.exe`** below and double-click it. No installer needed, no admin rights, no Python needed.
On first launch the app sets up its own private Python and MarkItDown automatically (a few minutes, about 500 MB, once).
On 1.2.0? Use **Options > Updates > Update app**. On an older version, replace the `.exe`; your settings and runtime are kept.

> Windows may show a SmartScreen warning because the app is not code-signed. Click **More info**, then **Run anyway**.

## Requirements

64-bit Windows 10 or 11, and internet access on first run.

See the [README](https://github.com/mohdbushnaq/markitdown-gui#readme) for screenshots and full instructions.

![CNC Viewer with the Macro Compass backplot, variable viewer, and inline G-code feedback](docs/cnc-viewer.png)

# CNC Viewer

[NC Viewer](https://ncviewer.com/) from [Toolpath Labs, Inc.](https://toolpath.com/) is the inspiration for the 3D backplot, syntax highlighting, icon style, colours, and general layout.

I built CNC Viewer in Rust for Windows with GPT-6.2 Astra and Sol. It supports the [MachPro](https://www.mach-labs.com/product/machpro/) G-code dialect my router uses. I can inspect and validate programs on the desktop, and my AI agents can use the CLI to check the G-code they generate.

## Features

- Interactive 3D backplot.
- Cutter trails that show the width of a configured bit, including the volume swept by a vertical flat-end bit on ramps and plunges, with optional depth cues.
- FANUC-style Macro B variables, expressions, loops, branches, and subprograms.
- Execution slider that follows repeated blocks.
- Variable viewer that updates as execution progresses.
- Feed-timed playback defaults to 8× estimated machining speed.
- Inline G-code errors and warnings.
- Background replotting after edits.
- Tabbed editor with split views.
- Disk sync with overwrite protection.
- An agent CLI for XYZ movement reports, projected cutter overlaps, stock sections, and labelled PNG views.

## Get started

Download `CNC-Viewer-windows-x64.exe` from [Releases](https://github.com/heetbeet/CNCViewer/releases), or build locally with `./tools/build.ps1` using Rust MSVC and the Windows SDK.

To read the complete agent guide, run this from Command Prompt in the download folder.

```cmd
CNC-Viewer-windows-x64.exe --help agents
```

After an unexpected exit, restored tabs open with plotting paused. Press Ctrl+Enter to plot the active tab. Use `--no-restore` to start with a blank tab and keep the saved session for later.

Contributions are welcome, especially for items in [Next work](#next-work). [Icon artwork credits](crates/cnc-app/assets/NOTICE.md).

## Next work

- Linux and macOS support.
- Selectable machine config files with custom G- and M-code mappings.
- Lua macros and additional G-code dialects.
- Per-tool cutter diameter editing and cutter-radius compensation (G41/G42) in the backplot.

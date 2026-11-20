# CNC Viewer 2.0

CNC Viewer is a native Windows G-code editor, simulator, and inspection tool. It includes an interactive backplot, feed-timed playback, and local GPU PNG export.

Build with Rust/MSVC and the Windows SDK:

```powershell
./tools/build.ps1
```

The editor freezes the current plot when typing starts. It replots after 450 ms of idle time for job sources up to 128 KiB, after 900 ms up to 1 MiB, and after 1.5 seconds above that. Ctrl+Enter plots immediately. Page Up and Page Down move through the focused editor. Ctrl+F and Ctrl+H open Find and Replace.

Pink rows show errors beside the affected G-code. Hover over a row when its message is clipped, or widen the editor to read it inline. The Diagnostics list jumps to the affected file and line. An execution error stops the plot at that block and leaves the preceding toolpath visible. The list also includes syntax errors the interpreter found in blocks it did not reach.

Amber rows warn about variables read before assignment on the executed path. Arithmetic treats unset values as zero where applicable, and a coordinate word with an unset value is omitted. Explicit comparisons with `#0` and `#3100` check for an unset value and do not raise the warning.

For example, `G2 X20 R4` from X0 on the XY plane reports “Arc radius R is too small to reach the endpoint.” Its diameter is 8 and the endpoints are 20 units apart. Other messages name the specific problem, such as a missing G55 work offset, an unloaded subprogram, or division by zero.

Disk changes reload clean sources while the window is focused. If the editor has unsaved changes, choose Reload or Overwrite beside the filename, then Apply sync. F5 reloads from disk. When there are unsaved changes, press it twice within 900 ms to discard them. Ctrl+S and the toolbar Save button confirm before overwriting a changed disk copy. Enter activates the default Yes button. Ctrl+Shift+S opens Save As.

Double-click `CNC Viewer.cmd` or `output/desktop/CNC Viewer.exe`. Drop G-code files onto the window or press Ctrl+O to open them as tabs. Ctrl+Enter plots the active tab. Space plays or pauses when the editor is unfocused. The mouse pans with the left button, orbits with the right button and zooms at the pointer with the wheel. Click a face, edge or corner of the view cube to set the angle. Variables expand beside the code. Right-click the Editor header to load a local machine setup JSON or add subprograms to the active tab. The toolbar uses [Font Awesome artwork](crates/cnc-app/assets/NOTICE.md).

The full-width tab strip sits directly under the green toolbar. Each tab contains its own editor, plot, camera, playback position, and view settings. A dot on the tab marks unsaved changes. Double-click the blank part of the strip to create a tab. Middle-click a tab or press Ctrl+W to close it. Drag to the other half of the viewer to split the window, or outside the window to move the tab to a separate window. The tab context menu offers both moves and Copy Filepath for saved files. A moved tab carries its unsaved text, subprograms, and machine setup.

The session log is `%LOCALAPPDATA%\CNCViewerNative\session.json`. A fresh launch without a file, after all windows have closed, restores up to seven file-backed tabs in one pane and selects the most recently opened or edited one on the right. Explicitly closed tabs and missing files are skipped. A launch while another window is open starts independently. The toolbar clock opens File history and samples. Sample stays above the scrollable history, which shows opening dates in YYYY-MM-DD format. Background checks gray out confirmed missing files in place and remove them from future visits to the menu.

```powershell
& './output/desktop/CNC Viewer.exe' --help | Out-String
& './output/desktop/CNC Viewer.exe' --help agents | Out-String
& './output/desktop/CNC Viewer.exe' inspect examples/goto-slots.nc | Out-String
& './output/desktop/CNC Viewer.exe' inspect examples/goto-slots.nc examples/inspection.json | Out-String
```

The executable prints the complete agent guide from its embedded Markdown with `--help agents`. Edit the source guide in [skills/cnc-viewer/AGENTS.md](skills/cnc-viewer/AGENTS.md) and rebuild to update it. Run `skills/cnc-viewer/install.ps1` to install a short Codex skill that reads the guide from the installed executable. Use `export-skill DIRECTORY` to extract the Markdown and optional Python helpers. Numeric inspections avoid graphics initialization. PNG views use the same native GPU renderer as the desktop app.

Verification:

```powershell
cargo test --locked --workspace --release
cargo clippy --locked --workspace -- -D warnings
cargo fmt --all --check
python tests/native_release.py
```

The test suite covers interpreter conformance, arcs in all three planes, the Macro Compass sample, macro motion, desktop input, GPU precision and cutter trail blending. The packaged release test exercises an isolated copy with an empty PATH. It covers numerical jobs, matrices, checkpoints, traces, motion, stock sections, labelled PNGs and the exported Python helpers.

The agent entry points are the CLI, Python helper and Rust crate. SpaceMouse packet decoding is covered by tests. Physical hardware validation is pending.

Supply machine setup values for fixture positions, probes, signals and tool geometry. G76 and G87 require machine-specific sequences, G88 requires a manual retract trajectory, and nonzero cutter compensation and dedicated machine cycles stop at the valid prefix.

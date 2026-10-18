# wezfont

**English** | [中文](README.zh-CN.md)

A font picker TUI for [WezTerm](https://wezterm.org).

Pick a main font and manage the fallback chain, with **live preview**: the choice is
written to `~/.wezterm-font.lua`, then the mtime of `~/.wezterm.lua` is bumped so
WezTerm reloads and the sample text re-renders in the new font immediately.

## Usage

```bash
wezfont              # open the TUI
wezfont --info       # detected paths, font count, current font and chain
wezfont --list       # list available font families
wezfont --set "Consolas"
wezfont --fallback "Microsoft YaHei,Cascadia Code"
wezfont --scan       # rescan system fonts and refresh the cache
```

### Keys

| Key | Action |
| --- | --- |
| `Tab` | switch between main font / fallback chain |
| `↑↓` or type | move / fuzzy-filter |
| `Enter` | apply the main font, or add/remove a fallback |
| `Shift+J` / `Shift+K` | reorder the selected fallback |
| `r` | rescan fonts |
| `Esc` | clear the filter, or quit |

## What gets written

`~/.wezterm-font.lua`:

```lua
return {
  font = "0xProto Nerd Font Mono",
  fallback = { "Microsoft YaHei", "Cascadia Code" },
}
```

Your `~/.wezterm.lua` reads that file and builds a `wezterm.font_with_fallback` chain.
Delete the file to fall back to the font configured by hand.

## Install

```bash
wezext install wezfont
# or, from source:
wezext install --build wezfont
```

## WSL

Fonts are always **Windows-side** (the WezTerm GUI renders on Windows), so running
`wezfont` from a WSL shell works fine — the Windows home directory is resolved
automatically.

## Build

```bash
cargo build --release
# -> target/release/wezfont.exe
```

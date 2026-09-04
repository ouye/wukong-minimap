# Black Myth: Wukong - Built-in Real-time Map (1.0.20+ branch)

![alt text](./docs/banner.png)

> **This is a fork of [jaskang/wukong-minimap](https://github.com/jaskang/wukong-minimap).**
>
> All of the core work — the injection, the rendering, the coordinate math, and the several
> hundred hand-collected map points — is [@jaskang](https://github.com/jaskang)'s.
> **Please go star the original repository first.**
>
> This branch does one thing: **make it work again on game versions after the
> 1.0.20 update of October 2025**, plus two optional extras: a heading-up map
> mode and a record of where you have walked. The minimap's gameplay and visual
> design are untouched.
>
> The full record of what changed is in [CHANGES.md](./CHANGES.md).

- Download: [releases](https://github.com/Ouye/wukong-minimap/releases)
- Original project: [jaskang/wukong-minimap](https://github.com/jaskang/wukong-minimap) · [BiliBili demo video](https://www.bilibili.com/video/BV1Y1KueREho/) · [Nexusmods](https://www.nexusmods.com/blackmythwukong/mods/1172)

Switch language: [中文](README.md)

## What this branch changes

The 1.0.20 game update (2025-10-13) triggered two **independent** failures:

**1. The plugin did not work at all.** Relinking the executable invalidated every SDK
offset. The SDK has been regenerated, and the global addresses are now resolved by
**runtime signature scanning** instead of being baked in at compile time, which should
survive future minor game updates much better.

**2. The minimap texture rendered as garbage.** This one has nothing to do with the
plugin's own code — the new game build's D3D12 environment exposed a long-standing
problem in the rendering library. A two-way controlled experiment pinned the trigger
down to **the texture being fully opaque (alpha == 255)**, and it is now worked around.

The trail of where you have walked is also recorded now, per map area, and drawn on
both the minimap and the big map.

Along the way: four general bugs fixed in the rendering library, resident memory cut
from 368 MB to 16 MB, and the install package from 96 MB to 23 MB.

## Changelog

- v2.0 (this fork)
  - Works on game versions after 1.0.20
  - Fixed the garbled minimap texture
  - New heading-up map mode: `Shift` + `0`
  - The walked trail is now recorded and drawn: `9` / `Shift` + `9`
  - Settings (window size, scale, orientation, trail) persist across restarts
  - The trail colour can be set in the config file
  - On-screen messages use a Chinese system font when one is available
  - The key bindings are listed alongside the big map
  - Maps are loaded on demand: 368 MB → 16 MB resident
  - Maps resampled to 2000×2000: 96 MB → 23 MB install size
- v1.7 (upstream)
  - UI adjustments, many map points added
- v1.6 (upstream)
  - Fixed AMD GPU rendering issues, added map points

## Key bindings

Press `Tab` to open the big map and the same list appears down its right-hand side,
so you do not have to come back here for it.

- `+` Zoom in the minimap window
- `-` Zoom out the minimap window
- `Shift` + `+` Zoom in the minimap scale
- `Shift` + `-` Zoom out the minimap scale
- `0` Show / hide the map
- `Shift` + `0` Toggle the map orientation mode **(new in this fork)**
  - Default: the map is fixed, the arrow turns with the character
  - Toggled: the arrow is locked pointing up, the map turns with the character
- `9` Toggle the walked trail **(new in this fork)** — off means neither drawn nor recorded
- `Shift` + `9` Clear the whole trail **(new in this fork)** — not undoable; press a second time within 3 seconds to confirm

These settings — window size, scale, orientation and the trail toggle — are kept in
`wukong_minimap_config.json` next to the dll and restored on the next launch. Delete
that file to go back to the defaults.

That file also holds the trail colour, which is edit-by-hand only:

```json
"trail_color": "#22E0FFCC"
```

`#RRGGBB` or `#RRGGBBAA` (the last two digits are opacity). Restart the game to apply.
A malformed value falls back to the built-in cyan and says so in the log.

The trail is recorded per map area and kept in `wukong_minimap_trails.json` next
to the dll, reloaded on the next launch. Copy that file to back it up, delete it
to wipe the trail by hand. It is written every 30 seconds and whenever you change
area, so a crash costs at most the last 30 seconds of trail.

## Demo screenshots

![alt text](./docs/demo0.png)
![alt text](./docs/demo1.png)
![alt text](./docs/demo2.png)

## Installation

Extract `wukong-minimap.zip` directly into the `b1\Binaries\Win64` folder under Black
Myth's installation directory (for Steam, right-click Black Myth -> Manage -> Browse
Local Files).

![alt text](./docs/install0.png)

This plugin includes the following files:

- `wukong_minimap.dll` — the plugin itself
- `dwmapi.dll` — the loader; proxies the system DLL to load `wukong_minimap.dll`
- `maps` — the map folder

If you have another way to load `wukong_minimap.dll`, you can skip `dwmapi.dll` entirely.

## For UE4SS users

UE4SS ships its own `dwmapi.dll` which intercepts the same system APIs and stops the
plugin from loading. Use the `dwmapi.dll` from wukong-minimap instead.

## Uninstallation

Delete `wukong_minimap.dll`.

## Building from source

You need Windows + MSVC (with the C++ toolset) + CMake + Rust (`x86_64-pc-windows-msvc`).

```powershell
# build, install into the game folder, and package
.\build.ps1 -Package -Install "D:\Games\Steam\steamapps\common\BlackMythWukong\b1\Binaries\Win64"
```

`check_offsets.ps1` checks whether the signatures still match the current executable
without launching the game — **run this first if the plugin breaks after a game
update**. It will tell you whether a rebuild is enough or the SDK has to be dumped again.

## Troubleshooting

The plugin writes `wukong_minimap.log` next to the dll. For verbose output, set
`RUST_LOG=debug` before launching the game. Please attach that file when reporting a
problem.

## Licence and credits

This project inherits upstream's **Apache License 2.0**. The original copyright belongs
to [@jaskang](https://github.com/jaskang).

- [jaskang/wukong-minimap](https://github.com/jaskang/wukong-minimap) — the original project (Apache-2.0)
- [hudhook](https://github.com/veeenu/hudhook) — injection and rendering framework (MIT, Andrea Venuta); a modified vendored copy is included in this repository
- [imgui](https://github.com/ocornut/imgui)
- [Dumper-7](https://github.com/Encryqed/Dumper-7) — generates the UE SDK

The map assets under `maps/` come from the upstream repository; this fork only
resampled them.

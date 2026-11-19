# TF2 Sound Studio

A small desktop tool for editing Team Fortress 2 sound waves: **mute**, **amplify** and **replace**
any of the ~16,000 `.wav`/`.mp3` entries in `tf2_sound_misc` and `tf2_sound_vo_english`.

Written in Rust with [egui](https://github.com/emilk/egui). Single self-contained executable.

![TF2 Sound Studio](docs/screenshot.png)

## Why edit the VPK instead of a custom folder

`sv_pure` servers ignore `tf/custom` soundscripts and loose `.wav` overrides, and the server's own
precache table fixes the wave path per sound (visible in `console.log` as `)items/cart_rolling.wav`).
The only thing that survives is the byte content behind that exact path. This tool rewrites the
VPK entry **in place, keeping the original length**, so the archive tree stays valid and the file
is served as-is.

## Features

- Lists every sound in both sound packs (15,958 entries) with instant search
- Explorer-style selection: any click anywhere on a row selects it (and clicking the
  same row again deselects), Ctrl/Alt+click toggles, Shift+click selects the range from
  the last anchor even after scrolling, Ctrl+A selects everything visible, Ctrl+D clears
- Amplify 0.1x - 4.0x with live dB readout; samples are clamped, so the wave keeps
  its exact byte length
- Mute (zeroes the PCM payload, header preserved)
- Replace with any `.wav`: the file is validated on pick (16-bit PCM only, format and
  duration shown), then resampled to the target rate, channel-converted and fitted to
  the original slot
- Restore selected / restore all from a backup that lives next to the game
- Filters: All / Modified / Original, modified rows marked `M`
- Refuses to write while `tf_win64.exe` is running (the VPKs are locked)
- No console window, speaker icon embedded in the exe and the window

## Build

Requires Rust (stable) and the MSVC toolchain on Windows.

```
cargo build --release
# -> target/release/tf2-sound-studio.exe
```

`build.rs` embeds `icon.ico` into the executable (`winres`); if the resource compiler is missing the
build still succeeds with a warning.

## Usage

1. Close Team Fortress 2.
2. Run `tf2 sound studio.exe`. The game folder is auto-detected from the Steam registry and
   `libraryfolders.vdf`; paste a path and press `Load` if detection fails.
3. Search, select rows, then amplify / mute / replace / restore.
   - Amplify: set the slider, press `Apply gain`.
   - Mute: `Silence selected`.
   - Replace: hit `Browse`, pick a 16-bit PCM `.wav`, check the format line
     (`Hz | channels | bit | seconds`), then `Replace selected`. The source is resampled
     and padded/trimmed to fit the target slot exactly, so the VPK entry stays the same size.
   - Undo: `Restore selected`, or `Restore all` for every file the tool has touched.
4. Backups are written to `<tf>\sound_studio_backup.bin`. Editing is reversible until you delete it.
   Original waves produced by the earlier Python helpers (`cart_silence_backup.bin`,
   `footstep_gain_backup.bin`) are imported automatically on first launch, so those files are
   restorable too.

`--verify` prints pack entry counts and wav metadata without touching anything.


## Safety behaviour

- Backups are written to disk **before** the game file is modified, so an interrupted run
  never leaves an un-backed-up wave.
- All edits start from the stored original, so pressing `Apply gain` twice does not stack
  (1.5x twice is still 1.5x, not 2.25x).
- Changing the search text or filter clears the selection, so hidden rows cannot be edited.
- Amplify and Silence work on 8/16-bit PCM waves and on MP3. MP3 is edited by rewriting the
  per-granule `global_gain`, so loudness changes while the file keeps its exact byte length and
  every other field (and decode timing) stays identical.
- Replacing an MP3 is not possible: the entry length cannot change inside a VPK. Float and
  24/32-bit waves are reported as skipped and are not marked modified.
- Large batches are processed a few files per frame, so the window stays responsive and shows
  `working 120/786`.
- `Clear sound cache` deletes the game's `*.sound.cache` files. TF2 resolves wave names through
  them, and a cache older than your edit keeps playing the old audio even when the VPK bytes are
  already silent. The game rebuilds them on the next launch.
- Muting 8-bit PCM writes 0x80 (the unsigned midpoint), not 0x00, so it is silence instead of
  a click.
- Replacement audio keeps its own channels: stereo stays stereo, mono is duplicated, stereo into
  mono is mixed down.
- Every write is read back and compared; a failed write is counted as a failure, not "ok".
- Corrupt VPK directories, damaged backup files and unreadable replacement WAVs are reported in
  the log instead of crashing.

## Issues and feature requests

Bugs and ideas: <https://github.com/Repurify/tf2-sound-studio/issues>

Describe what is broken or what you want added. For bugs, say how to reproduce it.

## A sound still plays after you silence it

That is almost always the game's sound cache, not the edit. TF2 resolves wave names through
`*.sound.cache` files (`tf/tf2_sound_misc.vpk.sound.cache`, `tf/sound/sound.cache`, and copies
under `download`, `sound_workshop` and each `custom` add-on). If one of those is older than your
edit, the client keeps playing the old audio even though the bytes in the VPK are already silent.

1. Close Team Fortress 2 completely (the VPK and the caches are locked while it runs).
2. Press `Clear sound cache` in the app.
3. Start TF2 again. The caches are rebuilt on launch.

If it still plays, the sound is a different entry: check for `_start` / `_stop` siblings of the loop
you muted, and for random one-shots a soundscape can trigger (`ambient_mp3/dog1-6`, `hawk`, `owl`,
`crow` are common ones). `snd_show` will not list those, because they come from the soundscape
layer rather than the engine's sound scheduler.

## Limitations

- Steam *verify integrity* or a game update restores original waves; re-apply afterwards.
- Sound range is server-authoritative. Changing sample amplitude or the soundscript's
  `soundlevel` does not extend how far a sound is networked to you; distant sounds are culled
  server-side.
- Client-side only. Nothing here changes hit registration, damage or movement.

## License

MIT

# ascii-cam

Real-time ASCII camera for the terminal, written in Rust.

`ascii-cam` is a desktop-focused Rust rewrite inspired by the MIT-licensed
[`terminalcam`](https://gitlab.com/here_forawhile/terminalcam) project. It keeps
the terminal camera workflow and uses a new Rust-native `.ascicam` recording
format instead of the original `.tcam` format.

## Features

- Live webcam feed rendered as ASCII in the terminal
- macOS and Linux capture through `ffmpeg`
- Color modes: 24-bit, 256-color, 16-color, gray, green, green gradient, red,
  red gradient, and off
- Interactive controls for contrast, brightness, invert, rotation, presets,
  help, and settings
- `.ascicam` recording and playback with keyframes, deltas, skip frames, and
  zlib compression
- HTML screenshots plus single-frame `.ascicam` snapshots
- Terminal-only UI with no GUI dependency

Android/Termux support is intentionally out of scope.

## Requirements

- Rust 1.95+
- `ffmpeg`
- A terminal with ANSI color support

On macOS, camera permission must be granted to the terminal app that launches
`ascii-cam`.

## Install

```bash
cargo install --path .
```

## Usage

```bash
# Live camera, default medium resolution and 24-bit color
cargo run --release

# Plain ASCII
cargo run --release -- --no-color

# Lower capture resolution
cargo run --release -- --resolution low

# Use a specific camera index
cargo run --release -- --camera 1

# Record while viewing
cargo run --release -- --record session.ascicam

# Play back a recording
cargo run --release -- --play session.ascicam
```

## Controls

| Key | Action |
|---|---|
| `1` | Toggle invert |
| `2` | Cycle rotation |
| `3` | Start or stop `.ascicam` recording |
| `4` | Save HTML screenshot and `.ascicam` snapshot |
| `5` | Cycle preset |
| Up / Down | Adjust contrast |
| Left / Right | Adjust brightness |
| `s` | Toggle settings |
| `h` | Toggle help |
| `q` | Quit |

## Capture Backends

`ascii-cam` currently shells out to `ffmpeg`:

- macOS: `avfoundation`
- Linux: `v4l2`

This keeps camera support practical and avoids platform-specific camera API
bindings in the first Rust version.

## Recording Format

Recordings use `.ascicam`, a Rust-owned binary format:

- `ACAM` magic header
- fixed dimensions and FPS in the header
- keyframes for full rendered frames
- delta frames for changed cells
- skip frames for identical frames
- optional zlib compression

The format is not compatible with terminalcam `.tcam` files by design.

## License

MIT. See [LICENSE](LICENSE).


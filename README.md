<div align="center">
  <h1>ascii-cam</h1>

  <p><strong>Real-time ASCII camera for your terminal — fast, colorful, and written in Rust.</strong></p>

  <p>
    <a href="https://github.com/cesarferreira/ascii-cam"><img alt="Rust" src="https://img.shields.io/badge/rust-1.95%2B-orange"></a>
    <a href="https://github.com/cesarferreira/ascii-cam/releases"><img alt="Release" src="https://img.shields.io/github/v/release/cesarferreira/ascii-cam?color=blue"></a>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
  </p>

  <p>
    <a href="#install">Install</a>
    &nbsp;·&nbsp;
    <a href="#quickstart">Quickstart</a>
    &nbsp;·&nbsp;
    <a href="#controls">Controls</a>
    &nbsp;·&nbsp;
    <a href="#recording">Recording</a>
  </p>

  <br>

  <!-- Screenshot placeholder: add assets/screenshot.png when a capture is ready. -->
  <p><em>Screenshot space reserved.</em></p>

  <br>
</div>

---

## Why ascii-cam

Webcams are usually trapped in GUI apps. **ascii-cam** turns a live camera feed
into terminal-native ASCII art with color, recording, screenshots, and keyboard
controls.

- **Terminal-first.** Live camera output renders directly in your shell with no GUI window.
- **Colorful when you want it.** Use 24-bit color, 256-color, 16-color, gray,
  green, red, or plain ASCII.
- **Record what you see.** Save and replay terminal sessions with the native
  `.ascicam` format.
- **Fast enough to be fun.** Rust handles frame conversion, ASCII mapping,
  terminal drawing, screenshots, and playback.
- **Desktop-focused.** macOS and Linux are supported through `ffmpeg`; Android
  and Termux are intentionally out of scope.

`ascii-cam` is inspired by the MIT-licensed
[`terminalcam`](https://gitlab.com/here_forawhile/terminalcam) project. It uses
a new Rust-native `.ascicam` recording format instead of terminalcam's `.tcam`
format.

<a id="install"></a>
## Install

Build from source:

```bash
cargo install --path . --locked
```

Or use the Makefile:

```bash
make install
```

Requirements:

- Rust 1.95+
- `ffmpeg`
- A terminal with ANSI color support

On macOS, grant camera permission to the terminal app that launches
`ascii-cam`.

<a id="quickstart"></a>
## Quickstart

Start the live camera:

```bash
ascii-cam
```

Use a lower capture resolution:

```bash
ascii-cam --resolution low
```

Render plain ASCII without color:

```bash
ascii-cam --no-color
```

Pick a different camera index:

```bash
ascii-cam --camera 1
```

Record while viewing:

```bash
ascii-cam --record session.ascicam
```

Play back a recording:

```bash
ascii-cam --play session.ascicam
```

From a checkout, replace `ascii-cam` with `cargo run --release --`.

<a id="controls"></a>
## Controls

All controls are available during live view:

| Key | Action |
|---|---|
| `1` | Toggle invert |
| `2` | Cycle rotation |
| `3` | Start or stop `.ascicam` recording |
| `4` | Save an HTML screenshot and `.ascicam` snapshot |
| `5` | Cycle preset |
| Up / Down | Adjust contrast |
| Left / Right | Adjust brightness |
| `s` | Toggle settings |
| `h` | Toggle help |
| `q` | Quit |

## Color Modes

`ascii-cam` can render with:

- 24-bit truecolor
- 256-color ANSI
- 16-color ANSI
- grayscale ANSI ramp
- fixed green or red
- brightness-mapped green or red
- plain ASCII with color disabled

## Capture

`ascii-cam` shells out to `ffmpeg` for camera capture:

- macOS: `avfoundation`
- Linux: `v4l2`

That keeps camera support practical without binding directly to each platform's
camera API. If `ffmpeg` exits early, `ascii-cam` reports the captured ffmpeg
stderr so camera permissions, bad indexes, or unsupported resolutions are easier
to diagnose.

<a id="recording"></a>
## Recording

Recordings use `.ascicam`, a Rust-owned binary format:

- `ACAM` magic header
- fixed dimensions and FPS in the header
- keyframes for full rendered frames
- delta frames for changed cells
- skip frames for identical frames
- optional zlib compression

The format is not compatible with terminalcam `.tcam` files by design.

## Development

Common targets:

```bash
make check
make test
make lint
make run ARGS="--resolution low --no-color"
```

Run the full local verification flow:

```bash
cargo fmt --all
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## License

MIT. See [LICENSE](LICENSE).

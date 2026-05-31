use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use futures_util::stream::{self, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use crate::app::{Cli, RampChoice, ServeArgs};
use crate::capture::{
    FfmpegCapture, Platform, ensure_supported_platform, resolve_capture_dimensions,
};
use crate::color::ColorMode;
use crate::render::{
    RAMP_LONG, RAMP_SHORT, RenderConfig, RenderedFrame, compute_render_size, render_frame,
};

const BROADCAST_CAPACITY: usize = 8;

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<Arc<String>>,
    token: Option<String>,
    clear_each_frame: bool,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BroadcastOutcome {
    Continue,
    Disconnect,
    End,
}

pub fn frame_payload(rendered: &RenderedFrame, clear: bool) -> String {
    let mut out = String::new();
    if clear {
        out.push_str("\x1b[2J");
    }
    out.push_str("\x1b[H");
    out.push_str(&rendered.terminal_text());
    out
}

pub fn negotiate_color_mode(accept: Option<&str>, bits_query: Option<&str>) -> ColorMode {
    if let Some(bits) = bits_query.and_then(parse_bits) {
        return color_mode_for_bits(bits);
    }
    let Some(accept) = accept else {
        return ColorMode::TrueColor;
    };
    for part in accept.split(',') {
        let part = part.trim();
        if let Some(bits) = part
            .split(';')
            .find_map(|param| param.trim().strip_prefix("bits="))
            .and_then(parse_bits)
        {
            return color_mode_for_bits(bits);
        }
    }
    ColorMode::TrueColor
}

pub fn authorized(required: Option<&str>, provided: Option<&str>) -> bool {
    match required {
        None => true,
        Some(expected) => provided == Some(expected),
    }
}

pub fn effective_bind(args: &ServeArgs) -> &str {
    if args.local { "127.0.0.1" } else { &args.bind }
}

pub fn stream_query_suffix(token: Option<&str>) -> String {
    token
        .map(|value| format!("?token={value}"))
        .unwrap_or_default()
}

pub fn broadcast_outcome(error: broadcast::error::RecvError) -> BroadcastOutcome {
    match error {
        broadcast::error::RecvError::Lagged(_) => BroadcastOutcome::Disconnect,
        broadcast::error::RecvError::Closed => BroadcastOutcome::End,
    }
}

pub fn run(cli: &Cli, args: ServeArgs) -> Result<()> {
    let platform = ensure_supported_platform(cli.platform)?;
    let camera_index = cli.camera.unwrap_or(0);
    let color_mode = if cli.no_color {
        ColorMode::Off
    } else {
        cli.color
    };
    let (tx, _rx) = broadcast::channel::<Arc<String>>(BROADCAST_CAPACITY);
    let state = AppState {
        tx: tx.clone(),
        token: args.token.clone(),
        clear_each_frame: args.clear_each_frame,
    };

    spawn_capture_thread(
        cli,
        args.cols,
        args.rows,
        platform,
        camera_index,
        color_mode,
        tx,
    );

    let bind = effective_bind(&args);
    let addr: SocketAddr = format!("{bind}:{}", args.port)
        .parse()
        .with_context(|| format!("invalid bind address {bind}:{}", args.port))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;

    runtime.block_on(async {
        let app = Router::new()
            .route("/", get(index_handler))
            .route("/stream", get(stream_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("bind HTTP server to {addr}"))?;
        print_consumer_hints(bind, args.port, args.token.as_deref());
        axum::serve(listener, app)
            .await
            .context("run HTTP server")?;
        Ok::<(), anyhow::Error>(())
    })
}

fn spawn_capture_thread(
    cli: &Cli,
    out_cols: usize,
    out_rows: usize,
    platform: Platform,
    camera_index: u32,
    color_mode: ColorMode,
    tx: broadcast::Sender<Arc<String>>,
) {
    let cli = cli.clone();
    thread::spawn(move || {
        if let Err(error) = capture_loop(
            &cli,
            out_cols,
            out_rows,
            platform,
            camera_index,
            color_mode,
            &tx,
        ) {
            eprintln!("capture thread stopped: {error:#}");
        }
    });
}

fn capture_loop(
    cli: &Cli,
    out_cols: usize,
    out_rows: usize,
    platform: Platform,
    camera_index: u32,
    color_mode: ColorMode,
    tx: &broadcast::Sender<Arc<String>>,
) -> Result<()> {
    let (cam_w, cam_h) = cli.resolution.dimensions();
    let (cam_w, cam_h) = resolve_capture_dimensions(platform, camera_index, cam_w, cam_h);
    let mut capture = FfmpegCapture::spawn(platform, camera_index, cli.fps, cam_w, cam_h)?;
    let ramp = match cli.ramp {
        RampChoice::Long => RAMP_LONG.to_string(),
        RampChoice::Short => RAMP_SHORT.to_string(),
    };
    let rotation = cli.rotate % 4;
    let char_aspect = cli.char_aspect;

    loop {
        let frame = capture.read_frame()?.rotate(rotation);
        let (cols, rows) = compute_render_size(
            out_cols.max(1),
            out_rows.max(1),
            frame.width,
            frame.height,
            char_aspect,
        );
        let config = RenderConfig {
            cols,
            rows,
            ramp: ramp.clone(),
            color_mode,
            contrast: cli.contrast,
            brightness: cli.brightness,
            invert: cli.invert,
        };
        let rendered = render_frame(&frame, &config);
        let payload = Arc::new(frame_payload(&rendered, false));
        let _ = tx.send(payload);
    }
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    token: Option<String>,
    bits: Option<String>,
}

async fn index_handler(
    State(state): State<AppState>,
    Query(query): Query<StreamQuery>,
) -> Response {
    if !authorized(state.token.as_deref(), query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let token_query = stream_query_suffix(state.token.as_deref());
    Html(viewer_html(&token_query)).into_response()
}

async fn stream_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Response {
    if !authorized(state.token.as_deref(), query.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let _ = negotiate_color_mode(
        headers.get("accept").and_then(|value| value.to_str().ok()),
        query.bits.as_deref(),
    );
    let rx = state.tx.subscribe();
    let clear_each_frame = state.clear_each_frame;
    let preamble: Vec<Result<Bytes, std::convert::Infallible>> = if clear_each_frame {
        vec![Ok(Bytes::from_static(b"\x1b[2J"))]
    } else {
        Vec::new()
    };
    let preamble = stream::iter(preamble);
    let frames =
        BroadcastStream::new(rx).take_while(|result| futures_util::future::ready(result.is_ok()));
    let frames = frames.filter_map(|result| async move {
        result
            .ok()
            .map(|frame| Ok(Bytes::from(frame.as_ref().clone())))
    });
    let body = Body::from_stream(preamble.chain(frames));
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; charset=utf-8")
        .body(body)
        .unwrap()
}

fn print_consumer_hints(bind: &str, port: u16, token: Option<&str>) {
    let query = stream_query_suffix(token);
    let network_wide = bind == "0.0.0.0" || bind == "::" || bind == "[::]";
    eprintln!();
    eprintln!("ascii-cam serve is running (Ctrl+C to stop)");
    if network_wide && token.is_none() {
        eprintln!("  warning: no --token set; anyone on your network can watch");
    }
    eprintln!();
    let hosts = consumer_hosts(bind, port);
    for host in &hosts {
        eprintln!("  Browser:  http://{host}{query}");
        eprintln!("  Terminal: curl -N \"http://{host}/stream{query}\"");
        eprintln!();
    }
}

fn consumer_hosts(bind: &str, port: u16) -> Vec<String> {
    let mut hosts = Vec::new();
    if bind == "0.0.0.0" || bind == "::" {
        hosts.push(format!("127.0.0.1:{port}"));
        if let Some(ip) = guess_outbound_ip() {
            hosts.push(format!("{ip}:{port}"));
        }
    } else {
        hosts.push(format!("{bind}:{port}"));
    }
    hosts
}

fn guess_outbound_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() {
        return None;
    }
    Some(ip.to_string())
}

pub fn viewer_html(token_query: &str) -> String {
    format!(
        r#"<!doctype html>
<meta charset="utf-8">
<title>ascii-cam</title>
<style>
  body {{ margin: 0; background: #050505; color: #eee; }}
  pre {{ font: 12px/1 monospace; white-space: pre; margin: 16px; }}
</style>
<pre id="screen"></pre>
<script>
const pre = document.getElementById('screen');
const url = '/stream{token_query}';
const HOME = '\x1b[H';

const PALETTE_16 = [
  [0, 0, 0], [205, 0, 0], [0, 205, 0], [205, 205, 0],
  [0, 0, 238], [205, 0, 205], [0, 205, 205], [229, 229, 229],
  [127, 127, 127], [255, 0, 0], [0, 255, 0], [255, 255, 0],
  [92, 92, 255], [255, 0, 255], [0, 255, 255], [255, 255, 255]
];

function xterm256(n) {{
  if (n < 16) return PALETTE_16[n];
  if (n >= 232) {{ const v = 8 + (n - 232) * 10; return [v, v, v]; }}
  n -= 16;
  const conv = (c) => (c === 0 ? 0 : 55 + c * 40);
  return [conv(Math.floor(n / 36)), conv(Math.floor((n % 36) / 6)), conv(n % 6)];
}}

function colorFromSgr(codeStr) {{
  const parts = codeStr.split(';').map((p) => parseInt(p, 10) || 0);
  const first = parts[0];
  if (codeStr === '' || first === 0) return null;
  if (first === 38 && parts[1] === 2) return [parts[2], parts[3], parts[4]];
  if (first === 38 && parts[1] === 5) return xterm256(parts[2]);
  if (first >= 30 && first <= 37) return PALETTE_16[first - 30];
  if (first >= 90 && first <= 97) return PALETTE_16[first - 90 + 8];
  return undefined;
}}

function sameColor(a, b) {{
  if (a === null && b === null) return true;
  if (a === null || b === null) return false;
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2];
}}

function renderFrame(frame) {{
  frame = frame.replace(/\x1b\[2J/g, '');
  const fragment = document.createDocumentFragment();
  let cur = null;
  let seg = '';
  let last = 0;
  const flush = () => {{
    if (!seg) return;
    if (cur) {{
      const span = document.createElement('span');
      span.style.color = 'rgb(' + cur[0] + ',' + cur[1] + ',' + cur[2] + ')';
      span.textContent = seg;
      fragment.appendChild(span);
    }} else {{
      fragment.appendChild(document.createTextNode(seg));
    }}
    seg = '';
  }};
  const re = /\x1b\[([0-9;]*)m/g;
  for (const m of frame.matchAll(re)) {{
    seg += frame.slice(last, m.index);
    last = m.index + m[0].length;
    const next = colorFromSgr(m[1]);
    if (next === undefined || sameColor(next, cur)) continue;
    flush();
    cur = next;
  }}
  seg += frame.slice(last);
  flush();
  pre.replaceChildren(fragment);
}}

fetch(url).then(async (response) => {{
  if (!response.ok) throw new Error('stream failed: ' + response.status);
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  while (true) {{
    const {{ value, done }} = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, {{ stream: true }});
    const last = buffer.lastIndexOf(HOME);
    const prev = last > 0 ? buffer.lastIndexOf(HOME, last - 1) : -1;
    if (prev >= 0) {{
      renderFrame(buffer.slice(prev + HOME.length, last));
      buffer = buffer.slice(last);
    }}
  }}
}}).catch((err) => {{ pre.textContent = String(err); }});
</script>
"#
    )
}

fn parse_bits(value: &str) -> Option<u8> {
    value.trim().parse().ok()
}

fn color_mode_for_bits(bits: u8) -> ColorMode {
    match bits {
        0 => ColorMode::Off,
        8 => ColorMode::Ansi256,
        16 => ColorMode::Ansi16,
        _ => ColorMode::TrueColor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ServeArgs;
    use crate::render::RenderedFrame;

    #[test]
    fn frame_payload_prefixes_cursor_home_and_includes_terminal_text() {
        let frame = RenderedFrame::new(2, 1, vec!["ab".to_string()], None).unwrap();
        let payload = frame_payload(&frame, false);
        assert!(payload.starts_with("\x1b[H"));
        assert!(payload.contains("ab"));
    }

    #[test]
    fn frame_payload_can_clear_screen_before_cursor_home() {
        let frame = RenderedFrame::new(2, 1, vec!["ab".to_string()], None).unwrap();
        let payload = frame_payload(&frame, true);
        assert!(payload.starts_with("\x1b[2J\x1b[H"));
    }

    #[test]
    fn negotiate_color_mode_reads_accept_bits_parameter() {
        assert_eq!(
            negotiate_color_mode(Some("text/plain; bits=8"), None),
            ColorMode::Ansi256
        );
        assert_eq!(
            negotiate_color_mode(Some("text/plain; bits=16"), None),
            ColorMode::Ansi16
        );
        assert_eq!(negotiate_color_mode(None, Some("8")), ColorMode::Ansi256);
    }

    #[test]
    fn authorized_allows_missing_token_when_not_required() {
        assert!(authorized(None, None));
        assert!(authorized(None, Some("secret")));
    }

    #[test]
    fn authorized_requires_matching_token_when_configured() {
        assert!(authorized(Some("secret"), Some("secret")));
        assert!(!authorized(Some("secret"), Some("wrong")));
        assert!(!authorized(Some("secret"), None));
    }

    #[test]
    fn effective_bind_uses_loopback_when_local_flag_is_set() {
        let args = ServeArgs {
            port: 8080,
            bind: "0.0.0.0".to_string(),
            local: true,
            token: None,
            cols: 120,
            rows: 40,
            clear_each_frame: false,
        };
        assert_eq!(effective_bind(&args), "127.0.0.1");
    }

    #[test]
    fn stream_query_suffix_includes_token_when_configured() {
        assert_eq!(stream_query_suffix(Some("secret")), "?token=secret");
        assert_eq!(stream_query_suffix(None), "");
    }

    #[test]
    fn broadcast_outcome_disconnects_on_lagged_and_ends_on_closed() {
        assert_eq!(
            broadcast_outcome(broadcast::error::RecvError::Lagged(1)),
            BroadcastOutcome::Disconnect
        );
        assert_eq!(
            broadcast_outcome(broadcast::error::RecvError::Closed),
            BroadcastOutcome::End
        );
    }
}

use ascii_cam::app::ServeArgs;
use ascii_cam::color::ColorMode;
use ascii_cam::render::RenderedFrame;
use ascii_cam::serve::{
    BroadcastOutcome, authorized, broadcast_outcome, effective_bind, frame_payload,
    negotiate_color_mode, stream_query_suffix,
};
use tokio::sync::broadcast;

#[test]
fn frame_payload_uses_cursor_home_and_terminal_text_for_curl_clients() {
    let frame = RenderedFrame::new(3, 2, vec!["abc".to_string(), "def".to_string()], None).unwrap();
    let payload = frame_payload(&frame, false);

    assert!(payload.starts_with("\x1b[H"));
    assert!(payload.contains("abc"));
    assert!(payload.contains("def"));
    assert!(!payload.starts_with("\x1b[2J"));
}

#[test]
fn frame_payload_can_emit_full_clear_before_each_frame_when_requested() {
    let frame = RenderedFrame::new(2, 1, vec!["ok".to_string()], None).unwrap();
    let payload = frame_payload(&frame, true);

    assert!(payload.starts_with("\x1b[2J\x1b[H"));
}

#[test]
fn negotiate_color_mode_honors_accept_header_and_bits_query() {
    assert_eq!(
        negotiate_color_mode(Some("text/plain; bits=8"), None),
        ColorMode::Ansi256
    );
    assert_eq!(
        negotiate_color_mode(Some("text/plain; bits=16"), None),
        ColorMode::Ansi16
    );
    assert_eq!(negotiate_color_mode(None, Some("0")), ColorMode::Off);
    assert_eq!(
        negotiate_color_mode(Some("text/plain"), Some("24")),
        ColorMode::TrueColor
    );
}

#[test]
fn authorized_enforces_token_when_configured_and_allows_open_access_otherwise() {
    assert!(authorized(None, None));
    assert!(authorized(Some("secret"), Some("secret")));
    assert!(!authorized(Some("secret"), Some("wrong")));
    assert!(!authorized(Some("secret"), None));
}

#[test]
fn effective_bind_defaults_to_all_interfaces_unless_local_flag_is_set() {
    let lan = ServeArgs {
        port: 8080,
        bind: "0.0.0.0".to_string(),
        local: false,
        token: None,
        cols: 120,
        rows: 40,
        clear_each_frame: false,
    };
    let local = ServeArgs {
        local: true,
        ..lan.clone()
    };
    assert_eq!(effective_bind(&lan), "0.0.0.0");
    assert_eq!(effective_bind(&local), "127.0.0.1");
}

#[test]
fn stream_query_suffix_formats_token_for_urls() {
    assert_eq!(stream_query_suffix(Some("mytoken")), "?token=mytoken");
    assert_eq!(stream_query_suffix(None), "");
}

#[test]
fn broadcast_outcome_disconnects_lagged_clients_instead_of_buffering_forever() {
    assert_eq!(
        broadcast_outcome(broadcast::error::RecvError::Lagged(3)),
        BroadcastOutcome::Disconnect
    );
    assert_eq!(
        broadcast_outcome(broadcast::error::RecvError::Closed),
        BroadcastOutcome::End
    );
}

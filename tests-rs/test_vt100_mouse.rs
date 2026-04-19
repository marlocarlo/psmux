/// Test that vt100 properly tracks mouse protocol mode/encoding
/// when child processes send DECSET escape sequences.

#[test]
fn test_vt100_mouse_mode_detection() {
    let mut parser = vt100::Parser::new(24, 80, 0);

    // Initial state: no mouse mode
    let mode = parser.screen().mouse_protocol_mode();
    let enc = parser.screen().mouse_protocol_encoding();
    println!("Initial mode: {:?}, encoding: {:?}", mode, enc);
    assert_eq!(mode, vt100::MouseProtocolMode::None);
    assert_eq!(enc, vt100::MouseProtocolEncoding::Default);

    // Simulate crossterm's EnableMouseCapture which sends:
    // \x1b[?1000h - X11 mouse reporting (Press)
    // \x1b[?1002h - Cell motion mouse tracking (ButtonMotion)
    // \x1b[?1003h - All motion tracking (AnyMotion)
    // \x1b[?1006h - SGR mouse encoding
    parser.process(b"\x1b[?1000h");
    let mode = parser.screen().mouse_protocol_mode();
    println!("After ?1000h: mode={:?}", mode);
    assert_eq!(mode, vt100::MouseProtocolMode::PressRelease);

    parser.process(b"\x1b[?1002h");
    let mode = parser.screen().mouse_protocol_mode();
    println!("After ?1002h: mode={:?}", mode);
    assert_eq!(mode, vt100::MouseProtocolMode::ButtonMotion);

    parser.process(b"\x1b[?1003h");
    let mode = parser.screen().mouse_protocol_mode();
    println!("After ?1003h: mode={:?}", mode);
    assert_eq!(mode, vt100::MouseProtocolMode::AnyMotion);

    parser.process(b"\x1b[?1006h");
    let enc = parser.screen().mouse_protocol_encoding();
    println!("After ?1006h: encoding={:?}", enc);
    assert_eq!(enc, vt100::MouseProtocolEncoding::Sgr);

    // Simulate DisableMouseCapture:
    // \x1b[?1006l \x1b[?1003l \x1b[?1002l \x1b[?1000l
    parser.process(b"\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l");
    let mode = parser.screen().mouse_protocol_mode();
    let enc = parser.screen().mouse_protocol_encoding();
    println!("After disable: mode={:?}, encoding={:?}", mode, enc);
    assert_eq!(mode, vt100::MouseProtocolMode::None);
    assert_eq!(enc, vt100::MouseProtocolEncoding::Default);

    println!("\nAll vt100 mouse mode tests passed!");
}

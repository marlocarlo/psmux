use super::*;

#[test]
fn timed_out_control_handler_completes_with_explicit_error() {
    let (_handler_tx, handler_rx) = mpsc::channel::<String>();
    let (response_tx, response_rx) = mpsc::channel();

    forward_control_response(
        "capture-pane",
        handler_rx,
        &response_tx,
        Duration::from_millis(1),
    );

    assert_eq!(
        response_rx.recv().unwrap(),
        ControlCommandResponse::error("capture-pane timed out")
    );
}

#[test]
fn disconnected_control_handler_completes_with_explicit_error() {
    let (handler_tx, handler_rx) = mpsc::channel::<String>();
    drop(handler_tx);
    let (response_tx, response_rx) = mpsc::channel();

    forward_control_response(
        "display-message",
        handler_rx,
        &response_tx,
        Duration::from_secs(1),
    );

    assert_eq!(
        response_rx.recv().unwrap(),
        ControlCommandResponse::error("display-message response channel disconnected")
    );
}

#[test]
fn command_block_has_one_atomic_begin_and_footer() {
    let block = format_control_command_block(
        true,
        "capture-pane -p",
        123,
        7,
        Some(Ok(ControlCommandResponse::success("pane text"))),
    );

    assert_eq!(block.matches("%begin 123 7 1").count(), 1);
    assert_eq!(block.matches("%end 123 7 1").count(), 1);
    assert_eq!(block.matches("%error").count(), 0);
    assert_eq!(
        block,
        "capture-pane -p\n%begin 123 7 1\npane text\n%end 123 7 1\n"
    );
}

#[test]
fn command_error_block_has_one_error_footer() {
    let block = format_control_command_block(
        false,
        "capture-pane -p",
        123,
        8,
        Some(Ok(ControlCommandResponse::error("capture-pane timed out"))),
    );

    assert_eq!(block.matches("%begin 123 8 1").count(), 1);
    assert_eq!(block.matches("%error 123 8 1").count(), 1);
    assert_eq!(block.matches("%end").count(), 0);
    assert_eq!(
        block,
        "%begin 123 8 1\ncapture-pane timed out\n%error 123 8 1\n"
    );
}

#[test]
fn successful_output_cannot_be_reinterpreted_as_an_error() {
    let body = "\u{0001}ERR\u{0001}literal pane output";
    let block = format_control_command_block(
        false,
        "capture-pane -p",
        123,
        9,
        Some(Ok(ControlCommandResponse::success(body))),
    );

    assert!(block.contains(body));
    assert!(block.contains("%end 123 9 1"));
    assert!(!block.contains("%error 123 9 1"));
}

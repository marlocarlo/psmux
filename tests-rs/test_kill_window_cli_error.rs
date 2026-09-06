use std::fs;
use std::io;
use std::net::TcpListener;
use std::process::Command;

#[test]
fn missing_target_fails_before_cli_dispatch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock control port");
    let port = listener.local_addr().unwrap().port();
    let profile = std::env::temp_dir()
        .join(format!(
            "psmux_kill_window_cli_error_{}_{port}",
            std::process::id()
        ));
    let psmux_dir = profile.join(".psmux");
    fs::create_dir_all(&psmux_dir).unwrap();
    fs::write(psmux_dir.join("probe.port"), port.to_string()).unwrap();
    fs::write(psmux_dir.join("probe.key"), "test-key").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_psmux"))
        .args(["killw", "-t"])
        .env("USERPROFILE", &profile)
        .env("PSMUX_TARGET_SESSION", "probe")
        .env_remove("PSMUX_TARGET_FULL")
        .env_remove("TMUX")
        .output()
        .expect("run psmux CLI");

    listener.set_nonblocking(true).unwrap();
    let no_connection = matches!(
        listener.accept(),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock
    );
    fs::remove_dir_all(&profile).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "psmux: -t expects an argument\n"
    );
    assert!(no_connection, "invalid command must not reach the server");
}

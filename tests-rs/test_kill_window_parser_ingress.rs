use super::*;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, RwLock};
use std::thread;

fn start_connection(
    aliases: HashMap<String, String>,
) -> (TcpStream, mpsc::Receiver<CtrlReq>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    let aliases = Arc::new(RwLock::new(aliases));
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_connection(stream, request_tx, "test-key", aliases);
    });
    let stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    (stream, request_rx, handle)
}

fn read_line(reader: &mut BufReader<TcpStream>) -> String {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    line.trim_end().to_string()
}

fn authenticate(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>) {
    stream.write_all(b"AUTH test-key\n").unwrap();
    stream.flush().unwrap();
    assert_eq!(read_line(reader), "OK");
}

fn assert_no_requests(requests: &mpsc::Receiver<CtrlReq>) {
    assert!(matches!(
        requests.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}

fn assert_no_control_command_dispatch(requests: &mpsc::Receiver<CtrlReq>) {
    let mut saw_registration = false;
    for request in requests.try_iter() {
        match request {
            CtrlReq::ControlRegister { .. } => saw_registration = true,
            CtrlReq::KillWindow
            | CtrlReq::KillWindowTarget { .. }
            | CtrlReq::BindKey(..)
            | CtrlReq::ConfirmBefore(..)
            | CtrlReq::SetHook(..)
            | CtrlReq::AppendHook(..) => panic!("invalid command was dispatched"),
            _ => {}
        }
    }
    assert!(saw_registration);
}

#[test]
fn simple_connection_rejects_missing_target_without_dispatch() {
    let (mut stream, requests, handle) = start_connection(HashMap::new());
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    authenticate(&mut stream, &mut reader);

    stream.write_all(b"kill-window -t\n").unwrap();
    stream.flush().unwrap();

    assert_eq!(read_line(&mut reader), "psmux: -t expects an argument");
    stream.shutdown(Shutdown::Both).unwrap();
    handle.join().unwrap();
    assert_no_requests(&requests);
}

#[test]
fn command_alias_rejects_missing_target_without_dispatch() {
    let aliases = HashMap::from([("close".to_string(), "kill-window".to_string())]);
    let (mut stream, requests, handle) = start_connection(aliases);
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    authenticate(&mut stream, &mut reader);

    stream.write_all(b"close -t\n").unwrap();
    stream.flush().unwrap();

    assert_eq!(read_line(&mut reader), "psmux: -t expects an argument");
    stream.shutdown(Shutdown::Both).unwrap();
    handle.join().unwrap();
    assert_no_requests(&requests);
}

#[test]
fn simple_connection_rejects_invalid_deferred_commands() {
    for command in [
        "bind-key x kill-window -t\n",
        "confirm-before kill-window -t\n",
        "confirm-before 'kill-window -t'\n",
        "set-hook pane-died kill-window -t\n",
    ] {
        let (mut stream, requests, handle) = start_connection(HashMap::new());
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        authenticate(&mut stream, &mut reader);

        stream.write_all(command.as_bytes()).unwrap();
        stream.flush().unwrap();

        assert_eq!(read_line(&mut reader), "psmux: -t expects an argument");
        stream.shutdown(Shutdown::Both).unwrap();
        handle.join().unwrap();
        assert_no_requests(&requests);
    }
}

#[test]
fn command_target_overrides_transport_target_after_alias_expansion() {
    let aliases = HashMap::from([(
        "close".to_string(),
        "kill-window -t :1".to_string(),
    )]);
    let (mut stream, requests, handle) = start_connection(aliases);
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    authenticate(&mut stream, &mut reader);

    stream
        .write_all(b"TARGET :0\nclose -t:2\n")
        .unwrap();
    stream.flush().unwrap();

    let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    let CtrlReq::KillWindowTarget {
        win,
        win_is_id,
        name,
        resp,
    } = request
    else {
        panic!("expected targeted kill-window request");
    };
    assert_eq!(win, Some(2));
    assert!(!win_is_id);
    assert_eq!(name, None);
    resp.send(Ok(())).unwrap();

    stream.shutdown(Shutdown::Both).unwrap();
    handle.join().unwrap();
}

#[test]
fn transport_target_is_used_when_command_has_no_target() {
    let (mut stream, requests, handle) = start_connection(HashMap::new());
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    authenticate(&mut stream, &mut reader);

    stream.write_all(b"TARGET :3\nkill-window\n").unwrap();
    stream.flush().unwrap();

    let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    let CtrlReq::KillWindowTarget {
        win,
        win_is_id,
        name,
        resp,
    } = request
    else {
        panic!("expected targeted kill-window request");
    };
    assert_eq!(win, Some(3));
    assert!(!win_is_id);
    assert_eq!(name, None);
    resp.send(Ok(())).unwrap();

    stream.shutdown(Shutdown::Both).unwrap();
    handle.join().unwrap();
}

#[test]
fn bare_command_target_does_not_discard_the_transport_window() {
    for target in ["2", "work.2"] {
        let (mut stream, requests, handle) = start_connection(HashMap::new());
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        authenticate(&mut stream, &mut reader);

        stream
            .write_all(format!("TARGET work:1\nkill-window -t {target}\n").as_bytes())
            .unwrap();
        stream.flush().unwrap();

        let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        let CtrlReq::KillWindowTarget {
            win,
            win_is_id,
            name,
            resp,
        } = request
        else {
            panic!("expected targeted kill-window request");
        };
        assert_eq!(win, Some(1));
        assert!(!win_is_id);
        assert_eq!(name, None);
        resp.send(Ok(())).unwrap();

        stream.shutdown(Shutdown::Both).unwrap();
        handle.join().unwrap();
    }
}

#[test]
fn valid_deferred_command_is_still_dispatched() {
    let (mut stream, requests, handle) = start_connection(HashMap::new());
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    authenticate(&mut stream, &mut reader);

    stream
        .write_all(b"bind-key x kill-window -t :1\n")
        .unwrap();
    stream.flush().unwrap();

    let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
    let CtrlReq::BindKey(table, key, command, repeat) = request else {
        panic!("expected bind-key request");
    };
    assert_eq!(table, "prefix");
    assert_eq!(key, "x");
    assert_eq!(command, "kill-window -t :1");
    assert!(!repeat);

    stream.shutdown(Shutdown::Both).unwrap();
    handle.join().unwrap();
}

#[test]
fn control_connection_reports_missing_target_and_stays_usable() {
    let (mut stream, requests, handle) = start_connection(HashMap::new());
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    authenticate(&mut stream, &mut reader);

    stream.write_all(b"CONTROL_NOECHO\n").unwrap();
    stream.flush().unwrap();
    assert!(read_line(&mut reader).starts_with("\u{1b}P1000p%begin "));
    assert!(read_line(&mut reader).starts_with("%end "));

    for command in [
        "killw -t\n",
        "bind-key x kill-window -t\n",
        "confirm-before kill-window -t\n",
        "set-hook pane-died kill-window -t\n",
    ] {
        stream.write_all(command.as_bytes()).unwrap();
        stream.flush().unwrap();
        assert!(read_line(&mut reader).starts_with("%begin "));
        assert_eq!(read_line(&mut reader), "psmux: -t expects an argument");
        assert!(read_line(&mut reader).starts_with("%error "));
    }

    stream.shutdown(Shutdown::Both).unwrap();
    handle.join().unwrap();
    assert_no_control_command_dispatch(&requests);
}

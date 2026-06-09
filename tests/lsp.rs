//! Black-box handshake test for `wirebug lsp`: initialize over stdio,
//! assert the advertised capabilities, shut down cleanly.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

fn read_message(reader: &mut BufReader<ChildStdout>) -> serde_json::Value {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read header line");
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(len) = line.strip_prefix("Content-Length: ") {
            content_length = Some(len.parse::<usize>().expect("content length"));
        }
    }
    let mut body = vec![0u8; content_length.expect("Content-Length header")];
    reader.read_exact(&mut body).expect("read body");
    serde_json::from_slice(&body).expect("valid JSON body")
}

fn send(stdin: &mut ChildStdin, body: &str) {
    stdin.write_all(&frame(body)).expect("write message");
    stdin.flush().expect("flush");
}

fn spawn_server() -> (Child, ChildStdin, BufReader<ChildStdout>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wirebug"))
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn wirebug lsp");
    let stdin = child.stdin.take().expect("stdin");
    let stdout = BufReader::new(child.stdout.take().expect("stdout"));
    (child, stdin, stdout)
}

#[test]
fn initialize_shutdown_round_trip() {
    let (mut child, mut stdin, mut stdout) = spawn_server();

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
    );
    let response = read_message(&mut stdout);
    assert_eq!(response["id"], 1);
    let caps = &response["result"]["capabilities"];
    assert_eq!(caps["textDocumentSync"], 1, "full sync: {caps}");
    let triggers = caps["completionProvider"]["triggerCharacters"]
        .as_array()
        .expect("trigger characters");
    assert!(triggers.iter().any(|t| t == "."), "dot trigger: {caps}");
    assert!(triggers.iter().any(|t| t == ":"), "colon trigger: {caps}");

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    );
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#,
    );
    let response = read_message(&mut stdout);
    assert_eq!(response["id"], 2);

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"exit","params":null}"#,
    );
    let status = child.wait().expect("server exits");
    assert!(status.success(), "clean exit, got {status:?}");
}

/// Opening a document with an error must produce a publishDiagnostics
/// notification carrying a `wirebug::` code — the full pipeline runs
/// against the buffer text, not the disk.
#[test]
fn did_open_publishes_diagnostics() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("wirebug.toml"),
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    )
    .expect("write manifest");
    let main = dir.path().join("main.wb");
    // On disk the file is fine; the opened buffer wires a missing port.
    std::fs::write(&main, "component Root { pub port p \"P\"; }\n").expect("write main.wb");
    let main = main.canonicalize().expect("canonical path");

    let (mut child, mut stdin, mut stdout) = spawn_server();
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
    );
    read_message(&mut stdout);
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    );

    let text = "component Root { pub port p \\\"P\\\"; wire red 1 [p, ghost.x]; }\\n";
    let open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file://{}","languageId":"wirebug","version":1,"text":"{}"}}}}}}"#,
        main.display(),
        text,
    );
    send(&mut stdin, &open);

    let note = read_message(&mut stdout);
    assert_eq!(note["method"], "textDocument/publishDiagnostics");
    let diagnostics = note["params"]["diagnostics"]
        .as_array()
        .expect("diagnostics array");
    assert!(
        diagnostics.iter().any(|d| d["code"]
            .as_str()
            .is_some_and(|c| c.starts_with("wirebug::"))),
        "expected a wirebug diagnostic from the buffer text: {note}"
    );

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#,
    );
    // Drain until the shutdown response; more diagnostics may interleave.
    loop {
        let message = read_message(&mut stdout);
        if message["id"] == 2 {
            break;
        }
    }
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"exit","params":null}"#,
    );
    assert!(child.wait().expect("server exits").success());
}

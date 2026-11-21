//! Black-box tests for `wirebug lsp` over stdio: the initialize handshake
//! and its advertised capabilities, published diagnostics, hover, and
//! formatting.

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
    assert_eq!(caps["hoverProvider"], true, "hover: {caps}");

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

/// A reserved word used as a name reaches the editor as its own diagnostic,
/// ranged on the word, like any other parse problem.
#[test]
fn reserved_word_diagnostic_points_at_the_word() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("wirebug.toml"),
        "[project]\nname = \"t\"\nversion = \"0.0.0\"\n",
    )
    .expect("write manifest");
    let main = dir.path().join("main.wb");
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

    // `size` is a contextual keyword and a fine name; `cable` is reserved.
    let text = "component Root { pub port size \\\"S\\\"; pub port cable \\\"C\\\"; }\\n";
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
    let [diagnostic] = diagnostics.as_slice() else {
        panic!("expected one diagnostic: {note}");
    };
    assert_eq!(diagnostic["code"], "wirebug::reserved_word");
    // The help line rides on the message, so the editor shows it too.
    let message = diagnostic["message"].as_str().expect("message");
    assert!(
        message.starts_with("'cable' is a reserved word\n") && message.contains("rename it"),
        "{message}"
    );
    let range = &diagnostic["range"];
    assert_eq!(range["start"]["line"], 0);
    // `component Root { pub port size "S"; pub port ` is 45 characters.
    assert_eq!(range["start"]["character"], 45);
    assert_eq!(range["end"]["character"], 50);

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#,
    );
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

/// Hovering a reference in an opened fixture file describes what it names:
/// an inline connector's port with its label, pin and declaring line, and
/// an instance with its label and type.
#[test]
fn hover_describes_the_reference_under_the_cursor() {
    let main = std::path::Path::new("tests/fixtures/inline_project/main.wb")
        .canonicalize()
        .expect("fixture path");
    let text = std::fs::read_to_string(&main).expect("read fixture");
    let uri = format!("file://{}", main.display());

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
    let open = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {"textDocument": {
            "uri": uri, "languageId": "wirebug", "version": 1, "text": text,
        }},
    });
    send(&mut stdin, &open.to_string());

    // The position `column` characters into the first line holding `needle`.
    let position = |needle: &str, column: usize| {
        let (line, src) = text
            .lines()
            .enumerate()
            .find(|(_, src)| src.contains(needle))
            .expect("needle in fixture");
        let character = src.find(needle).expect("needle") + column;
        serde_json::json!({"line": line, "character": character})
    };
    let mut hover = |id: u32, position: serde_json::Value| {
        let request = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/hover",
            "params": {"textDocument": {"uri": uri}, "position": position},
        });
        send(&mut stdin, &request.to_string());
        // Diagnostics for the opened file may arrive first.
        loop {
            let message = read_message(&mut stdout);
            if message["id"] == id {
                return message["result"]["contents"].clone();
            }
        }
    };

    let port = hover(2, position("[ecu.a, ic_pedal.apps1]", 18));
    assert_eq!(port["kind"], "markdown", "{port}");
    let value = port["value"].as_str().expect("markdown value");
    assert!(value.starts_with("**port** `ic_pedal.apps1`"), "{value}");
    assert!(value.contains("- label: \"APPS1\""), "{value}");
    assert!(value.contains("- pin: 1"), "{value}");
    assert!(value.contains("- defined in `main.wb`, line 29"), "{value}");

    let instance = hover(3, position("[ecu.a, ic_pedal.apps1]", 2));
    let value = instance["value"].as_str().expect("markdown value");
    assert!(value.starts_with("**instance** `ecu`"), "{value}");
    assert!(value.contains("- label: \"ECU\""), "{value}");
    assert!(value.contains("- type: `Device`"), "{value}");

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":4,"method":"shutdown","params":null}"#,
    );
    loop {
        let message = read_message(&mut stdout);
        if message["id"] == 4 {
            break;
        }
    }
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"exit","params":null}"#,
    );
    assert!(child.wait().expect("server exits").success());
}

/// Formatting an opened buffer replaces it whole with what `wirebug fmt`
/// writes; a buffer that does not parse gets no edits.
#[test]
fn formatting_lays_out_the_buffer_and_leaves_a_broken_one_alone() {
    let lamp = std::path::Path::new("tests/fixtures/unformatted_project/components/lamp.wb")
        .canonicalize()
        .expect("fixture path");
    let text = std::fs::read_to_string(&lamp).expect("read fixture");
    let uri = format!("file://{}", lamp.display());

    let (mut child, mut stdin, mut stdout) = spawn_server();
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
    );
    let response = read_message(&mut stdout);
    let caps = &response["result"]["capabilities"];
    assert_eq!(caps["documentFormattingProvider"], true, "{caps}");
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    );
    let open = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {"textDocument": {
            "uri": uri, "languageId": "wirebug", "version": 1, "text": text,
        }},
    });
    send(&mut stdin, &open.to_string());

    let format = |stdin: &mut ChildStdin, stdout: &mut BufReader<ChildStdout>, id: u32| {
        let request = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/formatting",
            "params": {
                "textDocument": {"uri": uri},
                "options": {"tabSize": 2, "insertSpaces": false},
            },
        });
        send(stdin, &request.to_string());
        // Diagnostics for the opened file may arrive first.
        loop {
            let message = read_message(stdout);
            if message["id"] == id {
                return message["result"].clone();
            }
        }
    };

    assert_eq!(
        format(&mut stdin, &mut stdout, 2),
        serde_json::json!([{
            "range": {
                "start": {"line": 0, "character": 0},
                "end": {"line": 1, "character": 0},
            },
            "newText": "component Lamp {\n    pub port anode   \"+\" pin 1;\n    pub port cathode \"-\" pin 2;\n}\n",
        }])
    );

    let change = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": "component Lamp { pub port anode \"+\" }\n"}],
        },
    });
    send(&mut stdin, &change.to_string());
    assert_eq!(format(&mut stdin, &mut stdout, 3), serde_json::json!([]));

    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":4,"method":"shutdown","params":null}"#,
    );
    loop {
        let message = read_message(&mut stdout);
        if message["id"] == 4 {
            break;
        }
    }
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"exit","params":null}"#,
    );
    assert!(child.wait().expect("server exits").success());
}

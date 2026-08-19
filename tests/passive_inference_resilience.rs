use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

use llamamanager::{
    passive_inference_metrics::poll_passive_inference_metrics, server_readiness::ServerEndpoint,
};

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn write_response(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

#[test]
fn transient_old_child_timeout_is_recovered_before_stale_fallback_is_needed() {
    // Keep the old port listening but never service it. The first /metrics connection therefore
    // hits a deterministic read timeout rather than a racy connection-refused result.
    let old_child = TcpListener::bind("127.0.0.1:0").unwrap();
    let old_child_port = old_child.local_addr().unwrap().port();

    let live_child = TcpListener::bind("127.0.0.1:0").unwrap();
    let live_child_port = live_child.local_addr().unwrap().port();
    let router = TcpListener::bind("127.0.0.1:0").unwrap();
    let router_port = router.local_addr().unwrap().port();

    let router_thread = thread::spawn(move || {
        for child_port in [old_child_port, live_child_port] {
            let (mut stream, _) = router.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.starts_with("GET /models HTTP/1.1"));
            let body = format!(
                "{{\"data\":[{{\"id\":\"Qwen3.8-27B\",\"status\":{{\"value\":\"loaded\",\"last_used\":99,\"args\":[\"llama-server\",\"--port\",\"{child_port}\",\"--spec-type\",\"draft-mtp\"]}}}}]}}"
            );
            write_response(&mut stream, "application/json", &body);
        }
    });

    let child_thread = thread::spawn(move || {
        let (mut stream, _) = live_child.accept().unwrap();
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET /metrics HTTP/1.1"));
        write_response(
            &mut stream,
            "text/plain",
            concat!(
                "llamacpp:requests_processing 1\n",
                "llamacpp:predicted_tokens_seconds 19.51\n",
                "llamacpp:draft_tokens_total 5797\n",
                "llamacpp:draft_tokens_accepted_total 4999\n",
            ),
        );
    });

    let snapshot = poll_passive_inference_metrics(
        &ServerEndpoint::loopback(router_port),
        Duration::from_millis(150),
    )
    .unwrap();

    router_thread.join().unwrap();
    child_thread.join().unwrap();
    drop(old_child);

    assert_eq!(
        snapshot.source_endpoint,
        format!("127.0.0.1:{live_child_port}")
    );
    assert_eq!(snapshot.decode_tps, Some(19.51));
    assert_eq!(snapshot.speculative_draft_tokens_total, Some(5797.0));
    assert_eq!(snapshot.speculative_accepted_tokens_total, Some(4999.0));
    assert!(snapshot.is_mtp());
}

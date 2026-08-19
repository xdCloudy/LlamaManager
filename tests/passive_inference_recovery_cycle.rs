use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

use llamamanager::{
    passive_inference_metrics::{
        PassiveInferenceMetricsError, poll_passive_inference_metrics,
    },
    server_readiness::ServerEndpoint,
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

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

#[test]
fn a_full_failed_poll_can_recover_fresh_on_the_next_cycle() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = thread::spawn(move || {
        let mut metrics_requests = 0_u8;

        while metrics_requests < 5 {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);

            if request.starts_with("GET /models HTTP/1.1") {
                // Make loopback router discovery fall back to the configured direct endpoint.
                write_response(&mut stream, "404 Not Found", "application/json", "{}");
                continue;
            }

            assert!(request.starts_with("GET /metrics HTTP/1.1"));
            metrics_requests += 1;

            if metrics_requests <= 4 {
                // End the response before HTTP headers complete. The first poll should exhaust all
                // four transient retry attempts and return an error to the UI, which can then keep
                // its prior value as STALE rather than looking live forever.
                continue;
            }

            write_response(
                &mut stream,
                "200 OK",
                "text/plain",
                concat!(
                    "llamacpp:prompt_tokens_seconds 80.25\n",
                    "llamacpp:predicted_tokens_seconds 6.75\n",
                    "llamacpp:requests_processing 0\n",
                ),
            );
        }
    });

    let endpoint = ServerEndpoint::loopback(port);

    let error = poll_passive_inference_metrics(&endpoint, Duration::from_millis(100)).unwrap_err();
    assert!(matches!(
        error,
        PassiveInferenceMetricsError::MissingHeaders
            | PassiveInferenceMetricsError::Io { .. }
            | PassiveInferenceMetricsError::Connect { .. }
    ));

    let recovered =
        poll_passive_inference_metrics(&endpoint, Duration::from_millis(100)).unwrap();
    assert_eq!(recovered.source_endpoint, format!("127.0.0.1:{port}"));
    assert_eq!(recovered.prompt_tps, Some(80.25));
    assert_eq!(recovered.decode_tps, Some(6.75));
    assert_eq!(recovered.requests_processing, Some(0.0));

    server.join().unwrap();
}

use clap::{load_yaml, App};
use std::fs;
use std::io::BufReader;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

const PERMITTED_DESTINATIONS: &[&str] = &["api.giphy.com:443"];

#[tokio::main]
async fn main() -> io::Result<()> {
    let proxy_options = parse_cli();
    let tls_config = build_tls_config(&proxy_options);

    let tls_acceptor = TlsAcceptor::from(tls_config.await);
    let listening_addr = format!("{}:{}", proxy_options.bind_address, proxy_options.bind_port);
    let listener = TcpListener::bind(listening_addr).await?;
    run_server(listener, tls_acceptor).await;
    Ok(())
}

fn parse_cli() -> ProxyOptions {
    let yaml = load_yaml!("cli.yml");
    let matches = App::from_yaml(yaml).get_matches();

    let bind_address = matches.value_of("bind-address").unwrap_or("127.0.0.1");
    let bind_port = matches.value_of("bind-port").unwrap_or("8080");
    let cert_path = matches.value_of("cert-path").unwrap_or("MyCertificate.crt");
    let key_path = matches.value_of("key-path").unwrap_or("MyKey.key");

    ProxyOptions {
        bind_address: bind_address.to_string(),
        bind_port: bind_port.to_string(),
        certs: load_certs(cert_path),
        private_key: load_private_key(key_path),
    }
}

struct ProxyOptions {
    bind_address: String,
    bind_port: String,
    certs: Vec<rustls::Certificate>,
    private_key: rustls::PrivateKey,
}

fn load_certs(filepath: &str) -> Vec<rustls::Certificate> {
    let certfile = fs::File::open(filepath).expect("cannot open certificate file");
    let mut reader = BufReader::new(certfile);
    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect()
}

fn load_private_key(filepath: &str) -> rustls::PrivateKey {
    let keyfile = fs::File::open(filepath).expect("cannot open private key file");
    let mut reader = BufReader::new(keyfile);

    loop {
        match rustls_pemfile::read_one(&mut reader).expect("cannot parse private key .pem file") {
            Some(rustls_pemfile::Item::RSAKey(key)) => return rustls::PrivateKey(key),
            Some(rustls_pemfile::Item::PKCS8Key(key)) => return rustls::PrivateKey(key),
            None => break,
            _ => {}
        }
    }

    panic!("no key found in {:?}", filepath);
}

async fn build_tls_config(proxy_options: &ProxyOptions) -> Arc<rustls::ServerConfig> {
    let config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(
            proxy_options.certs.clone(),
            proxy_options.private_key.clone(),
        )
        .expect("Unable to create TLS config");
    Arc::new(config)
}

async fn run_server(listener: TcpListener, tls_acceptor: TlsAcceptor) {
    loop {
        let (client_stream, _) = unwrap_or_continue!(listener.accept().await);

        let mut client_stream_tls =
            unwrap_or_continue!(establish_tls(client_stream, &tls_acceptor).await);

        let http_request_line =
            unwrap_or_continue!(read_http_request(&mut client_stream_tls).await);

        let rcvd_http_request = unwrap_or_continue!(parse_http_request(&http_request_line).await);

        if !is_http_connect(&rcvd_http_request).await {
            unwrap_or_continue!(send_unsupported_method_status(&mut client_stream_tls).await);
            continue;
        }

        if !is_permitted_destination(&rcvd_http_request.uri).await {
            unwrap_or_continue!(send_forbidden_status(&mut client_stream_tls).await);
            continue;
        }

        let mut dst_stream = unwrap_or_continue!(TcpStream::connect(rcvd_http_request.uri).await);

        unwrap_or_continue!(send_ok_status(&mut client_stream_tls).await);

        unwrap_or_continue!(client_stream_tls.flush().await);

        unwrap_or_continue!(io::copy_bidirectional(&mut client_stream_tls, &mut dst_stream).await);
    }
}

#[macro_export]
macro_rules! unwrap_or_continue {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(_) => continue,
        }
    };
}

async fn establish_tls(
    stream: TcpStream,
    acceptor: &TlsAcceptor,
) -> Result<TlsStream<TcpStream>, std::io::Error> {
    let acceptor = acceptor.clone();
    acceptor.accept(stream).await
}

async fn read_http_request(
    client_stream: &mut TlsStream<TcpStream>,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut buffer: [u8; 512] = [0; 512];
    client_stream.read(&mut buffer).await?;
    match std::str::from_utf8(&buffer) {
        Ok(s) => Ok(s.to_string()),
        Err(e) => Err(Box::new(e)),
    }
}

async fn parse_http_request(req: &str) -> Result<HttpRequest, Box<dyn std::error::Error>> {
    let req = req.split(' ').collect::<Vec<&str>>();
    let invalid_http_req_err = Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "Invalid HTTP request",
    ));
    let method = match req.get(0) {
        Some(m) => m.to_string(),
        None => return Err(invalid_http_req_err),
    };
    let uri = match req.get(1) {
        Some(u) => u.to_string(),
        None => return Err(invalid_http_req_err),
    };
    Ok(HttpRequest { method, uri })
}

#[derive(Debug, PartialEq)]
struct HttpRequest {
    method: String,
    uri: String,
}

async fn is_http_connect(req: &HttpRequest) -> bool {
    if req.method == "CONNECT" {
        return true;
    }
    false
}

async fn is_permitted_destination(url: &str) -> bool {
    PERMITTED_DESTINATIONS.contains(&url)
}

async fn send_unsupported_method_status(stream: &mut TlsStream<TcpStream>) -> io::Result<()> {
    let status_msg = create_http_status(405, "Method Not Allowed").await;
    send_http_status(stream, &status_msg).await?;
    Ok(())
}

async fn send_forbidden_status(stream: &mut TlsStream<TcpStream>) -> io::Result<()> {
    let status_msg = create_http_status(403, "Forbidden Yeah").await;
    send_http_status(stream, &status_msg).await?;
    Ok(())
}

async fn send_ok_status(stream: &mut TlsStream<TcpStream>) -> io::Result<()> {
    let status_msg = create_http_status(200, "OK").await;
    send_http_status(stream, &status_msg).await?;
    Ok(())
}

async fn send_http_status(stream: &mut TlsStream<TcpStream>, status_msg: &str) -> io::Result<()> {
    stream.write_all(status_msg.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

async fn create_http_status(status_code: u16, status_message: &str) -> String {
    format!("HTTP/1.1 {} {}\r\n\r\n", status_code, status_message)
}

#[cfg(test)]
mod tests {
    use crate::*;

    struct TestCase<I, E> {
        input: I,
        expected: E,
    }

    #[tokio::test]
    async fn test_is_permitted_destination() {
        let cases = vec![
            TestCase {
                input: "api.giphy.com:443",
                expected: true,
            },
            TestCase {
                input: "api.giphy.com:80",
                expected: false,
            },
            TestCase {
                input: "different.url.com:443",
                expected: false,
            },
        ];

        for c in cases {
            assert_eq!(is_permitted_destination(&c.input).await, c.expected);
        }
    }

    #[tokio::test]
    async fn test_is_http_connect() {
        let cases = vec![
            TestCase {
                input: HttpRequest {
                    method: "CONNECT".to_string(),
                    uri: "api.giphy.com:443".to_string(),
                },
                expected: true,
            },
            TestCase {
                input: HttpRequest {
                    method: "GET".to_string(),
                    uri: "api.giphy.com:443".to_string(),
                },
                expected: false,
            },
        ];
        for c in cases {
            assert_eq!(is_http_connect(&c.input).await, c.expected);
        }
    }

    #[tokio::test]
    async fn test_parse_http_request_connect_should_pass() {
        let line = "CONNECT example.com:443 HTTP/1.1\r\n\r\n";
        assert_eq!(
            parse_http_request(line).await.unwrap(),
            HttpRequest {
                method: "CONNECT".to_string(),
                uri: "example.com:443".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn test_parse_http_request_get_should_pass() {
        let line = "GET / HTTP/1.1\r\n\r\n";
        assert_eq!(
            parse_http_request(line).await.unwrap(),
            HttpRequest {
                method: "GET".to_string(),
                uri: "/".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn test_parse_http_request_invalid_input_should_pass() {
        let line = "This is some test text";
        assert_eq!(
            parse_http_request(line).await.unwrap(),
            HttpRequest {
                method: "This".to_string(),
                uri: "is".to_string(),
            }
        );
    }

    #[tokio::test]
    #[should_panic]
    async fn test_parse_http_request_invalid_input_should_panic() {
        let line = "ThisIsSomeTestText";
        parse_http_request(line).await.unwrap();
    }

    #[tokio::test]
    #[should_panic]
    async fn test_parse_http_request_no_input_should_panic() {
        let line = "";
        parse_http_request(line).await.unwrap();
    }

    #[test]
    fn test_unwrap_or_continue_unwraps_when_ok() {
        loop {
            let ok: Result<&str, &str> = Ok("ok");
            assert_eq!(unwrap_or_continue!(ok), "ok");
            break;
        }
    }

    #[test]
    fn test_unwrap_or_continue_continues_when_err() {
        for _ in [0] {
            let err: Result<&str, &str> = Err("err");
            unwrap_or_continue!(err);
            panic!(); // Should never reach
        }
    }
}

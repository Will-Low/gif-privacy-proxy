use clap::{load_yaml, App};
use std::fs;
use std::io::BufReader;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

#[tokio::main]
async fn main() -> io::Result<()> {
    let proxy_options = parse_cli();
    let tls_config = build_tls_config(&proxy_options);

    let tls_acceptor = TlsAcceptor::from(tls_config.await);
    let listening_addr = format!("{}:{}", proxy_options.bind_address, proxy_options.bind_port);
    let listener = TcpListener::bind(listening_addr).await.unwrap();

    loop {
        let (client_stream, _) = listener.accept().await.unwrap();
        let mut client_stream: TlsStream<TcpStream> =
            establish_tls(client_stream, &tls_acceptor).await;
        let http_request = parse_http_request(&mut client_stream).await;
        if !is_http_connect(&http_request).await {
            client_stream
                .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                .await
                .unwrap();
            client_stream.flush().await.unwrap();
            continue;
        }
        let mut dst_stream = TcpStream::connect(http_request.uri).await.unwrap();
        client_stream
            .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
            .await
            .unwrap();
        client_stream.flush().await.unwrap();
        io::copy_bidirectional(&mut client_stream, &mut dst_stream)
            .await
            .unwrap();
    }
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

    panic!(
        "no keys found in {:?} (encrypted keys not supported)",
        filepath
    );
}

async fn build_tls_config(proxy_options: &ProxyOptions) -> Arc<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(
            proxy_options.certs.clone(),
            proxy_options.private_key.clone(),
        )
        .expect("Unable to create TLS config");
    Arc::new(config)
}

async fn establish_tls(stream: TcpStream, acceptor: &TlsAcceptor) -> TlsStream<TcpStream> {
    let acceptor = acceptor.clone();
    acceptor
        .accept(stream)
        .await
        .expect("Unable to establish TLS with client")
}

async fn parse_http_request(client_stream: &mut TlsStream<TcpStream>) -> HttpRequest {
    let mut buffer: [u8; 512] = [0; 512];
    client_stream.read(&mut buffer).await.unwrap();
    let req = std::str::from_utf8(&buffer).unwrap();
    let req = req.split(' ').collect::<Vec<&str>>();
    let method = req[0].to_string();
    let uri = req[1].to_string();
    HttpRequest { method, uri }
}

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

/// Check both domain + path, in the event the domain has an endpoint with an open redirect
const PERMITTED_URLS: &'static [&str] = &["api.giphy.com/v1/gifs/search"];

fn is_permitted_url(url: &str) -> bool {
    PERMITTED_URLS.contains(&url)
}

fn send_unsupported_method_error(stream: &mut TlsStream<TcpStream>) -> io::Result<()> {
    stream
        .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
        .unwrap();
    stream.flush().unwrap();
}

#[cfg(test)]
mod tests {
    use crate::*;

    struct TestCase<I, E> {
        input: I,
        expected: E,
    }

    #[test]
    fn test_is_permitted_url() {
        let cases = vec![
            TestCase {
                input: "api.giphy.com/v1/gifs/search",
                expected: true,
            },
            TestCase {
                input: "api.giphy.com/v1/gifs/disallowed-endpoint",
                expected: false,
            },
            TestCase {
                input: "different.url.com/v1/gifs/search",
                expected: false,
            },
        ];

        for c in cases {
            assert_eq!(is_permitted_url(&c.input), c.expected);
        }
    }
}

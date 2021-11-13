use clap::{load_yaml, App};
use std::fs;
use std::io::BufReader;
use std::sync::Arc;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls;
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

// TODO - Remove
const PERMITTED_DESTINATIONS: &[&str] = &["api.giphy.com:443", "api.giphy.com", "https://api.giphy.com/v1/gifs/search"];

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

    panic!(
        "no keys found in {:?} (encrypted keys not supported)",
        filepath
    );
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
        let (client_stream, _) = match listener.accept().await {
            Ok(stream) => stream,
            Err(_) => continue,
        };

        let mut client_stream_tls = match establish_tls(client_stream, &tls_acceptor).await {
            Ok(stream) => stream,
            Err(_) => continue,
        };

        let rcvd_http_request = match parse_http_request(&mut client_stream_tls).await {
            Ok(request) => request,
            Err(_) => continue,
        };

        if !is_http_connect(&rcvd_http_request).await {
            if let Err(_) = send_unsupported_method_status(&mut client_stream_tls).await {
                continue;
            }
            continue;
        }

        if !is_permitted_destination(&rcvd_http_request.uri).await {
            if let Err(_) = send_forbidden_status(&mut client_stream_tls).await {
                continue;
            }
            continue;
        }

        let mut dst_stream = match TcpStream::connect(rcvd_http_request.uri).await {
            Ok(stream) => stream,
            Err(_) => continue,
        };

        if let Err(_) = send_ok_status(&mut client_stream_tls).await {
            continue;
        }

        client_stream_tls.flush().await.unwrap();
        if io::copy_bidirectional(&mut client_stream_tls, &mut dst_stream)
            .await
            .is_err()
        {
            continue;
        }
        client_stream_tls.shutdown().await.unwrap();
    }
}

async fn establish_tls(
    stream: TcpStream,
    acceptor: &TlsAcceptor,
) -> Result<TlsStream<TcpStream>, std::io::Error> {
    let acceptor = acceptor.clone();
    acceptor.accept(stream).await
}

async fn parse_http_request(
    client_stream: &mut TlsStream<TcpStream>,
) -> Result<HttpRequest, Box<dyn std::error::Error>> {
    let mut buffer: [u8; 512] = [0; 512];
    client_stream.read(&mut buffer).await?;
    let req = match std::str::from_utf8(&buffer) {
        Ok(r) => r,
        Err(e) => return Err(Box::new(e)),
    };
    let req = req.split(' ').collect::<Vec<&str>>();
    let method = req[0].to_string();
    let uri = req[1].to_string();
    Ok(HttpRequest { method, uri })
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
}

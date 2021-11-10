use async_std::io::{self, BufReader, WriteExt};
use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_tls::TlsAcceptor;
use clap::{load_yaml, App};
use std::fs;
use std::sync::Arc;

struct ProxyOptions {
    bind_address: String,
    bind_port: String,
    certs: Vec<rustls::Certificate>,
    private_key: rustls::PrivateKey,
}

#[async_std::main]
async fn main() {
    let proxy_options = parse_cli();
    let tls_config = build_tls_config(&proxy_options);

    let acceptor = TlsAcceptor::from(tls_config.await);
    let listener = TcpListener::bind(format!(
        "{}:{}",
        proxy_options.bind_address, proxy_options.bind_port
    ))
    .await
    .unwrap();

    while let Some(stream) = listener.incoming().next().await {
        let acceptor = acceptor.clone();
        let mut buffer: [u8; 8192] = [0; 8192];
        let stream = stream.unwrap();

        let handshake = acceptor.accept(&stream);

        let tls_stream = handshake.await;
        if let Err(_) = tls_stream {
            continue;
        }

        let mut tls_stream = tls_stream.unwrap();

        tls_stream.read(&mut buffer).await.unwrap();
        let mut headers = [httparse::EMPTY_HEADER; 4];
        let mut req = httparse::Request::new(&mut headers);
        let result = req.parse(&buffer);
        if result.is_err() {
            continue;
        }
        if req.method.unwrap() != "CONNECT" {
            tls_stream
                .write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n")
                .await
                .unwrap();
            tls_stream.flush().await.unwrap();
            continue;
        }

        // // TODO - verify path

        let mut dst_stream = TcpStream::connect(req.path.unwrap()).await.unwrap();
        tls_stream
            .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
            .await
            .unwrap();
        tls_stream.flush().await.unwrap();

        let mut buffer: [u8; 8192] = [0; 8192];
        loop {
            let read_size = tls_stream.read(&mut buffer).await.unwrap();
            // let mut buffer = vec!();
            //let read_size = read_into_buffer(&mut tls_stream, &mut buffer).await.unwrap();

            println!("{} 93", read_size);
            let write_size = dst_stream.write(&mut buffer[0..read_size]).await.unwrap();
            println!("{} 115", write_size);
            dst_stream.flush().await.unwrap();

            let read_size = dst_stream.read(&mut buffer).await.unwrap();
            println!("{} 125", read_size);
            let write_size = tls_stream.write(&mut buffer[0..read_size]).await.unwrap();
            println!("{} 126", write_size);
            tls_stream.flush().await.unwrap();
        }
    }
}

async fn read_into_buffer<R>(stream: &mut R, buffer: &mut [u8] ) -> io::Result<usize>
where
    R: async_std::io::Read + std::marker::Unpin,
{
    let mut cursor = 0;
    let mut stream_reader = BufReader::new(stream);
    loop {
        let mut buffer: [u8; 8192] = [0; 8192];
        println!("Loop start");
        let bytes_read = stream_reader.read(&mut buffer).await;
        println!("{:#?}", bytes_read);
        println!("{}", cursor);
        match bytes_read {
            Ok(0) => return Ok(cursor),
            Ok(num_bytes) => {
                println!("{:#?}", bytes_read);
                cursor += num_bytes;
            }
            Err(e) => return io::Result::Err(e),
        }
        println!("Loop end");
    }
}

async fn build_tls_config(proxy_options: &ProxyOptions) -> Arc<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::new(rustls::NoClientAuth::new());
    config
        .set_single_cert(
            proxy_options.certs.clone(),
            proxy_options.private_key.clone(),
        )
        .expect("bad certifcate/key");
    Arc::new(config)
}

enum ErrorTypes {
    IOError(io::Error),
}

impl From<io::Error> for ErrorTypes {
    fn from(error: io::Error) -> Self {
        ErrorTypes::IOError(error)
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

fn load_certs(filename: &str) -> Vec<rustls::Certificate> {
    let certfile = fs::File::open(filename).expect("cannot open certificate file");
    let mut reader = BufReader::new(certfile);
    rustls_pemfile::certs(&mut reader)
        .unwrap()
        .iter()
        .map(|v| rustls::Certificate(v.clone()))
        .collect()
}

fn load_private_key(filename: &str) -> rustls::PrivateKey {
    let keyfile = fs::File::open(filename).expect("cannot open private key file");
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
        filename
    );
}

/// Check both domain + path, in the event the domain has an endpoint with an open redirect
const PERMITTED_URLS: &'static [&str] = &["api.giphy.com/v1/gifs/search"];

fn is_permitted_url(url: &str) -> bool {
    PERMITTED_URLS.contains(&url)
}

// fn is_http_connect(req: Vec<u8>) -> bool {

// }

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

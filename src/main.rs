use clap::{App, load_yaml};
use tokio::net::TcpListener;

fn main() {
    build_cli();
    //let listener = TcpListener::bind("127.0.0.1:8080").await?;
}

fn build_cli() {
    let yaml = load_yaml!("cli.yml");
    let matches = App::from_yaml(yaml).get_matches();
}

#[tokio::main]
async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

/// Check both domain + path, in the event the domain has an endpoint with an open redirect
const PERMITTED_URLS: &'static [&str] = &["api.giphy.com/v1/gifs/search"];


fn is_permitted_url(url: &str) -> bool {
    PERMITTED_URLS.contains(&url)
}

#[cfg(test)]
mod tests {
    use crate::*;

    struct TestCase<I, E> {
        input: I,
        expected: E
    }

    #[test]
    fn test_is_permitted_url() {
        let cases = vec!(
            TestCase { input: "api.giphy.com/v1/gifs/search", expected: true },
            TestCase { input: "api.giphy.com/v1/gifs/disallowed-endpoint", expected: false },
            TestCase { input: "different.url.com/v1/gifs/search", expected: false }
        );

        for c in cases {
            assert_eq!(is_permitted_url(&c.input), c.expected);
        }
    }
}
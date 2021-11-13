# gif-privacy-proxy
A privacy-preserving proxy using HTTP tunneling (HTTP CONNECT) to search for GIFs.

## To Run
```
cargo run
```

## To Run with Optimization
```
cargo run --release
```

## To Test Connectivity
1. Run the proxy
2. Use the following `curl` command to query the Giphy API through the proxy:
```
curl --proxy-insecure -x https://127.0.0.1:8080  "https://api.giphy.com/v1/gifs/search?api_key=<YOUR GIPHY API KEY>&q=<YOUR QUERY TERM>"
```

_Note that `--proxy-insecure` is needed if using a self-signed certificate._

## Load Testing
Load testing was done using `jmeter` and was loaded from the following `curl` command:
```
curl -x https://127.0.0.1:8080  "https://api.giphy.com/v1/gifs/search?api_key=<REDACTED>&q=banjo"
```

The proxy was stable at 100 local threads with a 30-second ramp-up time.

## Future Steps
1. Moved permitted endpoints into a config file that's loaded on run.
2. Add a reasonable timeout for stalled connections.
3. Add retry logic and make error handling logic more transparent. In this initial version, errors terminate the TCP connection with the client, with limited best-effort error notification.
4. Add failure logging/monitoring, while respecting user privacy.
5. Identify expected load and load test realistic conditions. Add load-balancing upstream, as appropriate.
6. Add auto-scaling.
7. Add DDoS protections.
8. Add integration tests.
9. Add basic server throttling.
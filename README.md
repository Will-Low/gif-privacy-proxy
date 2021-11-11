# gif-privacy-proxy
A privacy-preserving proxy using HTTP tunneling (HTTP CONNECT) to search for GIFs.

## To Run
```
cargo run
```

## To Build/Run with Optimization
```
cargo build --release

target/release/gif-privacy-proxy
```

## To Test Connectivity
1. Run the proxy
```
curl --proxy-insecure -x https://127.0.0.1:8080  "https://api.giphy.com/v1/gifs/search"
```

## Next Steps
1. Moved permitted endpoints into a config file that's loaded on run.
2. 
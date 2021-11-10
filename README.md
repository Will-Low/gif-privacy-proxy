# gif-privacy-proxy
A privacy-preserving proxy to retrieve GIFs

## To Run
```
cargo run
```

## To Build/Run with Optimization
```
cargo build --release

target/release/gif-privacy-proxy
```

## Testing 
```
curl -x https://127.0.0.1:8080 -D- https://github.com --proxy-insecure
```

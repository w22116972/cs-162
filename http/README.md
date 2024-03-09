# HTTP

### Goal

Implement an HTTP server
- handles HTTP GET requests
- provide functionality through the use of HTTP response headers
- support for HTTP error codes
- create directory listings with HTML
- create a HTTP proxy

The request and response headers must comply with the HTTP 1.0 protocol

### Steps

```shell
# Build and run the server
cargo build && ./target/debug/http_server_rs --files www/
```

```shell
# Expect 200 OK and body of index.html
curl -v http://localhost:8080/index.html
# Expect 404 Not Found
curl -v http://localhost:8080/does_not_exist.html
# Expect 200 OK and body of index.html
curl -v http://localhost:8080/
```

### ref

- https://inst.eecs.berkeley.edu/~cs162/sp23/static/hw/hw-http-rs/
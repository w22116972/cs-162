use std::{env};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};

use crate::{args};

use crate::http::*;

use clap::Parser;
use tokio::net::{TcpListener, TcpStream};

use anyhow::Result;
use lazy_static::lazy_static;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;
use crate::stats::{Stats, StatsPtr};

// ref: https://docs.rs/lazy_static/latest/lazy_static/
lazy_static!{
    // Initialize a new StatsPtr instance for collecting statistics
    static ref STATS: StatsPtr = StatsPtr::new(RwLock::from(Stats::new()));
}

pub fn main() -> Result<()> {
    // Configure logging
    // You can print logs (to stderr) using
    // `log::info!`, `log::warn!`, `log::error!`, etc.
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .init();

    // Parse command line arguments
    let args = args::Args::parse();

    // Set the current working directory
    env::set_current_dir(&args.files)?;

    // Print some info for debugging
    log::info!("HTTP server initializing ---------");
    log::info!("Port:\t\t{}", args.port);
    log::info!("Num threads:\t{}", args.num_threads);
    log::info!("Directory:\t\t{}", &args.files);
    log::info!("----------------------------------");

    // Initialize a thread pool that starts running `listen`
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(args.num_threads)
        .build()?
        .block_on(listen(args.port))
}

/// 1. Bind the server socket to the provided port with a HOST of 0.0.0.0
/// 2. Listens for incoming connections. For each incoming connection, a new task is spawned to handle it.
async fn listen(port: u16) -> Result<()> {
    // Use Ipv4Addr::UNSPECIFIED as 0.0.0.0
    const INADDR_ANY: Ipv4Addr = Ipv4Addr::UNSPECIFIED;
    let socket_addr: SocketAddrV4 = SocketAddrV4::new(INADDR_ANY, port);

    // Use TcpListener::bind to bind the server socket to the provided port with the HOST
    let listener: TcpListener = TcpListener::bind(socket_addr).await.unwrap();
    // Use a loop to listen for incoming connections
    loop {
        // Use TcpListener::accept to listen for incoming connections
        let (socket, _) = listener.accept().await.unwrap();
        // A new task is spawned for each inbound socket.
        // The socket is moved to the new task and processed there.
        tokio::spawn(async move {
            handle_socket(socket).await?;
            Ok::<(), anyhow::Error>(())
        });
    }
}

/// Handles a single connection via `socket`.
async fn handle_socket(mut socket: TcpStream) -> Result<()> {
    // Parse the request from the client socket
    let request: Request = parse_request(&mut socket).await?;

    // If the request path is /stats, call handle_stats and return the result
    if request.path == "/stats" {
        handle_stats(&mut socket).await?;
        return Ok(())
    }

    // To access local files, prepend a “.” to the request path
    let file_path: String = format!(".{}", request.path);
    // Check if the file exists
    match tokio::fs::metadata(&file_path).await {
        // If the file denoted by path exists, serve the file. Read the contents of the file and write it to the client socket
        Ok(metadata) if metadata.is_file() => {
            let mut headers = HashMap::new();
            // Make sure to set the Content-Type header to be the MIME type indicated by the file extension.
            headers.insert("Content-Type".to_string(), get_mime_type(&file_path).to_string());
            // Set the Content-Length to be size of HTTP response body in bytes
            headers.insert("Content-Length".to_string(), metadata.len().to_string());
            // Send the HTTP headers to the client socket
            write_header_to_socket(&mut socket, 200, Some(headers)).await?;
            // Send the file contents to the client socket by chunks of at most 1024 bytes
            write_file_chunks_to_socket(&file_path, &mut socket).await?;
            // Increment the count for the status code in the STATS
            STATS.write().await.incr(200);
        },
        // If the fila path is a directory
        Ok(metadata) if metadata.is_dir() => {
            match tokio::fs::metadata(format_index(&file_path)).await {
                // If the directory contains an index.html file
                Ok(metadata) if metadata.is_file() => {
                    let mut headers = HashMap::new();
                    // Make sure to set the Content-Type header to be the MIME type indicated by the file extension.
                    headers.insert("Content-Type".to_string(), get_mime_type(&file_path).to_string());
                    // Set the Content-Length to be size of HTTP response body in bytes
                    headers.insert("Content-Length".to_string(), metadata.len().to_string());
                    // Send the HTTP headers to the client socket
                    write_header_to_socket(&mut socket, 200, Some(headers)).await?;
                    // Send the file contents to the client socket by chunks of at most 1024 bytes
                    write_file_chunks_to_socket(&format_index(&file_path), &mut socket).await?;
                    // Increment the count for the status code in the STATS
                    STATS.write().await.incr(200);
                },
                // If the directory does not contain an index.html file
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    // Respond with an HTML page containing links to all of immediate children of the directory
                    let mut headers = HashMap::new();
                    headers.insert("Content-Type".to_string(), "text/html".to_string());
                    write_header_to_socket(&mut socket, 200, Some(headers)).await?;
                    // To list the contents of a directory, use tokio::fs::read_dir
                    // Use http::format_href to format the HTML for each file
                    // Use Path and PathBuf to work with file paths
                    let mut entries = tokio::fs::read_dir(&file_path).await?;
                    while let Some(entry) = entries.next_entry().await? {
                        let filename = entry.file_name();
                        let path = entry.path();
                        let href = format_href(&path.into_os_string().into_string().unwrap(),
                                               &filename.as_os_str().to_string_lossy());
                        socket.write_all(href.as_bytes()).await?;
                    }
                    STATS.write().await.incr(200);
                },
                // If you encounter any other errors, use log::warn! to print out the error
                Err(e) => {
                    log::warn!("Error: {}", e);
                    // Increment the count for the status code in the STATS
                    STATS.write().await.incr(404);
                }
                _ => {}
            }
        },
        // If the directory does not exist, serve a 404 Not Found response to the client.
        Ok(_) => {
            write_header_to_socket(&mut socket, 404, None).await?;
            STATS.write().await.incr(404);
        },
        // If the file and dir does not exist, serve a 404 Not Found response
        Err(e) if e.kind() == ErrorKind::NotFound => {
            // Write a 404 Not Found response (the HTTP headers and body are optional)
            write_header_to_socket(&mut socket, 404, None).await?;
            STATS.write().await.incr(404);
        },
        // If you encounter any other errors, use log::warn! to print out the error
        Err(e) => {
            log::warn!("Error: {}", e);
            STATS.write().await.incr(500);
        }
    }
    Ok(())
}

// Task: No more than 1024 bytes of the file should be loaded into memory at any given time.
//       To alternate between reading the file into a buffer and writing the buffer contents to the socket.
/// Send the file contents to the client socket by chunks of at most 1024 bytes
async fn write_file_chunks_to_socket(file_path: &str, socket: &mut TcpStream) -> Result<()> {
    let mut file = File::open(&file_path).await?;
    let mut buf = vec![0; 1024]; // Buffer to read chunks of file content
    loop {
        let bytes_read = file.read(&mut buf).await?; // Read a chunk of the file into the buffer
        if bytes_read == 0 {
            break; // End of file reached
        }
        socket.write_all(&buf[..bytes_read]).await?; // Write the read chunk to the socket
    }
    Ok::<(), anyhow::Error>(())
}

async fn write_header_to_socket(socket: &mut TcpStream, status_code: u32, headers_option: Option<HashMap<String, String>>) -> Result<()>{
    start_response(socket, status_code).await?;
    if let Some(headers) = headers_option {
        for (key, value) in headers {
            send_header(socket, &key, &value).await?;
        }
    }
    // For blank line between headers and body
    end_headers(socket).await?;
    Ok::<(), anyhow::Error>(())
}

async fn handle_stats(socket: &mut TcpStream) -> Result<()> {
    let stats = STATS.read().await;
    let items = stats.items();
    let mut headers = HashMap::new();
    headers.insert("Content-Type".to_string(), "text/plain".to_string());
    write_header_to_socket(socket, 200, Some(headers)).await?;
    for (status_code, count) in items {
        let msg = format!("{}: {}\r\n", response_message(status_code), count);
        socket.write_all(msg.as_bytes()).await?;
    }
    Ok::<(), anyhow::Error>(())
}
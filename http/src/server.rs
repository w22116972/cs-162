use std::{env};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};

use crate::{args};

use crate::http::*;

use clap::Parser;
use tokio::net::{TcpListener, TcpStream};

use anyhow::Result;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    const INADDR_ANY: Ipv4Addr = Ipv4Addr::UNSPECIFIED;
    let socket_addr: SocketAddrV4 = SocketAddrV4::new(INADDR_ANY, port);

    // Use TcpListener::bind to bind the server socket to the provided port with the HOST
    let listener: TcpListener = TcpListener::bind(socket_addr).await.unwrap();
    loop {
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
    // To access local files, prepend a “.” to the request path
    let file_path: String = format!(".{}", request.path);
    // Check if the file exists
    match tokio::fs::metadata(&file_path).await {
        // If the file denoted by path exists, serve the file. Read the contents of the file and write it to the client socket
        Ok(metadata) if metadata.is_file() => {
            // Send a 200 OK response
            start_response(&mut socket, 200).await?;
            // Make sure to set the Content-Type header to be the MIME type indicated by the file extension.
            send_header(&mut socket, "Content-Type", get_mime_type(&file_path)).await?;
            // Set the Content-Length to be size of HTTP response body in bytes
            send_header(&mut socket, "Content-Length", &metadata.len().to_string()).await?;
            // For blank line between headers and body
            end_headers(&mut socket).await?;
            // Write the file to the client socket
            let mut file = File::open(file_path).await?;
            // No more than 1024 bytes of the file should be loaded into memory at any given time.
            // To alternate between reading the file into a buffer and writing the buffer contents to the socket.
            let mut buf = vec![0; 1024]; // Buffer to read chunks of file content
            loop {
                let bytes_read = file.read(&mut buf).await?; // Read a chunk of the file into the buffer
                if bytes_read == 0 {
                    break; // End of file reached
                }
                socket.write_all(&buf[..bytes_read]).await?; // Write the read chunk to the socket
            }
        },
        // If the fila path is a directory
        Ok(metadata) if metadata.is_dir() => {
            match tokio::fs::metadata(format_index(&file_path)).await {
                // If the directory contains an index.html file
                Ok(metadata) if metadata.is_file() => {
                    // respond with a 200 OK
                    start_response(&mut socket, 200).await?;
                    end_headers(&mut socket).await?;
                    // and the full contents of the index.html
                    let mut file = File::open(format_index(&file_path)).await?;
                    // No more than 1024 bytes of the file should be loaded into memory at any given time.
                    // To alternate between reading the file into a buffer and writing the buffer contents to the socket.
                    let mut buf = vec![0; 1024]; // Buffer to read chunks of file content
                    loop {
                        let bytes_read = file.read(&mut buf).await?; // Read a chunk of the file into the buffer
                        if bytes_read == 0 {
                            break; // End of file reached
                        }
                        socket.write_all(&buf[..bytes_read]).await?; // Write the read chunk to the socket
                    }

                },
                // If the directory does not contain an index.html file
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    // Respond with an HTML page containing links to all of immediate children of the directory
                    start_response(&mut socket, 200).await?;
                    send_header(&mut socket, "Content-Type", "text/html").await?;
                    end_headers(&mut socket).await?;
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
                },
                // If you encounter any other errors, use log::warn! to print out the error
                Err(e) => {
                    log::warn!("Error: {}", e);
                }
                _ => {}
            }
        },
        // If the directory does not exist, serve a 404 Not Found response to the client.
        Ok(_) => {
            start_response(&mut socket, 404).await?;
            end_headers(&mut socket).await?;
        },
        // If the file and dir does not exist, serve a 404 Not Found response
        Err(e) if e.kind() == ErrorKind::NotFound => {
            // Build and send a 404 Not Found response (the HTTP headers and body are optional)
            start_response(&mut socket, 404).await?;
            end_headers(&mut socket).await?;
        },
        // If you encounter any other errors, use log::warn! to print out the error
        Err(e) => {
            log::warn!("Error: {}", e);
        }
    }
    Ok(())
}

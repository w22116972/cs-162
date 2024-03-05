//! The gRPC server.
//!

use std::collections::HashMap;
use crate::{log, rpc::kv_store::*, SERVER_ADDR};
use anyhow::Result;
use tonic::{transport::Server, Request, Response, Status};

// Define a struct KvStore that will store the state of our server.
pub struct KvStore {
    // Task: Store the reference-counted lock
    // Use tokio::sync::RwLock to synchronize access to the store HashMap<Vec<u8>, Vec<u8>>.
    // Use std::sync::Arc to keep track of references to the lock in a thread-safe manner.
    lock: std::sync::Arc<tokio::sync::RwLock<HashMap<Vec<u8>, Vec<u8>>>>,
}

// Trait kv_store_server::KvStore in kv_store.rs
#[tonic::async_trait]
impl kv_store_server::KvStore for KvStore {
    async fn example(
        &self,
        req: Request<ExampleRequest>,
    ) -> Result<Response<ExampleReply>, Status> {
        log::info!("Received example request.");
        Ok(Response::new(ExampleReply {
            output: req.into_inner().input + 1,
        }))
    }

    async fn echo(&self, request: Request<EchoRequest>) -> std::result::Result<Response<EchoReply>, Status> {
        let message = request.into_inner().input;
        Ok(Response::new(EchoReply {
            output: message,
        }))
    }

    async fn put(&self, request: Request<PutRequest>) -> Result<Response<()>, Status> {
        let mut lock = self.lock.write().await;
        let request_entry = request.into_inner();
        let key = request_entry.key;
        let value = request_entry.value;
        lock.insert(key, value);
        Ok(Response::new(()))
    }

    async fn get(&self, request: Request<GetRequest>) -> std::result::Result<Response<GetReply>, Status> {
        let lock = self.lock.read().await;
        let key = request.into_inner().key;
        match lock.get(&key) {
            Some(value) => Ok(Response::new(GetReply {
                value: value.clone(),
            })),
            // If the key is not found, return Err(tonic::Status::new(tonic::Code::NotFound, "Key does not exist."))
            None => Err(Status::new(tonic::Code::NotFound, "Key does not exist.")),
        }
    }

}

pub async fn start() -> Result<()> {
    let svc = kv_store_server::KvStoreServer::new(KvStore {
        lock: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
    });

    log::info!("Starting KV store server.");
    Server::builder()
        .add_service(svc)
        .serve(SERVER_ADDR.parse().unwrap())
        .await?;
    Ok(())
}

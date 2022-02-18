use std::net::SocketAddr;

#[derive(Debug)]
pub enum ApiEvent {
  ServiceInitialized(SocketAddr)
}

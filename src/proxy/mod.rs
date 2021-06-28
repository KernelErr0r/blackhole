use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use futures::stream::SplitSink;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_util::codec::{BytesCodec, Framed};
use std::sync::Arc;

pub type ClientTx = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;
pub type ServerTx = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;
pub type TunnelHandler = Box<dyn Handler + Send>;

pub struct Proxy {
    tunnel_configurations: Vec<TunnelConfiguration>
}

impl Proxy {
    pub fn new() -> Proxy {
        Proxy { 
            tunnel_configurations: Vec::new() 
        }
    }
}

pub struct Tunnel {
    pub client_tx: ClientTx,
    pub server_tx: ServerTx,
}

impl Tunnel {
    pub fn new(client_tx: ClientTx, server_tx: ServerTx) -> Tunnel {
        Tunnel {
            client_tx,
            server_tx
        }
    }
}

pub struct TunnelConfiguration {
    source_address: String,
    source_port: u16,
    target_address: String,
    target_port: u16,
    handler: TunnelHandler,
}

impl TunnelConfiguration {
    pub fn new(source_address: String, source_port: u16, target_address: String, target_port: u16, handler: TunnelHandler) -> TunnelConfiguration {
        TunnelConfiguration {
            source_address,
            source_port,
            target_address,
            target_port,
            handler,
        }
    }
}

#[derive(Eq, PartialEq)]
pub enum Direction {
    Client2Server,
    Server2Client
}

pub trait Handler {
    fn handle_data(&self, tunnel: Arc<Mutex<Tunnel>>, direction: Direction, bytes: BytesMut);
}

impl Proxy {
    pub fn add_tcp_tunnel(&mut self, tunnel_configuration: TunnelConfiguration) {
        self.tunnel_configurations.push(tunnel_configuration);
    }

    pub fn start(self) {
        for tunnel_configuration in self.tunnel_configurations {
            tokio::spawn(async move {
                match TcpListener::bind(format!("{}:{}", tunnel_configuration.source_address, tunnel_configuration.source_port)).await {
                    Ok(listener) => {
                        let tunnel_configuration = Arc::new(Mutex::new(tunnel_configuration));

                        while let Ok((client, _)) = listener.accept().await {
                            let tunnel_configuration1 = tunnel_configuration.clone();
                            let tunnel_configuration2 = tunnel_configuration1.clone();

                            match TcpStream::connect(format!("{}:{}", tunnel_configuration1.lock().await.target_address, tunnel_configuration1.lock().await.target_port)).await {
                                Ok(server) => {
                                    let client = Framed::new(client, BytesCodec::new());
                                    let server = Framed::new(server, BytesCodec::new());

                                    let (client_tx, mut client_rx) = client.split();
                                    let (server_tx, mut server_rx) = server.split();

                                    let tunnel1 = Arc::new(Mutex::new(Tunnel::new(client_tx, server_tx)));
                                    let tunnel2 = tunnel1.clone();

                                    let cs_proxy_task = tokio::spawn(async move {
                                        loop {
                                            match client_rx.next().await {
                                                Some(raw_data) => {
                                                    if let Ok(raw_data) = raw_data {
                                                        tunnel_configuration1.lock().await.handler.handle_data(tunnel1.clone(), Direction::Client2Server, raw_data);
                                                    }
                                                },
                                                None => return,
                                            }
                                        }
                                    });

                                    let sc_proxy_task = tokio::spawn(async move {
                                        loop {
                                            match server_rx.next().await {
                                                Some(raw_data) => {
                                                    if let Ok(raw_data) = raw_data {
                                                        tunnel_configuration2.lock().await.handler.handle_data(tunnel2.clone(), Direction::Server2Client, raw_data);
                                                    }
                                                },
                                                None => return,
                                            }
                                        }
                                    });

                                    tokio::select! {
                                        _ = cs_proxy_task => {}
                                        _ = sc_proxy_task => {}
                                    };
                                },
                                Err(err) => {
                                    println!("Failed to connect to host {}:{} because of: {}", tunnel_configuration1.lock().await.target_address, tunnel_configuration1.lock().await.target_port, err);
                                },
                            }
                        }
                    },
                    Err(err) => {
                        println!("Failed to start listening on {}:{} because of: {}", tunnel_configuration.source_address, tunnel_configuration.source_port, err);
                    },
                }
            });
        }
    }
}
use std::{
  collections::HashMap,
  net::{IpAddr, SocketAddr},
};

use tokio::{io, net::UdpSocket, sync::mpsc::Sender};

use crate::{packet::ControlPacket, session::SessionHandle};

pub struct Client {
  rx_socket: UdpSocket,
  discriminator_map: HashMap<u32, Sender<ControlPacket>>,
  addr_map: HashMap<IpAddr, Sender<ControlPacket>>,
}

impl Client {
  pub async fn new() -> Result<Self, io::Error> {
    let rx_socket = UdpSocket::bind("0.0.0.0:3784").await?;

    Ok(Self {
      rx_socket,
      discriminator_map: HashMap::new(),
      addr_map: HashMap::new(),
    })
  }

  pub async fn poll(&mut self) {
    let mut buf = [0; 1500];
    if let Ok((len, addr)) = self.rx_socket.recv_from(&mut buf).await {
      self.handle_msg(&buf[0..len], addr).await;
    }
  }

  pub fn register_session(&mut self, handle: SessionHandle) {
    self.addr_map.insert(handle.remote_addr, handle.packet_tx);
  }

  async fn handle_msg(&mut self, buf: &[u8], addr: SocketAddr) {
    let packet = ControlPacket::decode(buf);
    let packet = match packet {
      Ok(packet) => packet,
      Err(e) => {
        println!("packet decode error {e:?}");
        return;
      }
    };

    let discriminator = packet.my_discriminator;
    if let Some(tx) = self.discriminator_map.get(&discriminator) {
      if tx.send(packet).await.is_err() {
        self.discriminator_map.remove(&discriminator);
      }
      return;
    }

    if let Some(tx) = self.addr_map.get(&addr.ip()) {
      if tx.send(packet).await.is_err() {
        self.addr_map.remove(&addr.ip());
      } else {
        self.discriminator_map.insert(discriminator, tx.clone());
      }
    }
  }
}

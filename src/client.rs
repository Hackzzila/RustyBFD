use std::{
  collections::HashMap,
  net::{IpAddr, SocketAddr},
};

use tokio::{
  io,
  net::UdpSocket,
  sync::mpsc::{self, Receiver, Sender},
};

use crate::packet::ControlPacket;

pub(crate) enum Event {
  NewSession(IpAddr, Sender<ControlPacket>),
}

pub struct Client {
  pub(crate) event_tx: Sender<Event>,
}

impl Client {
  pub async fn new() -> Result<(Client, EventLoop), io::Error> {
    let rx_socket = UdpSocket::bind("0.0.0.0:3784").await?;

    let (event_tx, event_rx) = mpsc::channel(10);

    let client = Self { event_tx };

    let event_loop = EventLoop {
      rx_socket,
      event_rx,
      discriminator_map: HashMap::new(),
      addr_map: HashMap::new(),
    };

    Ok((client, event_loop))
  }
}

pub struct EventLoop {
  rx_socket: UdpSocket,
  event_rx: Receiver<Event>,
  discriminator_map: HashMap<u32, Sender<ControlPacket>>,
  addr_map: HashMap<IpAddr, Sender<ControlPacket>>,
}

impl EventLoop {
  pub async fn poll(&mut self) {
    let mut buf = [0; 1500];
    tokio::select! {
      Some(event) = self.event_rx.recv() => {
        self.handle_event(event).await;
      }

      Ok((len, addr)) = self.rx_socket.recv_from(&mut buf) => {
        self.handle_msg(&buf[0..len], addr).await;
      }
    }
  }

  async fn handle_event(&mut self, event: Event) {
    match event {
      Event::NewSession(addr, tx) => {
        self.addr_map.insert(addr, tx);
      }
    }
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

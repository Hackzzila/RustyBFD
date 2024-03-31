use std::{
  net::{IpAddr, Ipv4Addr},
  time::Duration,
};

use client::Client;
use session::SessionOpt;

mod client;
mod packet;
mod session;

#[tokio::main]
async fn main() {
  let (client, mut event_loop) = Client::new().await.unwrap();

  tokio::task::spawn(async move {
    loop {
      event_loop.poll().await;
    }
  });

  let mut sess = client
    .new_session(SessionOpt {
      local_addr: IpAddr::V4(Ipv4Addr::new(172, 20, 5, 164)),
      remote_addr: IpAddr::V4(Ipv4Addr::new(172, 20, 5, 1)),
      local_discriminator: rand::random(),
      demand_mode: false,
      desired_min_tx: Duration::from_millis(1000),
      required_min_rx: Duration::from_millis(300),
      detect_mult: 3,
    })
    .await;

  loop {
    sess.poll().await;
  }
}

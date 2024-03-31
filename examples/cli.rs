use std::net::{IpAddr, Ipv4Addr};

use clap::Parser;
use rustybfd::*;
use tracing::debug;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
  #[arg(short, long)]
  local_addr: Ipv4Addr,

  #[arg(short, long)]
  remote_addr: Ipv4Addr,

  #[arg(long, default_value = "1s")]
  desired_min_tx: humantime::Duration,

  #[arg(long, default_value = "300ms")]
  required_min_rx: humantime::Duration,

  #[arg(long)]
  local_discriminator: Option<u32>,

  #[arg(long, short = 'm', default_value_t = 3)]
  detect_mult: u8,
}

#[tokio::main]
async fn main() {
  let subscriber = tracing_subscriber::FmtSubscriber::new();
  tracing::subscriber::set_global_default(subscriber).unwrap();

  let args = Args::parse();

  let (client, mut event_loop) = Client::new().await.unwrap();

  tokio::task::spawn(async move {
    loop {
      event_loop.poll().await;
    }
  });

  let mut sess = client
    .new_session(SessionOpt {
      local_addr: IpAddr::V4(args.local_addr),
      remote_addr: IpAddr::V4(args.remote_addr),
      local_discriminator: args.local_discriminator.unwrap_or_else(rand::random),
      demand_mode: false,
      desired_min_tx: args.desired_min_tx.into(),
      required_min_rx: args.required_min_rx.into(),
      detect_mult: args.detect_mult,
    })
    .await;

  loop {
    sess.poll().await;
  }
}

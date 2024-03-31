use std::{
  fmt::Debug,
  net::{IpAddr, SocketAddr},
  pin::Pin,
  time::Duration,
};

use rand::Rng;
use tokio::{
  net::UdpSocket,
  sync::mpsc::{self, Receiver, Sender},
  time::{Instant, Sleep},
};
use tracing::{info, instrument, warn};

use crate::packet::{ControlPacket, Diagnostic, SessionState};

pub struct SessionHandle {
  pub(crate) remote_addr: IpAddr,
  pub(crate) packet_tx: Sender<ControlPacket>,
}

#[derive(Debug, Clone)]
pub struct SessionOpt {
  pub local_addr: IpAddr,
  pub remote_addr: IpAddr,
  pub local_discriminator: u32,
  pub demand_mode: bool,
  pub desired_min_tx: Duration,
  pub required_min_rx: Duration,
  pub detect_mult: u8,
}

impl Session {
  pub async fn new(opt: SessionOpt) -> (Session, SessionHandle) {
    let (packet_tx, packet_rx) = mpsc::channel(10);
    let tx_socket = UdpSocket::bind(SocketAddr::new(opt.local_addr, 0)).await.unwrap();
    tx_socket.set_ttl(255).unwrap();

    let sess = Session {
      packet_rx,
      tx_socket,
      state: SessionState::Down,
      remote_state: SessionState::Down,
      local_discriminator: opt.local_discriminator,
      remote_discriminator: 0,
      local_diagnostic: Diagnostic::NoDiagnostic,
      remote_diagnostic: Diagnostic::NoDiagnostic,
      desired_min_tx: opt.desired_min_tx,
      required_min_rx: opt.required_min_rx,
      remote_min_rx: Duration::from_micros(1),
      demand_mode: opt.demand_mode,
      remote_demand_mode: false,
      detect_mult: opt.detect_mult,
      auth_type: (),
      remote_addr: opt.remote_addr,
      sleep: Box::pin(tokio::time::sleep_until(Instant::now())),
      set_poll: true,
      timeout_sleep: Box::pin(tokio::time::sleep_until(Instant::now())),
      detection_time: Duration::ZERO,
    };

    let handle = SessionHandle {
      remote_addr: opt.remote_addr,
      packet_tx,
    };

    (sess, handle)
  }
}

pub struct Session {
  packet_rx: Receiver<ControlPacket>,
  tx_socket: UdpSocket,
  state: SessionState,
  remote_state: SessionState,
  local_discriminator: u32,
  remote_discriminator: u32,
  local_diagnostic: Diagnostic,
  remote_diagnostic: Diagnostic,
  desired_min_tx: Duration,
  required_min_rx: Duration,
  remote_min_rx: Duration,
  demand_mode: bool,
  remote_demand_mode: bool,
  detect_mult: u8,
  auth_type: (),
  remote_addr: IpAddr,
  sleep: Pin<Box<Sleep>>,
  set_poll: bool,
  timeout_sleep: Pin<Box<Sleep>>,
  detection_time: Duration,
}

impl Session {
  pub fn get_local_state(&self) -> SessionState {
    self.state
  }

  pub fn get_remote_state(&self) -> SessionState {
    self.remote_state
  }

  pub fn get_local_discriminator(&self) -> u32 {
    self.local_discriminator
  }

  pub fn get_remote_discriminator(&self) -> u32 {
    self.remote_discriminator
  }

  pub fn get_local_diagnostic(&self) -> Diagnostic {
    self.local_diagnostic
  }

  pub fn get_remote_diagnostic(&self) -> Diagnostic {
    self.remote_diagnostic
  }

  #[instrument(skip(self), fields(remote_addr = ?self.remote_addr))]
  pub async fn poll(&mut self) {
    fn log_if_changed<T: Debug + PartialEq>(old: T, new: T, msg: &str) {
      if old != new {
        info!(?old, ?new, "{}", msg)
      }
    }

    let old_local_state = self.get_local_state();
    let old_remote_state = self.get_remote_state();
    let old_local_diagnostic = self.get_local_diagnostic();
    let old_remote_diagnostic = self.get_remote_diagnostic();
    let old_remote_discriminator = self.get_remote_discriminator();

    tokio::select! {
      Some(packet) = self.packet_rx.recv() => {
        self.handle_packet(packet).await
      }

      _ = &mut self.sleep => {
        self.send_packet(false).await;
      }

      _ = &mut self.timeout_sleep => {
        if self.state == SessionState::Up {
          warn!("session timed out");
          self.local_diagnostic = Diagnostic::TimeExpired;
          self.state = SessionState::Down;
          self.send_packet(false).await;
        }
      }
    }

    log_if_changed(
      old_remote_discriminator,
      self.get_remote_discriminator(),
      "remote discriminator changed",
    );

    log_if_changed(old_local_state, self.get_local_state(), "local session state changed");

    log_if_changed(
      old_remote_state,
      self.get_remote_state(),
      "remote session state changed",
    );

    log_if_changed(
      old_local_diagnostic,
      self.get_local_diagnostic(),
      "local diagnostic changed",
    );

    log_if_changed(
      old_remote_diagnostic,
      self.get_remote_diagnostic(),
      "remote diagnostic changed",
    );
  }

  async fn send_packet(&mut self, fin: bool) {
    let mut rng = rand::thread_rng();
    let jitter = rng.gen_range(0.75..=1.0);
    let interval = self.remote_min_rx.as_secs_f64().max(self.desired_min_tx.as_secs_f64());
    let interval = Duration::from_secs_f64(interval * jitter);
    let deadline = Instant::now() + interval;
    self.sleep.as_mut().reset(deadline);

    let mut pbuf = [0; 24];

    let mut tx_packet = ControlPacket {
      version: 1,
      diagnostic: self.local_diagnostic,
      state: self.state,
      poll: self.set_poll,
      fin,
      control_plane_independent: false,
      auth_present: false,
      demand_mode: false,
      multipoint: false,
      detect_mult: self.detect_mult,
      my_discriminator: self.local_discriminator,
      your_discriminator: self.remote_discriminator,
      desired_min_tx: self.desired_min_tx,
      required_min_rx: self.required_min_rx,
      required_min_echo_rx: Duration::ZERO,
    };

    if self.state == SessionState::Up {
      tx_packet.demand_mode = true;
    }

    tx_packet.encode(&mut pbuf);

    self
      .tx_socket
      .send_to(&pbuf, SocketAddr::new(self.remote_addr, 3784))
      .await
      .unwrap();
  }

  async fn handle_packet(&mut self, packet: ControlPacket) {
    self.detection_time = Duration::from_secs_f64(
      packet
        .desired_min_tx
        .as_secs_f64()
        .max(self.required_min_rx.as_secs_f64())
        * (packet.detect_mult as f64),
    );

    self.timeout_sleep.as_mut().reset(Instant::now() + self.detection_time);

    self.remote_discriminator = packet.my_discriminator;
    self.remote_state = packet.state;
    self.remote_diagnostic = packet.diagnostic;
    self.remote_min_rx = packet.required_min_rx;
    self.remote_demand_mode = packet.demand_mode;

    if packet.fin {
      self.set_poll = false;
    }

    if self.state == SessionState::AdminDown {
      return;
    }

    self.local_diagnostic = Diagnostic::NoDiagnostic;
    match (self.state, self.remote_state) {
      (SessionState::Down, SessionState::AdminDown) => {}
      (_, SessionState::AdminDown) => {
        self.local_diagnostic = Diagnostic::NeigborSignaledDown;
        self.state = SessionState::Down;
      }

      (SessionState::Down, SessionState::Down) => self.state = SessionState::Init,
      (SessionState::Down, SessionState::Init) => self.state = SessionState::Up,

      (SessionState::Init, SessionState::Up) | (SessionState::Init, SessionState::Init) => {
        self.state = SessionState::Up
      }

      (SessionState::Up, SessionState::Down) => {
        self.local_diagnostic = Diagnostic::NeigborSignaledDown;
        self.state = SessionState::Down;
      }

      (_, _) => {}
    }

    if packet.poll {
      self.send_packet(true).await;
    }
  }
}

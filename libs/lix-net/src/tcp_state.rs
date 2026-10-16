use crate::ipv4::Ipv4Address;
use crate::tcp::{
    TCP_FLAG_ACK, TCP_FLAG_FIN, TCP_FLAG_PSH, TCP_FLAG_RST, TCP_FLAG_SYN, TCP_MSS, TcpSegment,
};
const RTO_BASE_MS: u64 = 500;
const RTO_CAP_MS: u64 = 4000;
const MAX_RETRIES: u32 = 8;
const TIME_WAIT_MS: u64 = 2000;
const PENDING_TX_TIMEOUT_MS: u64 = 10_000;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcpState {
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    Closing,
    TimeWait,
    CloseWait,
    LastAck,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcpError {
    WouldBlock,
    NotConnected,
    Closed,
    Timeout,
    Refused,
    Reset,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TcpEvent {
    None,
    Established,
    Closed,
    Refused,
    Reset,
    TimedOut,
}
#[derive(Clone, Copy, Debug)]
struct Pending {
    seq: u32,
    flags: u16,
    payload_len: usize,
}
impl Pending {
    fn consumed(&self) -> u32 {
        (self.payload_len as u32)
            + u32::from(self.flags & TCP_FLAG_SYN != 0)
            + u32::from(self.flags & TCP_FLAG_FIN != 0)
    }
}
#[derive(Clone, Copy, Debug)]
struct Flight {
    seq: u32,
    end: u32,
    flags: u16,
    payload_len: usize,
}
impl Flight {
    fn from_pending(pending: &Pending) -> Self {
        Self {
            seq: pending.seq,
            end: pending.seq.wrapping_add(pending.consumed()),
            flags: pending.flags,
            payload_len: pending.payload_len,
        }
    }
    const fn data_start(&self) -> u32 {
        if self.flags & TCP_FLAG_SYN != 0 {
            self.seq.wrapping_add(1)
        } else {
            self.seq
        }
    }
}
pub struct PendingInfo<'a> {
    pub seq: u32,
    pub acknowledgment: u32,
    pub flags: u16,
    pub window: u16,
    pub payload: &'a [u8],
}
pub struct TcpInput<'a> {
    pub received: Option<&'a [u8]>,
    pub event: TcpEvent,
}
const fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}
fn seq_gt(a: u32, b: u32) -> bool {
    seq_lt(b, a)
}
pub struct TcpConnection {
    local_port: u16,
    remote_ip: Ipv4Address,
    remote_port: u16,
    state: TcpState,
    active_open: bool,
    snd_una: u32,
    snd_nxt: u32,
    rcv_nxt: u32,
    iss: u32,
    peer_window: u16,
    our_window: u16,
    tx: [u8; TCP_MSS],
    tx_len: usize,
    pending: Option<Pending>,
    pending_since: u64,
    flight: Option<Flight>,
    rto_deadline: u64,
    backoff: u32,
    retries: u32,
    timewait_deadline: u64,
    close_requested: bool,
    fin_sent: bool,
    dead: bool,
}
impl TcpConnection {
    pub fn active_open(
        local_port: u16,
        remote_ip: Ipv4Address,
        remote_port: u16,
        iss: u32,
        now_ms: u64,
    ) -> Self {
        let mut connection = Self {
            local_port,
            remote_ip,
            remote_port,
            state: TcpState::SynSent,
            active_open: true,
            snd_una: iss,
            snd_nxt: iss.wrapping_add(1),
            rcv_nxt: 0,
            iss,
            peer_window: TCP_MSS as u16,
            our_window: TCP_MSS as u16,
            tx: [0; TCP_MSS],
            tx_len: 0,
            pending: None,
            pending_since: 0,
            flight: None,
            rto_deadline: 0,
            backoff: 0,
            retries: 0,
            timewait_deadline: 0,
            close_requested: false,
            fin_sent: false,
            dead: false,
        };
        connection.queue_pending(
            Pending {
                seq: iss,
                flags: TCP_FLAG_SYN,
                payload_len: 0,
            },
            now_ms,
        );
        connection
    }
    pub fn passive_open(
        local_port: u16,
        remote_ip: Ipv4Address,
        remote_port: u16,
        iss: u32,
        irs: u32,
        peer_window: u16,
        now_ms: u64,
    ) -> Self {
        let mut connection = Self {
            local_port,
            remote_ip,
            remote_port,
            state: TcpState::SynReceived,
            active_open: false,
            snd_una: iss,
            snd_nxt: iss.wrapping_add(1),
            rcv_nxt: irs.wrapping_add(1),
            iss,
            peer_window,
            our_window: TCP_MSS as u16,
            tx: [0; TCP_MSS],
            tx_len: 0,
            pending: None,
            pending_since: 0,
            flight: None,
            rto_deadline: 0,
            backoff: 0,
            retries: 0,
            timewait_deadline: 0,
            close_requested: false,
            fin_sent: false,
            dead: false,
        };
        connection.queue_pending(
            Pending {
                seq: iss,
                flags: TCP_FLAG_SYN | TCP_FLAG_ACK,
                payload_len: 0,
            },
            now_ms,
        );
        connection
    }
    pub const fn state(&self) -> TcpState {
        self.state
    }
    pub const fn local_port(&self) -> u16 {
        self.local_port
    }
    pub const fn remote_ip(&self) -> Ipv4Address {
        self.remote_ip
    }
    pub const fn remote_port(&self) -> u16 {
        self.remote_port
    }
    pub const fn is_dead(&self) -> bool {
        self.dead
    }
    pub fn is_established(&self) -> bool {
        self.state == TcpState::Established
    }
    pub const fn peer_window(&self) -> u16 {
        self.peer_window
    }
    pub const fn set_receive_window(&mut self, window: u16) {
        self.our_window = window;
    }
    pub fn pending(&self) -> Option<PendingInfo<'_>> {
        let pending = self.pending.as_ref()?;
        Some(PendingInfo {
            seq: pending.seq,
            acknowledgment: self.rcv_nxt,
            flags: pending.flags,
            window: self.our_window,
            payload: &self.tx[..pending.payload_len],
        })
    }
    pub fn acknowledge_pending(&mut self, now_ms: u64) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if pending.consumed() == 0 {
            return;
        }
        self.flight = Some(Flight::from_pending(&pending));
        self.rto_deadline = now_ms.saturating_add(self.rto_ms());
    }
    pub fn send(&mut self, data: &[u8], now_ms: u64) -> Result<usize, TcpError> {
        if self.dead || self.close_requested {
            return Err(TcpError::Closed);
        }
        if !matches!(self.state, TcpState::Established | TcpState::CloseWait) {
            return Err(TcpError::NotConnected);
        }
        if self.pending.is_some() || self.flight.is_some() {
            return Err(TcpError::WouldBlock);
        }
        if data.is_empty() {
            return Ok(0);
        }
        let allowed = data.len().min(TCP_MSS).min(self.peer_window as usize);
        if allowed == 0 {
            return Err(TcpError::WouldBlock);
        }
        self.tx[..allowed].copy_from_slice(&data[..allowed]);
        self.tx_len = allowed;
        let seq = self.snd_nxt;
        self.snd_nxt = self.snd_nxt.wrapping_add(allowed as u32);
        self.queue_pending(
            Pending {
                seq,
                flags: TCP_FLAG_PSH | TCP_FLAG_ACK,
                payload_len: allowed,
            },
            now_ms,
        );
        Ok(allowed)
    }
    pub fn close(&mut self, now_ms: u64) -> Result<(), TcpError> {
        if self.dead {
            return Err(TcpError::Closed);
        }
        match self.state {
            TcpState::SynSent | TcpState::SynReceived => {
                self.pending = None;
                self.flight = None;
                self.dead = true;
                Ok(())
            }
            TcpState::Established | TcpState::CloseWait => {
                if self.close_requested {
                    return Ok(());
                }
                self.close_requested = true;
                self.state = if self.state == TcpState::Established {
                    TcpState::FinWait1
                } else {
                    TcpState::LastAck
                };
                if self.pending.is_none() && self.flight.is_none() {
                    self.queue_fin(now_ms);
                }
                Ok(())
            }
            _ => Err(TcpError::Closed),
        }
    }
    pub fn on_segment<'s>(&mut self, segment: &'s TcpSegment<'_>, now_ms: u64) -> TcpInput<'s> {
        if self.dead {
            return TcpInput {
                received: None,
                event: TcpEvent::None,
            };
        }
        if segment.flags & TCP_FLAG_RST != 0 {
            self.pending = None;
            self.flight = None;
            self.dead = true;
            let event = if self.state == TcpState::SynSent && self.active_open {
                TcpEvent::Refused
            } else {
                TcpEvent::Reset
            };
            return TcpInput {
                received: None,
                event,
            };
        }
        let mut event = TcpEvent::None;
        if segment.flags & TCP_FLAG_ACK != 0 {
            if seq_gt(segment.acknowledgment, self.snd_nxt) {
                return TcpInput {
                    received: None,
                    event: TcpEvent::None,
                };
            }
            if seq_gt(segment.acknowledgment, self.snd_una) {
                self.snd_una = segment.acknowledgment;
                event = self.on_acknowledgment(segment.acknowledgment, now_ms);
            }
        }
        match self.state {
            TcpState::SynSent => {
                if segment.flags & TCP_FLAG_SYN != 0
                    && segment.flags & TCP_FLAG_ACK != 0
                    && segment.acknowledgment == self.iss.wrapping_add(1)
                {
                    self.rcv_nxt = segment.sequence.wrapping_add(1);
                    self.peer_window = segment.window;
                    self.state = TcpState::Established;
                    self.flight = None;
                    self.retries = 0;
                    self.backoff = 0;
                    self.supersede_syn_pending();
                    self.queue_ack(now_ms);
                    return TcpInput {
                        received: None,
                        event: TcpEvent::Established,
                    };
                }
                TcpInput {
                    received: None,
                    event,
                }
            }
            TcpState::SynReceived => {
                if segment.flags & TCP_FLAG_ACK != 0
                    && segment.acknowledgment == self.iss.wrapping_add(1)
                {
                    self.state = TcpState::Established;
                    self.flight = None;
                    self.retries = 0;
                    self.backoff = 0;
                    self.peer_window = segment.window;
                    self.supersede_syn_pending();
                    return TcpInput {
                        received: None,
                        event: TcpEvent::Established,
                    };
                }
                TcpInput {
                    received: None,
                    event,
                }
            }
            _ => self.on_established_segment(segment, now_ms, event),
        }
    }
    pub fn tick(&mut self, now_ms: u64) -> Option<TcpEvent> {
        if self.dead {
            return None;
        }
        if self.state == TcpState::TimeWait && now_ms >= self.timewait_deadline {
            self.dead = true;
            return None;
        }
        if self.pending.is_some() {
            if self.flight.is_none()
                && now_ms >= self.pending_since.saturating_add(PENDING_TX_TIMEOUT_MS)
            {
                self.dead = true;
                return Some(TcpEvent::TimedOut);
            }
            return None;
        }
        if let Some(flight) = self.flight
            && now_ms >= self.rto_deadline
        {
            if self.retries >= MAX_RETRIES {
                self.dead = true;
                return Some(TcpEvent::TimedOut);
            }
            self.retries += 1;
            self.backoff = (self.backoff + 1).min(3);
            self.rto_deadline = now_ms.saturating_add(self.rto_ms());
            self.queue_pending(
                Pending {
                    seq: flight.seq,
                    flags: flight.flags,
                    payload_len: flight.payload_len,
                },
                now_ms,
            );
        }
        None
    }
    fn rto_ms(&self) -> u64 {
        (RTO_BASE_MS << self.backoff.min(3)).min(RTO_CAP_MS)
    }
    const fn queue_pending(&mut self, pending: Pending, now_ms: u64) {
        self.pending = Some(pending);
        self.pending_since = now_ms;
    }
    fn queue_ack(&mut self, now_ms: u64) {
        if self.pending.is_some() {
            return;
        }
        self.queue_pending(
            Pending {
                seq: self.snd_nxt,
                flags: TCP_FLAG_ACK,
                payload_len: 0,
            },
            now_ms,
        );
    }
    const fn supersede_syn_pending(&mut self) {
        if let Some(pending) = self.pending
            && pending.flags & TCP_FLAG_SYN != 0
        {
            self.pending = None;
        }
    }
    fn queue_fin(&mut self, now_ms: u64) {
        if self.fin_sent {
            return;
        }
        let seq = self.snd_nxt;
        self.snd_nxt = self.snd_nxt.wrapping_add(1);
        self.fin_sent = true;
        self.queue_pending(
            Pending {
                seq,
                flags: TCP_FLAG_FIN | TCP_FLAG_ACK,
                payload_len: 0,
            },
            now_ms,
        );
    }
    fn on_acknowledgment(&mut self, ack: u32, now_ms: u64) -> TcpEvent {
        let mut event = TcpEvent::None;
        if let Some(mut flight) = self.flight {
            if !seq_lt(ack, flight.end) {
                self.flight = None;
                self.tx_len = 0;
                self.retries = 0;
                self.backoff = 0;
            } else if seq_gt(ack, flight.seq) {
                let data_start = flight.data_start();
                if seq_gt(ack, data_start) && flight.payload_len > 0 {
                    let trim = ack.wrapping_sub(data_start) as usize;
                    let trimmed = trim.min(flight.payload_len);
                    if trimmed > 0 && trimmed < self.tx_len {
                        self.tx.copy_within(trimmed..self.tx_len, 0);
                        self.tx_len -= trimmed;
                        flight.payload_len -= trimmed;
                        flight.end = flight.end.wrapping_sub(trimmed as u32);
                    }
                }
                flight.seq = ack;
                self.flight = Some(flight);
            }
        }
        if self.close_requested
            && !self.fin_sent
            && self.flight.is_none()
            && self.pending.is_none()
            && matches!(self.state, TcpState::FinWait1 | TcpState::LastAck)
        {
            self.queue_fin(now_ms);
        }
        match self.state {
            TcpState::FinWait1 if self.flight.is_none() && self.fin_sent => {
                self.state = TcpState::FinWait2;
            }
            TcpState::Closing if self.flight.is_none() && self.fin_sent => {
                self.enter_time_wait(now_ms);
                event = TcpEvent::Closed;
            }
            TcpState::LastAck if self.flight.is_none() && self.fin_sent => {
                self.dead = true;
                event = TcpEvent::Closed;
            }
            _ => {}
        }
        event
    }
    fn enter_time_wait(&mut self, now_ms: u64) {
        self.state = TcpState::TimeWait;
        self.timewait_deadline = now_ms.saturating_add(TIME_WAIT_MS);
        self.queue_ack(now_ms);
    }
    fn on_established_segment<'s>(
        &mut self,
        segment: &'s TcpSegment<'_>,
        now_ms: u64,
        mut event: TcpEvent,
    ) -> TcpInput<'s> {
        let mut received = None;
        let mut needs_ack = false;
        if !segment.payload.is_empty() {
            if segment.sequence == self.rcv_nxt
                && matches!(
                    self.state,
                    TcpState::Established
                        | TcpState::FinWait1
                        | TcpState::FinWait2
                        | TcpState::CloseWait
                )
            {
                self.rcv_nxt = self.rcv_nxt.wrapping_add(segment.payload.len() as u32);
                received = Some(segment.payload);
            }
            needs_ack = true;
        }
        self.peer_window = segment.window;
        if segment.flags & TCP_FLAG_FIN != 0 {
            let fin_sequence = segment.sequence.wrapping_add(segment.payload.len() as u32);
            if fin_sequence == self.rcv_nxt {
                self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
                needs_ack = true;
                match self.state {
                    TcpState::Established | TcpState::CloseWait => {
                        self.state = TcpState::CloseWait;
                    }
                    TcpState::FinWait1 => {
                        if self.flight.is_none() && self.fin_sent {
                            self.enter_time_wait(now_ms);
                            event = TcpEvent::Closed;
                        } else {
                            self.state = TcpState::Closing;
                        }
                    }
                    TcpState::FinWait2 => {
                        self.enter_time_wait(now_ms);
                        event = TcpEvent::Closed;
                    }
                    TcpState::TimeWait => {
                        self.timewait_deadline = now_ms.saturating_add(TIME_WAIT_MS);
                    }
                    _ => {}
                }
            } else {
                needs_ack = true;
            }
        }
        if needs_ack {
            self.queue_ack(now_ms);
        }
        TcpInput { received, event }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    const LOCAL: u16 = 50000;
    const REMOTE_PORT: u16 = 80;
    fn remote() -> Ipv4Address {
        Ipv4Address([10, 0, 2, 2])
    }
    fn segment<'a>(
        sequence: u32,
        acknowledgment: u32,
        flags: u16,
        payload: &'a [u8],
    ) -> TcpSegment<'a> {
        TcpSegment {
            source_port: REMOTE_PORT,
            destination_port: LOCAL,
            sequence,
            acknowledgment,
            flags,
            window: 4096,
            payload,
        }
    }
    fn drain(connection: &mut TcpConnection, out: &mut Vec<(u32, u32, u16)>) {
        while let Some(info) = connection.pending() {
            out.push((info.seq, info.acknowledgment, info.flags));
            connection.acknowledge_pending(0);
        }
    }
    #[test]
    fn active_open_completes_handshake_on_syn_ack() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let mut sent = Vec::new();
        drain(&mut connection, &mut sent);
        assert_eq!(sent, alloc::vec![(1000, 0, TCP_FLAG_SYN)]);
        let response = segment(5000, 1001, TCP_FLAG_SYN | TCP_FLAG_ACK, &[]);
        let outcome = connection.on_segment(&response, 0);
        assert_eq!(outcome.event, TcpEvent::Established);
        assert_eq!(connection.state(), TcpState::Established);
        sent.clear();
        drain(&mut connection, &mut sent);
        assert_eq!(sent, alloc::vec![(1001, 5001, TCP_FLAG_ACK)]);
    }
    #[test]
    fn syn_ack_with_wrong_ack_is_ignored() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let response = segment(5000, 999, TCP_FLAG_SYN | TCP_FLAG_ACK, &[]);
        let outcome = connection.on_segment(&response, 0);
        assert_eq!(outcome.event, TcpEvent::None);
        assert_eq!(connection.state(), TcpState::SynSent);
    }
    #[test]
    fn passive_open_acknowledges_syn_and_establishes_on_ack() {
        let mut connection =
            TcpConnection::passive_open(LOCAL, remote(), REMOTE_PORT, 700, 4000, 4096, 0);
        let mut sent = Vec::new();
        drain(&mut connection, &mut sent);
        assert_eq!(sent, alloc::vec![(700, 4001, TCP_FLAG_SYN | TCP_FLAG_ACK)]);
        let response = segment(4001, 701, TCP_FLAG_ACK, &[]);
        let outcome = connection.on_segment(&response, 0);
        assert_eq!(outcome.event, TcpEvent::Established);
        assert!(connection.is_established());
    }
    #[test]
    fn in_order_data_is_delivered_and_acknowledged() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let response = segment(5000, 1001, TCP_FLAG_SYN | TCP_FLAG_ACK, &[]);
        connection.on_segment(&response, 0);
        connection.acknowledge_pending(0);
        let data = segment(5001, 1001, TCP_FLAG_ACK | TCP_FLAG_PSH, b"hola");
        let outcome = connection.on_segment(&data, 0);
        assert_eq!(outcome.received, Some(&b"hola"[..]));
        let mut sent = Vec::new();
        drain(&mut connection, &mut sent);
        assert_eq!(sent.last(), Some(&(1001, 5005, TCP_FLAG_ACK)));
    }
    #[test]
    fn out_of_order_data_is_dropped_and_duplicate_ack_sent() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let response = segment(5000, 1001, TCP_FLAG_SYN | TCP_FLAG_ACK, &[]);
        connection.on_segment(&response, 0);
        connection.acknowledge_pending(0);
        let data = segment(5010, 1001, TCP_FLAG_ACK, b"lejos");
        let outcome = connection.on_segment(&data, 0);
        assert_eq!(outcome.received, None);
        let mut sent = Vec::new();
        drain(&mut connection, &mut sent);
        assert_eq!(sent.last(), Some(&(1001, 5001, TCP_FLAG_ACK)));
    }
    #[test]
    fn unacknowledged_segment_is_retransmitted_with_backoff() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        assert_eq!(connection.tick(10), None);
        assert_eq!(connection.pending().map(|info| info.seq), Some(1000));
        connection.acknowledge_pending(0);
        assert_eq!(connection.tick(499), None);
        assert_eq!(connection.tick(500), None);
        let retransmit = connection.pending().expect("retransmit pendiente");
        assert_eq!(retransmit.seq, 1000);
        assert_eq!(retransmit.flags, TCP_FLAG_SYN);
        connection.acknowledge_pending(500);
        assert_eq!(connection.tick(1499), None);
        assert_eq!(connection.tick(1500), None);
        assert!(connection.pending().is_some());
    }
    #[test]
    fn handshake_times_out_after_max_retries() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        connection.acknowledge_pending(0);
        let mut now = 0u64;
        let mut timed_out = None;
        for _ in 0..128 {
            now += 400;
            if let Some(event) = connection.tick(now) {
                timed_out = Some(event);
                break;
            }
            if connection.pending().is_some() {
                connection.acknowledge_pending(now);
            }
        }
        assert_eq!(timed_out, Some(TcpEvent::TimedOut));
        assert!(connection.is_dead());
    }
    #[test]
    fn pending_segment_that_never_transmits_times_out() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        assert_eq!(connection.tick(9_999), None);
        assert_eq!(connection.tick(10_000), Some(TcpEvent::TimedOut));
        assert!(connection.is_dead());
    }
    #[test]
    fn reset_during_connect_reports_refused() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let response = segment(0, 0, TCP_FLAG_RST, &[]);
        let outcome = connection.on_segment(&response, 0);
        assert_eq!(outcome.event, TcpEvent::Refused);
        assert!(connection.is_dead());
    }
    #[test]
    fn established_connection_closes_with_fin_handshake() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let syn_ack = segment(5000, 1001, TCP_FLAG_SYN | TCP_FLAG_ACK, &[]);
        connection.on_segment(&syn_ack, 0);
        connection.acknowledge_pending(0);
        connection.close(0).expect("cierre fallido");
        assert_eq!(connection.state(), TcpState::FinWait1);
        let mut sent = Vec::new();
        drain(&mut connection, &mut sent);
        assert_eq!(
            sent.last(),
            Some(&(1001, 5001, TCP_FLAG_FIN | TCP_FLAG_ACK))
        );
        let peer_fin = segment(5001, 1002, TCP_FLAG_ACK | TCP_FLAG_FIN, &[]);
        let outcome = connection.on_segment(&peer_fin, 10);
        assert_eq!(outcome.event, TcpEvent::Closed);
        assert_eq!(connection.state(), TcpState::TimeWait);
        assert_eq!(connection.tick(2000), None);
        assert!(!connection.is_dead());
        assert_eq!(connection.tick(2020), None);
        assert!(connection.is_dead());
    }
    #[test]
    fn server_side_close_after_peer_fin_uses_last_ack() {
        let mut connection =
            TcpConnection::passive_open(LOCAL, remote(), REMOTE_PORT, 700, 4000, 4096, 0);
        connection.acknowledge_pending(0);
        let ack = segment(4001, 701, TCP_FLAG_ACK, &[]);
        let outcome = connection.on_segment(&ack, 0);
        assert_eq!(outcome.event, TcpEvent::Established);
        let peer_fin = segment(4001, 701, TCP_FLAG_ACK | TCP_FLAG_FIN, &[]);
        let outcome = connection.on_segment(&peer_fin, 0);
        assert_eq!(outcome.event, TcpEvent::None);
        assert_eq!(connection.state(), TcpState::CloseWait);
        connection.acknowledge_pending(0);
        connection.close(0).expect("cierre fallido");
        assert_eq!(connection.state(), TcpState::LastAck);
        let mut sent = Vec::new();
        drain(&mut connection, &mut sent);
        assert_eq!(sent.last(), Some(&(701, 4002, TCP_FLAG_FIN | TCP_FLAG_ACK)));
        let final_ack = segment(4002, 702, TCP_FLAG_ACK, &[]);
        let outcome = connection.on_segment(&final_ack, 0);
        assert_eq!(outcome.event, TcpEvent::Closed);
        assert!(connection.is_dead());
    }
    #[test]
    fn send_is_limited_to_one_segment_in_flight() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let syn_ack = segment(5000, 1001, TCP_FLAG_SYN | TCP_FLAG_ACK, &[]);
        connection.on_segment(&syn_ack, 0);
        connection.acknowledge_pending(0);
        let written = connection.send(b"abc", 0).expect("envio fallido");
        assert_eq!(written, 3);
        assert_eq!(connection.send(b"def", 0), Err(TcpError::WouldBlock));
        connection.acknowledge_pending(0);
        assert_eq!(connection.send(b"def", 0), Err(TcpError::WouldBlock));
        let ack = segment(5001, 1004, TCP_FLAG_ACK, &[]);
        connection.on_segment(&ack, 0);
        assert_eq!(connection.send(b"def", 0).expect("envio fallido"), 3);
    }
    #[test]
    fn send_respects_peer_window() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        let mut syn_ack = segment(5000, 1001, TCP_FLAG_SYN | TCP_FLAG_ACK, &[]);
        syn_ack.window = 2;
        connection.on_segment(&syn_ack, 0);
        connection.acknowledge_pending(0);
        assert_eq!(connection.send(b"abc", 0).expect("envio fallido"), 2);
    }
    #[test]
    fn data_arriving_with_fin_is_delivered_before_close() {
        let mut connection =
            TcpConnection::passive_open(LOCAL, remote(), REMOTE_PORT, 700, 4000, 4096, 0);
        connection.acknowledge_pending(0);
        let ack = segment(4001, 701, TCP_FLAG_ACK, &[]);
        connection.on_segment(&ack, 0);
        let data_fin = segment(4001, 701, TCP_FLAG_ACK | TCP_FLAG_FIN, b"chau");
        let outcome = connection.on_segment(&data_fin, 0);
        assert_eq!(outcome.received, Some(&b"chau"[..]));
        assert_eq!(connection.state(), TcpState::CloseWait);
    }
    #[test]
    fn close_during_handshake_aborts_connection() {
        let mut connection = TcpConnection::active_open(LOCAL, remote(), REMOTE_PORT, 1000, 0);
        connection.close(0).expect("abortar fallido");
        assert!(connection.is_dead());
        assert_eq!(connection.send(b"x", 0), Err(TcpError::Closed));
    }
}

#![no_std]
mod arp;
mod checksum;
mod dhcp;
mod dns;
mod ethernet;
mod ipv4;
mod routing;
mod tcp;
mod tcp_state;
mod udp;
#[cfg(test)]
extern crate alloc;
pub use arp::{ArpPacket, build_arp_reply, build_arp_request};
pub use checksum::ones_complement_sum;
pub use dhcp::{
    DhcpMessage, DhcpMessageType, build_dhcp_discover, build_dhcp_discover_frame,
    build_dhcp_request_frame,
};
pub use dns::{
    DnsAResponse, MAX_DNS_NAME, build_dns_a_query, parse_dns_a_response, valid_dns_name,
};
pub use ethernet::{EthernetFrame, MacAddress};
pub use ipv4::{Ipv4Address, Ipv4Packet};
pub use routing::{IPV4_LIMITED_BROADCAST, Route, decide_route, is_broadcast, subnet_broadcast};
pub use tcp::{
    TCP_FLAG_ACK, TCP_FLAG_FIN, TCP_FLAG_PSH, TCP_FLAG_RST, TCP_FLAG_SYN, TCP_FLAG_URG,
    TCP_HEADER_LENGTH, TCP_MSS, TcpSegment, build_ipv4_tcp_frame, build_tcp_segment,
};
pub use tcp_state::{PendingInfo, TcpConnection, TcpError, TcpEvent, TcpInput, TcpState};
pub use udp::{UdpDatagram, build_ipv4_udp_frame};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ethernet_frame_and_rejects_short_input() {
        let frame = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0x08, 0x00, 99];
        let parsed = EthernetFrame::parse(&frame).unwrap();
        assert_eq!(parsed.ether_type, 0x0800);
        assert_eq!(parsed.payload, &[99]);
        assert!(EthernetFrame::parse(&frame[..13]).is_none());
    }
    #[test]
    fn parses_ipv4_with_valid_checksum_and_rejects_corruption() {
        let packet = [
            0x45, 0, 0, 20, 0, 0, 0, 0, 64, 17, 0xf6, 0xd5, 192, 0, 2, 1, 192, 0, 2, 2,
        ];
        let parsed = Ipv4Packet::parse(&packet).unwrap();
        assert_eq!(parsed.protocol, 17);
        let mut corrupted = packet;
        corrupted[8] = 63;
        assert!(Ipv4Packet::parse(&corrupted).is_none());
    }
    #[test]
    fn udp_lengths_are_bounded_to_datagram() {
        let packet = [0, 53, 0x12, 0x34, 0, 10, 0, 0, 1, 2, 99];
        let parsed = UdpDatagram::parse(&packet).unwrap();
        assert_eq!(parsed.destination_port, 0x1234);
        assert_eq!(parsed.payload, &[1, 2]);
        assert!(UdpDatagram::parse(&packet[..9]).is_none());
    }
    #[test]
    fn builds_discover_and_parses_offer_options() {
        let mac = MacAddress([2, 0, 0, 0, 0, 1]);
        let mut request = [0u8; 300];
        let length = build_dhcp_discover(0x1234_5678, mac, &mut request).unwrap();
        assert_eq!(length, 259);
        assert_eq!(request[0], 1);
        assert_eq!(&request[4..8], &[0x12, 0x34, 0x56, 0x78]);
        assert!(build_dhcp_discover(1, mac, &mut [0; 250]).is_none());
        let mut offer = [0u8; 250];
        offer[0] = 2;
        offer[1] = 1;
        offer[2] = 6;
        offer[4..8].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        offer[16..20].copy_from_slice(&[10, 0, 2, 15]);
        offer[28..34].copy_from_slice(&mac.0);
        offer[236..240].copy_from_slice(&[99, 130, 83, 99]);
        offer[240..250].copy_from_slice(&[53, 1, 2, 54, 4, 10, 0, 2, 2, 255]);
        let parsed = DhcpMessage::parse(&offer).unwrap();
        assert_eq!(parsed.message_type, Some(DhcpMessageType::Offer));
        assert_eq!(parsed.server_id, Some(Ipv4Address([10, 0, 2, 2])));
        assert_eq!(parsed.offered_address, Ipv4Address([10, 0, 2, 15]));
    }
    #[test]
    fn rejects_dhcp_options_without_terminator_or_with_overrun() {
        let mut packet = [0u8; 243];
        packet[1] = 1;
        packet[2] = 6;
        packet[236..240].copy_from_slice(&[99, 130, 83, 99]);
        packet[240..].copy_from_slice(&[53, 4, 1]);
        assert!(DhcpMessage::parse(&packet).is_none());
    }
    #[test]
    fn builds_broadcast_discover_with_valid_ipv4_header() {
        let mac = MacAddress([2, 1, 2, 3, 4, 5]);
        let mut frame = [0u8; 320];
        let length = build_dhcp_discover_frame(7, mac, &mut frame).unwrap();
        assert_eq!(length, 301);
        assert_eq!(&frame[..6], &[255; 6]);
        assert_eq!(ones_complement_sum([&frame[14..34]]), 0);
        assert_eq!(&frame[26..30], &[0, 0, 0, 0]);
        assert_eq!(&frame[34..38], &[0, 68, 0, 67]);
        assert!(build_dhcp_discover_frame(7, mac, &mut [0; 300]).is_none());
    }
    #[test]
    fn builds_dhcp_request_selecting_offer() {
        let mac = MacAddress([2, 1, 2, 3, 4, 5]);
        let requested = Ipv4Address([10, 0, 2, 15]);
        let server = Ipv4Address([10, 0, 2, 2]);
        let mut frame = [0u8; 320];
        let length = build_dhcp_request_frame(9, mac, requested, server, &mut frame).unwrap();
        assert_eq!(length, 304);
        assert_eq!(frame[282], 53);
        assert_eq!(frame[284], DhcpMessageType::Request as u8);
        assert_eq!(&frame[287..291], &requested.0);
        assert_eq!(&frame[293..297], &server.0);
        assert_eq!(ones_complement_sum([&frame[14..34]]), 0);
    }
    #[test]
    fn udp_checksum_is_validated_on_ipv4_paths() {
        let mac = MacAddress([2, 1, 2, 3, 4, 5]);
        let peer = MacAddress([2, 1, 2, 3, 4, 6]);
        let source = Ipv4Address([10, 0, 2, 15]);
        let destination = Ipv4Address([10, 0, 2, 2]);
        let mut frame = [0u8; 128];
        let length = build_ipv4_udp_frame(
            mac,
            peer,
            source,
            destination,
            5000,
            53,
            b"hola",
            7,
            64,
            &mut frame,
        )
        .unwrap();
        let ip = Ipv4Packet::parse(&frame[14..length]).unwrap();
        assert!(UdpDatagram::parse_ipv4(ip.payload, ip.source, ip.destination).is_some());
        let mut corrupted = frame;
        corrupted[14 + 20 + 8] ^= 0xff;
        let ip = Ipv4Packet::parse(&corrupted[14..length]).unwrap();
        assert!(UdpDatagram::parse_ipv4(ip.payload, ip.source, ip.destination).is_none());
    }
    #[test]
    fn udp_length_field_must_match_datagram_size() {
        let source = Ipv4Address([1, 2, 3, 4]);
        let destination = Ipv4Address([5, 6, 7, 8]);
        let mut datagram = [0u8; 16];
        datagram[0..2].copy_from_slice(&1u16.to_be_bytes());
        datagram[2..4].copy_from_slice(&2u16.to_be_bytes());
        datagram[4..6].copy_from_slice(&8u16.to_be_bytes());
        assert!(UdpDatagram::parse_ipv4(&datagram, source, destination).is_none());
        datagram[4..6].copy_from_slice(&16u16.to_be_bytes());
        assert!(UdpDatagram::parse_ipv4(&datagram, source, destination).is_some());
    }
    #[test]
    fn arp_builders_reject_short_output_buffers() {
        let mac = MacAddress([2, 1, 2, 3, 4, 5]);
        let address = Ipv4Address([10, 0, 2, 15]);
        assert!(build_arp_request(mac, address, address, &mut [0u8; 41]).is_none());
        let mut request = [0u8; 42];
        let length = build_arp_request(mac, address, address, &mut request).unwrap();
        assert_eq!(length, 42);
        let parsed = ArpPacket::parse(&request[14..length]).unwrap();
        assert_eq!(parsed.operation, 1);
        assert_eq!(parsed.sender_ipv4, address);
        let reply = build_arp_reply(mac, address, &parsed, &mut [0u8; 41]);
        assert!(reply.is_none());
    }
    #[test]
    fn tcp_handshake_round_trip_over_builders() {
        let mac = MacAddress([2, 1, 2, 3, 4, 5]);
        let peer = MacAddress([2, 1, 2, 3, 4, 6]);
        let source = Ipv4Address([10, 0, 2, 15]);
        let destination = Ipv4Address([10, 0, 2, 2]);
        let mut connection = TcpConnection::active_open(50000, destination, 80, 1000, 0);
        let mut frame = [0u8; 128];
        let info = connection.pending().expect("syn pendiente");
        let length = build_ipv4_tcp_frame(
            mac,
            peer,
            source,
            destination,
            50000,
            80,
            info.seq,
            info.acknowledgment,
            info.flags,
            info.window,
            info.payload,
            1,
            64,
            &mut frame,
        )
        .unwrap();
        connection.acknowledge_pending(0);
        let sent = TcpSegment::parse_ipv4(&frame[34..length], source, destination).unwrap();
        assert_eq!(sent.flags, TCP_FLAG_SYN);
        let mut response = [0u8; 40];
        let response_len = build_tcp_segment(
            destination,
            source,
            80,
            50000,
            7000,
            1001,
            TCP_FLAG_SYN | TCP_FLAG_ACK,
            4096,
            &[],
            &mut response,
        )
        .unwrap();
        let parsed = TcpSegment::parse_ipv4(&response[..response_len], destination, source)
            .expect("syn-ack ilegible");
        let outcome = connection.on_segment(&parsed, 0);
        assert_eq!(outcome.event, TcpEvent::Established);
    }
}

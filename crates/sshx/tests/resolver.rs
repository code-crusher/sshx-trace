//! Integration tests for the client's DNS resolver fallback, which is used
//! for Android/Termux support (issue #141).

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::{A, AAAA};
use hickory_proto::rr::{RData, Record};
use hickory_proto::serialize::binary::{BinDecodable, BinEncodable, BinEncoder};
use hyper_util::client::legacy::connect::dns::Name;
use sshx::resolver::FallbackResolver;
use tokio::net::UdpSocket;
use tower_service::Service;

/// The `.test` TLD is reserved by RFC 2606 and can never resolve, so any
/// lookup for it through the system resolver is guaranteed to fail.
const UNRESOLVABLE_HOST: &str = "example.test";

/// Serve a single DNS query over UDP, replying with the given address.
async fn serve_once(socket: &UdpSocket, ip: IpAddr) {
    let mut buf = [0u8; 512];
    let (size, peer) = socket.recv_from(&mut buf).await.unwrap();

    let request = Message::from_bytes(&buf[..size]).unwrap();
    let query = request.queries()[0].clone();

    let mut response = Message::new();
    response.set_id(request.id());
    response.set_message_type(MessageType::Response);
    response.set_op_code(OpCode::Query);
    response.set_recursion_desired(true);
    response.set_response_code(ResponseCode::NoError);
    response.add_query(query.clone());
    let rdata = match ip {
        IpAddr::V4(ip) => RData::A(A(ip)),
        IpAddr::V6(ip) => RData::AAAA(AAAA(ip)),
    };
    response.add_answer(Record::from_rdata(query.name().to_owned(), 3600, rdata));

    let mut bytes = Vec::new();
    let mut encoder = BinEncoder::new(&mut bytes);
    response.emit(&mut encoder).unwrap();
    socket.send_to(&bytes, peer).await.unwrap();
}

#[tokio::test]
async fn fallback_resolver_queries_nameservers_on_system_failure() {
    const MOCK_ADDR: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 10));

    // Start a mock nameserver that answers a single query in the background.
    let socket = UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
    let nameserver = socket.local_addr().unwrap();
    tokio::spawn(async move {
        serve_once(&socket, MOCK_ADDR).await;
    });

    let mut resolver = FallbackResolver::with_nameservers(vec![nameserver]);
    let name = Name::from_str(UNRESOLVABLE_HOST).unwrap();
    let addrs: Vec<SocketAddr> = resolver.call(name).await.unwrap().collect();

    assert_eq!(addrs, vec![SocketAddr::new(MOCK_ADDR, 0)]);
}

#[tokio::test]
async fn fallback_resolver_reports_system_error_without_nameservers() {
    // With no fallback nameservers, a lookup failure from the system resolver
    // is reported to the caller.
    let mut resolver = FallbackResolver::with_nameservers(vec![]);
    let name = Name::from_str(UNRESOLVABLE_HOST).unwrap();
    assert!(resolver.call(name).await.is_err());
}

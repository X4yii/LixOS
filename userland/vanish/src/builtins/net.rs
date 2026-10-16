use alloc::string::String;
use sys_libc::syscalls::{
    socket_bind, socket_recvfrom, socket_sendto, socket_udp, sys_handle_close, sys_sleep,
    SocketAddrV4,
};
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut parts = s.split('.');
    let a = parts.next()?.parse::<u8>().ok()?;
    let b = parts.next()?.parse::<u8>().ok()?;
    let c = parts.next()?.parse::<u8>().ok()?;
    let d = parts.next()?.parse::<u8>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some([a, b, c, d])
}
fn parse_port(s: &str) -> Option<u16> {
    s.parse::<u16>().ok().filter(|&p| p > 0)
}
pub fn run(args: &[&str], _current_dir: &mut String) -> bool {
    let sub = args.first().copied().unwrap_or("help");
    match sub {
        "send" => {
            if args.len() < 4 {
                sys_libc::println!("Uso: net send <ip> <puerto> <mensaje>");
                return true;
            }
            let Some(ip) = parse_ipv4(args[1]) else {
                sys_libc::println!("[net] Error: Direccion IP invalida: '{}'", args[1]);
                return true;
            };
            let Some(port) = parse_port(args[2]) else {
                sys_libc::println!("[net] Error: Puerto invalido: '{}'", args[2]);
                return true;
            };
            let msg = args[3..].join(" ");
            let sock = match socket_udp() {
                Ok(s) => s,
                Err(e) => {
                    sys_libc::println!("[net] Error al abrir socket UDP (errno={})", e);
                    return true;
                }
            };
            let addr = SocketAddrV4 {
                family: 2,
                port_be: port.to_be(),
                address: ip,
                reserved: [0; 8],
            };
            match socket_sendto(sock, &addr, msg.as_bytes()) {
                Ok(bytes) => {
                    sys_libc::println!(
                        "[net] Transmitidos {} bytes a {}.{}.{}.{}:{}",
                        bytes,
                        ip[0],
                        ip[1],
                        ip[2],
                        ip[3],
                        port
                    );
                }
                Err(e) => {
                    sys_libc::println!("[net] Error enviando datagrama (errno={})", e);
                }
            }
            let _ = sys_handle_close(sock);
            true
        }
        "listen" => {
            let port = if args.len() >= 2 {
                match parse_port(args[1]) {
                    Some(p) => p,
                    None => {
                        sys_libc::println!("[net] Puerto invalido: '{}'", args[1]);
                        return true;
                    }
                }
            } else {
                8080
            };
            let sock = match socket_udp() {
                Ok(s) => s,
                Err(e) => {
                    sys_libc::println!("[net] Error abriendo socket (errno={})", e);
                    return true;
                }
            };
            if let Err(e) = socket_bind(sock, port) {
                sys_libc::println!("[net] Error en bind({}) (errno={})", port, e);
                let _ = sys_handle_close(sock);
                return true;
            }
            sys_libc::println!(
                "[net] Escuchando datagramas UDP en puerto {} (esperando hasta 5s)...",
                port
            );
            let mut buf = [0u8; 1500];
            let mut src_addr = SocketAddrV4::default();
            let mut received = false;
            for _ in 0..50 {
                match socket_recvfrom(sock, &mut buf, &mut src_addr) {
                    Ok(n) if n > 0 => {
                        let text = core::str::from_utf8(&buf[..n]).unwrap_or("<binario>");
                        let ip = src_addr.address;
                        let src_port = u16::from_be(src_addr.port_be);
                        sys_libc::println!(
                            "[net] Recibidos {} bytes de {}.{}.{}.{}:{}: {}",
                            n,
                            ip[0],
                            ip[1],
                            ip[2],
                            ip[3],
                            src_port,
                            text
                        );
                        received = true;
                        break;
                    }
                    _ => {
                        sys_sleep(100);
                    }
                }
            }
            if !received {
                sys_libc::println!("[net] Timeout: No se recibieron datagramas.");
            }
            let _ = sys_handle_close(sock);
            true
        }
        "ping" => {
            if args.len() < 2 {
                sys_libc::println!("Uso: net ping <ip>");
                return true;
            }
            let Some(ip) = parse_ipv4(args[1]) else {
                sys_libc::println!("[net] Error: Direccion IP invalida: '{}'", args[1]);
                return true;
            };
            let sock = match socket_udp() {
                Ok(s) => s,
                Err(e) => {
                    sys_libc::println!("[net] Error abriendo socket (errno={})", e);
                    return true;
                }
            };
            let addr = SocketAddrV4 {
                family: 2,
                port_be: 7u16.to_be(),
                address: ip,
                reserved: [0; 8],
            };
            let probe = b"LixOS UDP Probe";
            sys_libc::println!(
                "PING {}.{}.{}.{} con 15 bytes de datos UDP...",
                ip[0],
                ip[1],
                ip[2],
                ip[3]
            );
            match socket_sendto(sock, &addr, probe) {
                Ok(_) => {
                    let mut buf = [0u8; 128];
                    let mut reply_addr = SocketAddrV4::default();
                    let mut replied = false;
                    for i in 0..10 {
                        match socket_recvfrom(sock, &mut buf, &mut reply_addr) {
                            Ok(n) if n > 0 => {
                                sys_libc::println!(
                                    "15 bytes recibidos de {}.{}.{}.{}: rtt={}ms",
                                    ip[0],
                                    ip[1],
                                    ip[2],
                                    ip[3],
                                    i * 50
                                );
                                replied = true;
                                break;
                            }
                            _ => {
                                sys_sleep(50);
                            }
                        }
                    }
                    if !replied {
                        sys_libc::println!(
                            "Probe transmitido hacia {}.{}.{}.{} (sin respuesta UDP echo, normal si no hay echo server activo)",
                            ip[0],
                            ip[1],
                            ip[2],
                            ip[3]
                        );
                    }
                }
                Err(e) => {
                    sys_libc::println!("Error al transmitir sonda (errno={})", e);
                }
            }
            let _ = sys_handle_close(sock);
            true
        }
        _ => {
            sys_libc::println!("Comandos de red LixOS:");
            sys_libc::println!("  net send <ip> <puerto> <texto>  - Envia un datagrama UDP");
            sys_libc::println!(
                "  net listen [puerto]             - Escucha datagramas UDP (por defecto 8080)"
            );
            sys_libc::println!(
                "  net ping <ip>                   - Envia sonda de red UDP a una IP"
            );
            true
        }
    }
}

use crate::ipv4::Ipv4Address;
pub const IPV4_LIMITED_BROADCAST: Ipv4Address = Ipv4Address([255, 255, 255, 255]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    Broadcast,
    Direct,
    ViaGateway,
}
pub fn is_broadcast(destination: Ipv4Address) -> bool {
    destination == IPV4_LIMITED_BROADCAST
}
pub fn subnet_broadcast(address: Ipv4Address, mask: Ipv4Address) -> Ipv4Address {
    let mut broadcast = [0u8; 4];
    for index in 0..4 {
        broadcast[index] = address.0[index] | !mask.0[index];
    }
    Ipv4Address(broadcast)
}
pub fn same_subnet(address: Ipv4Address, mask: Ipv4Address, destination: Ipv4Address) -> bool {
    (0..4).all(|index| address.0[index] & mask.0[index] == destination.0[index] & mask.0[index])
}
pub fn decide_route(
    address: Ipv4Address,
    subnet_mask: Option<Ipv4Address>,
    gateway: Option<Ipv4Address>,
    destination: Ipv4Address,
) -> Option<(Route, Ipv4Address)> {
    if is_broadcast(destination) {
        return Some((Route::Broadcast, destination));
    }
    match subnet_mask {
        None => Some((Route::Direct, destination)),
        Some(mask) => {
            if !same_subnet(address, mask, destination) {
                gateway.map(|gateway| (Route::ViaGateway, gateway))
            } else if subnet_broadcast(address, mask) == destination {
                Some((Route::Broadcast, destination))
            } else {
                Some((Route::Direct, destination))
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn address() -> Ipv4Address {
        Ipv4Address([10, 0, 2, 15])
    }
    fn mask() -> Ipv4Address {
        Ipv4Address([255, 255, 255, 0])
    }
    fn gateway() -> Ipv4Address {
        Ipv4Address([10, 0, 2, 2])
    }
    #[test]
    fn limited_broadcast_needs_no_gateway() {
        assert_eq!(
            decide_route(address(), None, None, IPV4_LIMITED_BROADCAST),
            Some((Route::Broadcast, IPV4_LIMITED_BROADCAST))
        );
    }
    #[test]
    fn same_subnet_routes_directly() {
        assert_eq!(
            decide_route(
                address(),
                Some(mask()),
                Some(gateway()),
                Ipv4Address([10, 0, 2, 9])
            ),
            Some((Route::Direct, Ipv4Address([10, 0, 2, 9])))
        );
    }
    #[test]
    fn other_subnet_routes_via_gateway() {
        assert_eq!(
            decide_route(
                address(),
                Some(mask()),
                Some(gateway()),
                Ipv4Address([8, 8, 8, 8])
            ),
            Some((Route::ViaGateway, gateway()))
        );
    }
    #[test]
    fn other_subnet_without_gateway_has_no_route() {
        assert_eq!(
            decide_route(address(), Some(mask()), None, Ipv4Address([8, 8, 8, 8])),
            None
        );
    }
    #[test]
    fn subnet_broadcast_is_detected() {
        assert_eq!(
            decide_route(
                address(),
                Some(mask()),
                Some(gateway()),
                Ipv4Address([10, 0, 2, 255])
            ),
            Some((Route::Broadcast, Ipv4Address([10, 0, 2, 255])))
        );
    }
    #[test]
    fn without_mask_every_destination_is_direct() {
        assert_eq!(
            decide_route(address(), None, None, Ipv4Address([8, 8, 4, 4])),
            Some((Route::Direct, Ipv4Address([8, 8, 4, 4])))
        );
    }
}

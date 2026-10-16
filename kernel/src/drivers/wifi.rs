use crate::drivers::net::{NetworkDevice, NetworkError};
use crate::sync::IrqSafeSpinlock;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use lix_net::MacAddress;
use lix_wifi::ath9k_htc::{encode_tx_stream, Chip, HtcEndpointSet, RxStream};
use lix_wifi::data::{decapsulate_80211_to_ethernet, encapsulate_ethernet_to_80211, LLC_SNAP_HEADER};
use lix_wifi::htc::{encode_frame, parse_frame, HtcEngine, HtcEvent, WMI_CONTROL_SVC};
use lix_wifi::station::{
    build_association_request, build_authentication, build_probe_request, parse_association_response,
    parse_authentication,
};
use lix_wifi::wmi::WmiClient;
use lix_wifi::wpa2::{derive_psk, Handshake};
use lix_wifi::{Beacon, Bssid};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceState {
    Matched,
    WaitingForFirmware,
    FirmwareUnavailable,
    FirmwareLoading,
    HtcStarting,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StationState {
    Uninitialized,
    Ready,
    Scanning,
    Authenticating,
    Associating,
    Handshaking,
    Linked,
    Failed,
}

pub struct UsbWifiDevice {
    pub slot: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub chip: Chip,
    pub endpoints: HtcEndpointSet,
    pub state: DeviceState,
    pub station_state: StationState,
    pub mac_address: [u8; 6],
    pub bssid: [u8; 6],
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub psk: [u8; 32],
    pub channel: u8,
    pub htc: HtcEngine,
    pub wmi: WmiClient,
    pub handshake: Option<Handshake>,
    pub tk: Option<[u8; 16]>,
    pub tx_pn: u64,
    pub rx_pn: u64,
    pub tx_seq: u16,
    pub wmi_endpoint: u8,
    pub data_endpoint: u8,
}

static DEVICES: IrqSafeSpinlock<Vec<UsbWifiDevice>> = IrqSafeSpinlock::new(Vec::new());
static REGISTERED_NET_DEVICE: AtomicBool = AtomicBool::new(false);

pub struct WifiNetworkAdapter;

impl NetworkDevice for WifiNetworkAdapter {
    fn mac_address(&self) -> MacAddress {
        let devices = DEVICES.lock();
        let mac = devices
            .first()
            .map(|device| device.mac_address)
            .unwrap_or([0x00, 0x03, 0x7f, 0x12, 0x34, 0x56]);
        MacAddress(mac)
    }

    fn transmit(&self, frame: &[u8]) -> Result<(), NetworkError> {
        transmit_ethernet(frame)
    }
}

pub static WIFI_ADAPTER: WifiNetworkAdapter = WifiNetworkAdapter;

pub fn probe_usb(slot: u8, vendor: u16, product: u16, configuration: &[u8]) -> bool {
    let Some(chip) = lix_wifi::ath9k_htc::match_usb(vendor, product) else {
        return false;
    };
    let endpoints = match lix_wifi::ath9k_htc::find_htc_endpoints(configuration) {
        Ok(Some(endpoints)) => endpoints,
        Ok(None) | Err(_) => {
            crate::kprintln!(
                "[wifi] {:04x}:{:04x} coincide con ath9k_htc pero faltan endpoints HIF",
                vendor,
                product
            );
            return false;
        }
    };
    let state = match chip {
        Chip::Ar9271 => DeviceState::WaitingForFirmware,
        Chip::Ar7010Bridge => DeviceState::FirmwareUnavailable,
    };
    let mut mac = [0x00, 0x03, 0x7f, 0x00, 0x00, slot];
    let tsc = unsafe { core::arch::x86_64::_rdtsc() };
    mac[3] = (tsc & 0xff) as u8;
    mac[4] = ((tsc >> 8) & 0xff) as u8;

    let device = UsbWifiDevice {
        slot,
        vendor_id: vendor,
        product_id: product,
        chip,
        endpoints,
        state,
        station_state: StationState::Uninitialized,
        mac_address: mac,
        bssid: [0u8; 6],
        ssid: [0u8; 32],
        ssid_len: 0,
        psk: [0u8; 32],
        channel: 1,
        htc: HtcEngine::new(),
        wmi: WmiClient::new(1000),
        handshake: None,
        tk: None,
        tx_pn: 1,
        rx_pn: 0,
        tx_seq: 0,
        wmi_endpoint: 1,
        data_endpoint: 2,
    };
    let mut devices = DEVICES.lock();
    devices.retain(|existing| existing.slot != slot);
    devices.push(device);
    crate::kprintln!(
        "[wifi] ath9k_htc reconocido {:04x}:{:04x}, slot {}; endpoints WLAN bulk {:#04x}/{:#04x}, registro interrupt {:#04x}/{:#04x}; firmware pendiente",
        vendor,
        product,
        slot,
        endpoints.wlan_in.address,
        endpoints.wlan_out.address,
        endpoints.reg_in.address,
        endpoints.reg_out.address
    );
    true
}

pub fn remove_usb(slot: u8) {
    DEVICES.lock().retain(|device| device.slot != slot);
}

pub fn set_usb_state(slot: u8, state: DeviceState) {
    let mut devices = DEVICES.lock();
    if let Some(device) = devices.iter_mut().find(|device| device.slot == slot) {
        device.state = state;
        if state == DeviceState::HtcStarting {
            device.htc.begin(0, 5000);
        }
    }
}

pub fn usb_devices() -> Vec<UsbWifiDeviceSnapshot> {
    DEVICES
        .lock()
        .iter()
        .map(|device| UsbWifiDeviceSnapshot {
            slot: device.slot,
            vendor_id: device.vendor_id,
            product_id: device.product_id,
            chip: device.chip,
            endpoints: device.endpoints,
            state: device.state,
            station_state: device.station_state,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UsbWifiDeviceSnapshot {
    pub slot: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub chip: Chip,
    pub endpoints: HtcEndpointSet,
    pub state: DeviceState,
    pub station_state: StationState,
}

pub fn on_control_rx(slot: u8, data: &[u8]) {
    let mut send_buffer = [0u8; 512];
    let mut send_len = 0usize;
    let mut service_ready = false;

    {
        let mut devices = DEVICES.lock();
        let Some(device) = devices.iter_mut().find(|d| d.slot == slot) else {
            return;
        };

        if let Ok(Some(event)) = device.htc.on_control_message(data) {
            match event {
                HtcEvent::TargetReady { .. } => {
                    if let Ok(len) = device
                        .htc
                        .request_service(WMI_CONTROL_SVC, 0, 5000, &mut send_buffer)
                    {
                        send_len = len;
                    }
                }
                HtcEvent::ServiceConnected { endpoint_id, .. } => {
                    device.wmi_endpoint = endpoint_id;
                    if let Ok(len) = device.htc.setup_complete(&mut send_buffer) {
                        send_len = len;
                        let _ = device.htc.mark_running();
                        device.state = DeviceState::Ready;
                        device.station_state = StationState::Ready;
                        service_ready = true;
                    }
                }
                HtcEvent::ServiceFailed { .. } => {
                    device.state = DeviceState::Failed;
                    device.station_state = StationState::Failed;
                }
                HtcEvent::DataMessage { .. } => {}
            }
        }
    }

    if send_len > 0 {
        let mut htc_wrapped = [0u8; 512];
        if let Ok(wrapped_len) = encode_frame(0, 0, &send_buffer[..send_len], &mut htc_wrapped) {
            let _ = crate::drivers::xhci::wifi_send_control(slot, &htc_wrapped[..wrapped_len]);
        }
    }

    if service_ready {
        if !REGISTERED_NET_DEVICE.swap(true, Ordering::SeqCst) {
            crate::drivers::net::register(&WIFI_ADAPTER);
            crate::kprintln!(
                "[wifi] HTC y WMI inicializados en slot {}; interfaz wifi registrada en pila de red",
                slot
            );
        }
    }
}

pub fn on_wlan_rx(slot: u8, data: &[u8]) {
    let mut stream = RxStream::new(data);
    while let Some(Ok(rx_frame)) = stream.next() {
        let Ok(htc_frame) = parse_frame(rx_frame.bytes) else {
            continue;
        };
        let payload = htc_frame.payload;
        if payload.len() < 24 {
            continue;
        }

        let fc = u16::from_le_bytes([payload[0], payload[1]]);
        let frame_type = (fc >> 2) & 0x03;
        let frame_subtype = (fc >> 4) & 0x0f;

        if frame_type == 0 {
            handle_management_frame(slot, frame_subtype as u8, payload);
        } else if frame_type == 2 {
            handle_data_frame(slot, fc, payload);
        }
    }
}

fn handle_management_frame(slot: u8, subtype: u8, payload: &[u8]) {
    match subtype {
        8 | 5 => {
            if let Ok(beacon) = Beacon::parse(payload) {
                let mut should_auth = false;
                let mut bssid = [0u8; 6];
                let mut station_mac = [0u8; 6];
                let mut seq = 0u16;

                {
                    let mut devices = DEVICES.lock();
                    let Some(device) = devices.iter_mut().find(|d| d.slot == slot) else {
                        return;
                    };
                    if device.station_state == StationState::Scanning
                        && beacon.ssid == &device.ssid[..device.ssid_len]
                    {
                        device.bssid = beacon.bssid.0;
                        device.channel = beacon.channel.unwrap_or(1);
                        device.station_state = StationState::Authenticating;
                        bssid = device.bssid;
                        station_mac = device.mac_address;
                        seq = device.tx_seq;
                        device.tx_seq = device.tx_seq.wrapping_add(1);
                        should_auth = true;
                    }
                }

                if should_auth {
                    let mut auth_buf = [0u8; 256];
                    if let Ok(auth_len) =
                        build_authentication(&mut auth_buf, &station_mac, &bssid, seq)
                    {
                        send_mgmt_packet(slot, &auth_buf[..auth_len]);
                    }
                }
            }
        }
        11 => {
            let mut should_assoc = false;
            let mut bssid = [0u8; 6];
            let mut station_mac = [0u8; 6];
            let mut ssid = [0u8; 32];
            let mut ssid_len = 0usize;
            let mut channel = 1u8;
            let mut seq = 0u16;

            {
                let mut devices = DEVICES.lock();
                let Some(device) = devices.iter_mut().find(|d| d.slot == slot) else {
                    return;
                };
                if device.station_state == StationState::Authenticating
                    && parse_authentication(payload, &Bssid(device.bssid), &device.mac_address)
                        .is_ok()
                {
                    device.station_state = StationState::Associating;
                    bssid = device.bssid;
                    station_mac = device.mac_address;
                    ssid = device.ssid;
                    ssid_len = device.ssid_len;
                    channel = device.channel;
                    seq = device.tx_seq;
                    device.tx_seq = device.tx_seq.wrapping_add(1);
                    should_assoc = true;
                }
            }

            if should_assoc {
                let mut assoc_buf = [0u8; 256];
                if let Ok(assoc_len) = build_association_request(
                    &mut assoc_buf,
                    &station_mac,
                    &bssid,
                    &ssid[..ssid_len],
                    Some(channel),
                    true,
                    seq,
                ) {
                    send_mgmt_packet(slot, &assoc_buf[..assoc_len]);
                }
            }
        }
        1 => {
            let mut devices = DEVICES.lock();
            let Some(device) = devices.iter_mut().find(|d| d.slot == slot) else {
                return;
            };
            if device.station_state == StationState::Associating
                && parse_association_response(payload, &Bssid(device.bssid), &device.mac_address)
                    .is_ok()
            {
                device.station_state = StationState::Handshaking;
                device.handshake = Some(Handshake::new(
                    device.psk,
                    device.bssid,
                    device.mac_address,
                ));
                crate::kprintln!(
                    "[wifi] Asociado con BSSID {:02x?}; esperando 4-way handshake",
                    device.bssid
                );
            }
        }
        _ => {}
    }
}

fn handle_data_frame(slot: u8, fc: u16, payload: &[u8]) {
    let protected = (fc & (1 << 14)) != 0;

    if protected {
        let mut eth_buf = [0u8; 2048];
        let mut eth_len = 0usize;

        {
            let mut devices = DEVICES.lock();
            let Some(device) = devices.iter_mut().find(|d| d.slot == slot) else {
                return;
            };
            let Some(tk) = device.tk else {
                return;
            };

            if let Ok(len) = decapsulate_80211_to_ethernet(
                payload,
                &device.mac_address,
                &device.bssid,
                &tk,
                &mut device.rx_pn,
                &mut eth_buf,
            ) {
                eth_len = len;
            }
        }

        if eth_len >= 14 {
            let frame = &eth_buf[..eth_len];
            if frame[12..14] == [0x88, 0x8e] {
                handle_eapol(slot, &frame[14..]);
            } else {
                let _ = crate::drivers::net::receive(frame);
            }
        }
    } else {
        let is_qos = ((fc >> 4) & 0x0f) == 8;
        let header_len = if is_qos { 26 } else { 24 };
        if payload.len() >= header_len + 8 {
            let snap = &payload[header_len..header_len + 8];
            if snap[..6] == LLC_SNAP_HEADER && snap[6..8] == [0x88, 0x8e] {
                let eapol = &payload[header_len + 8..];
                handle_eapol(slot, eapol);
            }
        }
    }
}

fn handle_eapol(slot: u8, eapol_frame: &[u8]) {
    let mut response_data = None;
    let mut handshake_complete = false;

    {
        let mut devices = DEVICES.lock();
        let Some(device) = devices.iter_mut().find(|d| d.slot == slot) else {
            return;
        };
        let Some(handshake) = device.handshake.as_mut() else {
            return;
        };

        let tsc = unsafe { core::arch::x86_64::_rdtsc() };
        let mut snonce = [0u8; 32];
        for (i, byte) in snonce.iter_mut().enumerate() {
            *byte = ((tsc >> (i % 8 * 8)) & 0xff) as u8 ^ (i as u8);
        }

        if !handshake.message_2_transmitted() {
            if let Ok(m2) = handshake.on_message_1(eapol_frame, &snonce) {
                response_data = Some((m2, device.bssid, device.mac_address, device.tx_seq));
                device.tx_seq = device.tx_seq.wrapping_add(1);
            }
        } else if !handshake.completed() {
            if handshake.verify_message_3(eapol_frame).is_ok() {
                if let Ok(m4) = handshake.build_message_4() {
                    if let Some(tk) = handshake.transmit_key() {
                        device.tk = Some(*tk);
                        device.station_state = StationState::Linked;
                        handshake_complete = true;
                    }
                    response_data = Some((m4, device.bssid, device.mac_address, device.tx_seq));
                    device.tx_seq = device.tx_seq.wrapping_add(1);
                }
            }
        }
    }

    if let Some((eapol, bssid, station_mac, seq)) = response_data {
        send_eapol_packet(slot, &eapol, &bssid, &station_mac, seq);
    }

    if handshake_complete {
        crate::kprintln!(
            "[wifi] WPA2-PSK 4-way handshake completado con éxito; enlace Wi-Fi asegurado y activo"
        );
    }
}

fn send_mgmt_packet(slot: u8, frame: &[u8]) {
    let mut htc_buf = [0u8; 512];
    let Ok(htc_len) = encode_frame(0, 0, frame, &mut htc_buf) else {
        return;
    };
    let mut stream_buf = [0u8; 512];
    let Some(stream_len) = encode_tx_stream(&htc_buf[..htc_len], &mut stream_buf) else {
        return;
    };
    let _ = crate::drivers::xhci::wifi_send_wlan(slot, &stream_buf[..stream_len]);
}

fn send_eapol_packet(
    slot: u8,
    eapol: &[u8],
    bssid: &[u8; 6],
    station_mac: &[u8; 6],
    sequence: u16,
) {
    let mut dot11_frame = [0u8; 1024];
    if dot11_frame.len() < 24 + 8 + eapol.len() {
        return;
    }

    dot11_frame[0] = 0x08;
    dot11_frame[1] = 0x01;
    dot11_frame[2..4].fill(0);
    dot11_frame[4..10].copy_from_slice(bssid);
    dot11_frame[10..16].copy_from_slice(station_mac);
    dot11_frame[16..22].copy_from_slice(bssid);
    let seq_control = (sequence & 0x0fff) << 4;
    dot11_frame[22..24].copy_from_slice(&seq_control.to_le_bytes());

    dot11_frame[24..30].copy_from_slice(&LLC_SNAP_HEADER);
    dot11_frame[30..32].copy_from_slice(&[0x88, 0x8e]);
    dot11_frame[32..32 + eapol.len()].copy_from_slice(eapol);

    let total_len = 32 + eapol.len();
    let mut htc_buf = [0u8; 1024];
    let Ok(htc_len) = encode_frame(2, 0, &dot11_frame[..total_len], &mut htc_buf) else {
        return;
    };
    let mut stream_buf = [0u8; 1024];
    let Some(stream_len) = encode_tx_stream(&htc_buf[..htc_len], &mut stream_buf) else {
        return;
    };
    let _ = crate::drivers::xhci::wifi_send_wlan(slot, &stream_buf[..stream_len]);
}

fn transmit_ethernet(frame: &[u8]) -> Result<(), NetworkError> {
    let mut dot11_buf = [0u8; 2048];
    let (dot11_len, slot, data_ep) = {
        let mut devices = DEVICES.lock();
        let Some(device) = devices
            .iter_mut()
            .find(|d| d.station_state == StationState::Linked && d.tk.is_some())
        else {
            return Err(NetworkError::NoDevice);
        };

        let tk = device.tk.ok_or(NetworkError::DeviceFault)?;
        let slot = device.slot;
        let data_ep = device.data_endpoint;
        let seq = device.tx_seq;
        device.tx_seq = device.tx_seq.wrapping_add(1);

        let dot11_len = encapsulate_ethernet_to_80211(
            frame,
            &device.bssid,
            &device.mac_address,
            &mut device.tx_pn,
            &tk,
            seq,
            &mut dot11_buf,
        )
        .map_err(|_| NetworkError::InvalidFrame)?;

        (dot11_len, slot, data_ep)
    };

    let mut htc_buf = [0u8; 2048];
    let htc_len = encode_frame(data_ep, 0, &dot11_buf[..dot11_len], &mut htc_buf)
        .map_err(|_| NetworkError::DeviceFault)?;

    let mut stream_buf = [0u8; 2048];
    let stream_len = encode_tx_stream(&htc_buf[..htc_len], &mut stream_buf)
        .ok_or(NetworkError::FrameTooLarge)?;

    crate::drivers::xhci::wifi_send_wlan(slot, &stream_buf[..stream_len])
        .map_err(|_| NetworkError::DeviceFault)?;

    Ok(())
}

pub fn connect(slot: u8, ssid: &[u8], passphrase: &str) -> Result<(), &'static str> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err("SSID inválido");
    }
    if !lix_wifi::valid_wpa2_passphrase(passphrase.as_bytes()) {
        return Err("Contraseña WPA2-PSK inválida (8-63 caracteres)");
    }
    let psk = derive_psk(passphrase, ssid).map_err(|_| "Error derivando PSK")?;

    let (station_mac, seq) = {
        let mut devices = DEVICES.lock();
        let Some(device) = devices.iter_mut().find(|d| d.slot == slot) else {
            return Err("Dispositivo Wi-Fi no encontrado en slot");
        };
        device.ssid = [0u8; 32];
        device.ssid[..ssid.len()].copy_from_slice(ssid);
        device.ssid_len = ssid.len();
        device.psk = psk;
        device.station_state = StationState::Scanning;
        device.tk = None;
        device.handshake = None;
        device.tx_pn = 1;
        device.rx_pn = 0;
        let station_mac = device.mac_address;
        let seq = device.tx_seq;
        device.tx_seq = device.tx_seq.wrapping_add(1);
        (station_mac, seq)
    };

    let mut probe_buf = [0u8; 256];
    let probe_len = build_probe_request(&mut probe_buf, &station_mac, ssid, seq)
        .map_err(|_| "Error construyendo probe request")?;

    send_mgmt_packet(slot, &probe_buf[..probe_len]);
    crate::kprintln!(
        "[wifi] Iniciando escaneo y conexión a SSID: {:?} en slot {}",
        core::str::from_utf8(ssid).unwrap_or("???"),
        slot
    );
    Ok(())
}

pub fn disconnect(slot: u8) {
    let mut devices = DEVICES.lock();
    if let Some(device) = devices.iter_mut().find(|d| d.slot == slot) {
        device.station_state = StationState::Ready;
        device.tk = None;
        device.handshake = None;
        crate::kprintln!("[wifi] Desconectado de red en slot {}", slot);
    }
}

pub fn status(slot: u8) -> Option<StationState> {
    DEVICES
        .lock()
        .iter()
        .find(|d| d.slot == slot)
        .map(|d| d.station_state)
}

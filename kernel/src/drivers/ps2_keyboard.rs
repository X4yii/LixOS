use x86_64::instructions::port::Port;
const STATUS_PORT: u16 = 0x64;
const DATA_PORT: u16 = 0x60;
fn wait_input_empty(status: &mut Port<u8>) -> bool {
    for _ in 0..100_000 {
        let value: u8 = unsafe { status.read() };
        if value == 0xff {
            return false;
        }
        if value & 2 == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}
fn read_controller_byte(status: &mut Port<u8>, data: &mut Port<u8>) -> Option<u8> {
    for _ in 0..100_000 {
        let value: u8 = unsafe { status.read() };
        if value == 0xff {
            return None;
        }
        if value & 1 != 0 {
            return Some(unsafe { data.read() });
        }
        core::hint::spin_loop();
    }
    None
}
fn controller_command(status: &mut Port<u8>, command: u8) -> bool {
    if !wait_input_empty(status) {
        return false;
    }
    unsafe {
        status.write(command);
    }
    true
}
pub fn init() -> bool {
    let mut status = Port::new(STATUS_PORT);
    let mut data = Port::new(DATA_PORT);
    let initial: u8 = unsafe { status.read() };
    if initial == 0xff {
        return false;
    }
    let _ = controller_command(&mut status, 0xad);
    for _ in 0..32 {
        let state: u8 = unsafe { status.read() };
        if state & 1 == 0 {
            break;
        }
        let _: u8 = unsafe { data.read() };
    }
    if !controller_command(&mut status, 0x20) {
        crate::kprintln!("[ps2] No se pudo leer el byte de configuración del i8042");
        return false;
    }
    let Some(mut config) = read_controller_byte(&mut status, &mut data) else {
        crate::kprintln!("[ps2] Timeout leyendo configuración del i8042");
        return false;
    };
    config |= 1;
    config &= !(1 << 4);
    if !controller_command(&mut status, 0x60) || !wait_input_empty(&mut status) {
        crate::kprintln!("[ps2] No se pudo preparar el byte de configuración del i8042");
        return false;
    }
    unsafe {
        data.write(config);
    }
    if !controller_command(&mut status, 0xae) {
        crate::kprintln!("[ps2] No se pudo habilitar el puerto de teclado");
        return false;
    }
    if !wait_input_empty(&mut status) {
        return false;
    }
    unsafe {
        data.write(0xf4);
    }
    if read_controller_byte(&mut status, &mut data) != Some(0xfa) {
        crate::kprintln!("[ps2] El teclado no confirmó Enable Scanning (ACK)");
        return false;
    }
    crate::kprintln!("[ps2] Teclado i8042 habilitado; IRQ1 y scanning activos");
    true
}
pub fn handle_interrupt() -> bool {
    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    if let Some((key, code, pressed)) = crate::drivers::keyboard::process_scancode(scancode) {
        crate::drivers::input::push(crate::drivers::input::InputEvent {
            time_ns: crate::task::scheduler::total_ticks().saturating_mul(10_000_000),
            device: 0,
            kind: lix_usb::INPUT_KEY,
            code,
            value: i32::from(pressed),
        });
        if pressed {
            if let Some(key) = key {
                crate::drivers::keyboard::dispatch_decoded_key(key);
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    }
}

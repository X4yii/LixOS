use pic8259::ChainedPics;
use crate::sync::IrqSafeSpinlock;
use super::idt::PIC_1_OFFSET;

pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: IrqSafeSpinlock<ChainedPics> = IrqSafeSpinlock::new(
    unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) }
);

pub fn init() {
    crate::kprintln!("[pic] Inicializando y remapeando controlador 8259 PIC...");
    unsafe { 
        PICS.lock().initialize();
        let mut pic1_data: x86_64::instructions::port::Port<u8> = x86_64::instructions::port::Port::new(0x21);
        let mut pic2_data: x86_64::instructions::port::Port<u8> = x86_64::instructions::port::Port::new(0xA1);
        pic1_data.write(0xFD);
        pic2_data.write(0xFF);
    }
}
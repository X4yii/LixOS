//! # Definiciones del Protocolo de Arranque Limine (v8+)

#[repr(C)]
pub struct BaseRevision {
    id: [u64; 2],
    revision: u64,
}

impl BaseRevision {
    pub const fn new(revision: u64) -> Self {
        Self {
            id: [0xf9562b2d3616ae62, 0x83d210d9b3231b0b],
            revision,
        }
    }

    pub fn is_supported(&self) -> bool {
        self.revision == 0
    }
}

#[repr(C)]
pub struct HhdmRequest {
    id: [u64; 4],
    revision: u64,
    response: *const HhdmResponse,
}

impl HhdmRequest {
    pub const fn new() -> Self {
        Self {
            id: [
                0xc7b1dd30df4c8b88,
                0x0a82e883a194f07b,
                0x48ee8f524303a786,
                0xabdafa4624fb7818,
            ],
            revision: 0,
            response: core::ptr::null(),
        }
    }

    pub fn get_response(&self) -> Option<&'static HhdmResponse> {
        if self.response.is_null() {
            None
        } else {
            unsafe { Some(&*self.response) }
        }
    }
}

#[repr(C)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

#[repr(C)]
pub struct FramebufferRequest {
    id: [u64; 4],
    revision: u64,
    response: *const FramebufferResponse,
}

impl FramebufferRequest {
    pub const fn new() -> Self {
        Self {
            id: [
                0x9d5827dcd881dd7b,
                0xa3148604f34c6e64,
                0x48ee8f524303a786,
                0xabdafa4624fb7818,
            ],
            revision: 0,
            response: core::ptr::null(),
        }
    }

    pub fn get_response(&self) -> Option<&'static FramebufferResponse> {
        if self.response.is_null() {
            None
        } else {
            unsafe { Some(&*self.response) }
        }
    }
}

#[repr(C)]
pub struct FramebufferResponse {
    pub revision: u64,
    pub framebuffer_count: u64,
    pub framebuffers: *const *const Framebuffer,
}

#[repr(C)]
pub struct Framebuffer {
    pub address: *mut u8,
    pub width: u64,
    pub height: u64,
    pub pitch: u64,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
}

unsafe impl Sync for BaseRevision {}
unsafe impl Sync for HhdmRequest {}
unsafe impl Sync for FramebufferRequest {}

use crate::Error;
pub trait Entropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), Error>;
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoEntropy;
impl Entropy for NoEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), Error> {
        for byte in buf.iter_mut() {
            *byte = 0;
        }
        Err(Error::EntropyUnavailable)
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HardwareEntropy;
impl HardwareEntropy {
    pub fn is_supported() -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            let cpuid = core::arch::x86_64::__cpuid(1);
            (cpuid.ecx & (1 << 30)) != 0
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            false
        }
    }
}
impl Entropy for HardwareEntropy {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), Error> {
        #[cfg(target_arch = "x86_64")]
        {
            if !Self::is_supported() {
                buf.fill(0);
                return Err(Error::EntropyUnavailable);
            }
            let mut chunks = buf.chunks_exact_mut(8);
            for chunk in chunks.by_ref() {
                let mut val = 0u64;
                let mut success = false;
                for _ in 0..10 {
                    let ok: u8;
                    unsafe {
                        core::arch::asm!(
                            "rdrand {0}",
                            "setc {1}",
                            out(reg) val,
                            out(reg_byte) ok,
                            options(nomem, nostack)
                        );
                    }
                    if ok == 1 {
                        success = true;
                        break;
                    }
                }
                if !success {
                    buf.fill(0);
                    return Err(Error::EntropyUnavailable);
                }
                chunk.copy_from_slice(&val.to_le_bytes());
            }
            let remainder = chunks.into_remainder();
            if !remainder.is_empty() {
                let mut val = 0u64;
                let mut success = false;
                for _ in 0..10 {
                    let ok: u8;
                    unsafe {
                        core::arch::asm!(
                            "rdrand {0}",
                            "setc {1}",
                            out(reg) val,
                            out(reg_byte) ok,
                            options(nomem, nostack)
                        );
                    }
                    if ok == 1 {
                        success = true;
                        break;
                    }
                }
                if !success {
                    buf.fill(0);
                    return Err(Error::EntropyUnavailable);
                }
                remainder.copy_from_slice(&val.to_le_bytes()[..remainder.len()]);
            }
            Ok(())
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            buf.fill(0);
            Err(Error::EntropyUnavailable)
        }
    }
}
pub fn fill<E: Entropy + ?Sized>(source: &mut E, buf: &mut [u8]) -> Result<(), Error> {
    if buf.is_empty() {
        return Ok(());
    }
    source.fill(buf)
}
pub fn fresh_salt<E: Entropy + ?Sized>(source: &mut E) -> Result<[u8; 16], Error> {
    let mut salt = [0u8; 16];
    fill(source, &mut salt)?;
    Ok(salt)
}
pub fn fresh_nonce<E: Entropy + ?Sized>(source: &mut E) -> Result<[u8; 12], Error> {
    let mut nonce = [0u8; 12];
    fill(source, &mut nonce)?;
    Ok(nonce)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardware_entropy_fills_or_fails_closed() {
        let mut source = HardwareEntropy;
        let mut buf = [0u8; 32];
        if HardwareEntropy::is_supported() {
            assert!(source.fill(&mut buf).is_ok());
            assert_ne!(buf, [0u8; 32]);
        } else {
            assert_eq!(source.fill(&mut buf), Err(Error::EntropyUnavailable));
            assert_eq!(buf, [0u8; 32]);
        }
    }
    #[test]
    fn fresh_salt_and_nonce_generate_unique_values() {
        if !HardwareEntropy::is_supported() {
            return;
        }
        let mut source = HardwareEntropy;
        let salt1 = fresh_salt(&mut source).unwrap();
        let salt2 = fresh_salt(&mut source).unwrap();
        assert_ne!(salt1, salt2);
        let nonce1 = fresh_nonce(&mut source).unwrap();
        let nonce2 = fresh_nonce(&mut source).unwrap();
        assert_ne!(nonce1, nonce2);
    }
}

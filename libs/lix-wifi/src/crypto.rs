#![allow(clippy::many_single_char_names)]
use alloc::vec::Vec;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CryptoError {
    InvalidLength,
    InvalidNonce,
    BufferTooSmall,
    AuthenticationFailed,
}
const SHA1_BLOCK: usize = 64;
#[derive(Clone, Copy)]
pub struct Sha1 {
    state: [u32; 5],
    buffer: [u8; SHA1_BLOCK],
    buffered: usize,
    total: u64,
}
impl Sha1 {
    pub const fn new() -> Self {
        Self {
            state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0],
            buffer: [0; SHA1_BLOCK],
            buffered: 0,
            total: 0,
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        self.absorb(data);
    }
    fn absorb(&mut self, mut data: &[u8]) {
        if self.buffered > 0 {
            let take = core::cmp::min(SHA1_BLOCK - self.buffered, data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered == SHA1_BLOCK {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        while data.len() >= SHA1_BLOCK {
            let (block, rest) = data.split_at(SHA1_BLOCK);
            let mut fixed = [0u8; SHA1_BLOCK];
            fixed.copy_from_slice(block);
            self.compress(&fixed);
            data = rest;
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffered = data.len();
        }
    }
    pub fn finalize(self) -> [u8; 20] {
        let bits = self.total.wrapping_mul(8);
        let mut state = self;
        let mut padding = [0u8; 136];
        padding[0] = 0x80;
        let zeros = if state.buffered < 56 {
            56 - state.buffered - 1
        } else {
            120 - state.buffered - 1
        };
        padding[1 + zeros..1 + zeros + 8].copy_from_slice(&bits.to_be_bytes());
        state.absorb(&padding[..1 + zeros + 8]);
        let mut out = [0u8; 20];
        for (index, word) in state.state.iter().enumerate() {
            out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
    fn compress(&mut self, block: &[u8; SHA1_BLOCK]) {
        let mut w = [0u32; 80];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..80 {
            w[index] = (w[index - 3] ^ w[index - 8] ^ w[index - 14] ^ w[index - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = self.state;
        for (index, word) in w.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999u32),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }
}
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut digest = Sha1::new();
    digest.update(data);
    digest.finalize()
}
#[derive(Clone, Copy)]
pub struct HmacSha1 {
    inner: Sha1,
    outer_key: [u8; SHA1_BLOCK],
}
impl HmacSha1 {
    pub fn new(key: &[u8]) -> Self {
        let mut block = [0u8; SHA1_BLOCK];
        if key.len() > SHA1_BLOCK {
            block[..20].copy_from_slice(&sha1(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut ipad = [0x36u8; SHA1_BLOCK];
        for index in 0..SHA1_BLOCK {
            ipad[index] ^= block[index];
        }
        let mut inner = Sha1::new();
        inner.update(&ipad);
        Self {
            inner,
            outer_key: block,
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }
    pub fn finalize(self) -> [u8; 20] {
        let inner_digest = self.inner.finalize();
        let mut opad = [0x5cu8; SHA1_BLOCK];
        for index in 0..SHA1_BLOCK {
            opad[index] ^= self.outer_key[index];
        }
        let mut outer = Sha1::new();
        outer.update(&opad);
        outer.update(&inner_digest);
        outer.finalize()
    }
}
pub fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
    let mut mac = HmacSha1::new(key);
    mac.update(data);
    mac.finalize()
}
pub fn pbkdf2_sha1(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    out: &mut [u8],
) -> Result<(), CryptoError> {
    if iterations == 0 || out.is_empty() {
        return Err(CryptoError::InvalidLength);
    }
    let blocks = out.len().div_ceil(20);
    if blocks > u32::MAX as usize {
        return Err(CryptoError::InvalidLength);
    }
    let mut salt_block = Vec::with_capacity(salt.len().saturating_add(4));
    salt_block.extend_from_slice(salt);
    let base_len = salt_block.len();
    for index in 1..=blocks {
        salt_block.truncate(base_len);
        salt_block.extend_from_slice(&(index as u32).to_be_bytes());
        let mut u = hmac_sha1(password, &salt_block);
        let mut result = u;
        for _ in 1..iterations {
            u = hmac_sha1(password, &u);
            for byte in 0..20 {
                result[byte] ^= u[byte];
            }
        }
        let start = (index - 1) * 20;
        let end = core::cmp::min(start + 20, out.len());
        out[start..end].copy_from_slice(&result[..end - start]);
    }
    Ok(())
}
const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];
const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];
fn xtime(value: u8) -> u8 {
    (value << 1) ^ if value & 0x80 != 0 { 0x1b } else { 0 }
}
#[derive(Clone, Copy)]
pub struct Aes128 {
    round_keys: [[u8; 16]; 11],
}
impl Aes128 {
    pub fn new(key: &[u8; 16]) -> Self {
        let mut words = [[0u8; 4]; 44];
        for index in 0..4 {
            words[index].copy_from_slice(&key[index * 4..index * 4 + 4]);
        }
        for index in 4..44 {
            let mut temp = words[index - 1];
            if index % 4 == 0 {
                temp = [
                    SBOX[temp[1] as usize],
                    SBOX[temp[2] as usize],
                    SBOX[temp[3] as usize],
                    SBOX[temp[0] as usize],
                ];
                temp[0] ^= RCON[index / 4 - 1];
            }
            for byte in 0..4 {
                words[index][byte] = words[index - 4][byte] ^ temp[byte];
            }
        }
        let mut round_keys = [[0u8; 16]; 11];
        for round in 0..11 {
            for word in 0..4 {
                round_keys[round][word * 4..word * 4 + 4].copy_from_slice(&words[round * 4 + word]);
            }
        }
        Self { round_keys }
    }
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        add_round_key(block, &self.round_keys[0]);
        for round in 1..10 {
            sub_bytes(block);
            shift_rows(block);
            mix_columns(block);
            add_round_key(block, &self.round_keys[round]);
        }
        sub_bytes(block);
        shift_rows(block);
        add_round_key(block, &self.round_keys[10]);
    }
}
fn add_round_key(state: &mut [u8; 16], key: &[u8; 16]) {
    for index in 0..16 {
        state[index] ^= key[index];
    }
}
fn sub_bytes(state: &mut [u8; 16]) {
    for byte in state.iter_mut() {
        *byte = SBOX[*byte as usize];
    }
}
fn shift_rows(state: &mut [u8; 16]) {
    let original = *state;
    for row in 1..4 {
        for column in 0..4 {
            state[row + 4 * column] = original[row + 4 * ((column + row) % 4)];
        }
    }
}
fn mix_columns(state: &mut [u8; 16]) {
    for column in 0..4 {
        let a = [
            state[4 * column],
            state[4 * column + 1],
            state[4 * column + 2],
            state[4 * column + 3],
        ];
        state[4 * column] = xtime(a[0]) ^ (xtime(a[1]) ^ a[1]) ^ a[2] ^ a[3];
        state[4 * column + 1] = a[0] ^ xtime(a[1]) ^ (xtime(a[2]) ^ a[2]) ^ a[3];
        state[4 * column + 2] = a[0] ^ a[1] ^ xtime(a[2]) ^ (xtime(a[3]) ^ a[3]);
        state[4 * column + 3] = (xtime(a[0]) ^ a[0]) ^ a[1] ^ a[2] ^ xtime(a[3]);
    }
}
const CCM_L: usize = 2;
const CCM_NONCE: usize = 15 - CCM_L;
pub const CCM_TAG: usize = 8;
fn ccm_cbc_mac(
    cipher: &Aes128,
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<[u8; CCM_TAG], CryptoError> {
    if nonce.len() != CCM_NONCE {
        return Err(CryptoError::InvalidNonce);
    }
    if plaintext.len() > u16::MAX as usize || aad.len() > 0xfeff {
        return Err(CryptoError::InvalidLength);
    }
    let mut flags = 0x01u8 | (((CCM_TAG as u8 - 2) / 2) << 3);
    if !aad.is_empty() {
        flags |= 0x40;
    }
    let mut block = [0u8; 16];
    block[0] = flags;
    block[1..1 + CCM_NONCE].copy_from_slice(nonce);
    block[14] = (plaintext.len() >> 8) as u8;
    block[15] = plaintext.len() as u8;
    cipher.encrypt_block(&mut block);
    if !aad.is_empty() {
        let mut stream = Vec::with_capacity(2 + aad.len());
        stream.push((aad.len() >> 8) as u8);
        stream.push(aad.len() as u8);
        stream.extend_from_slice(aad);
        for offset in (0..stream.len()).step_by(16) {
            let end = core::cmp::min(offset + 16, stream.len());
            for index in 0..(end - offset) {
                block[index] ^= stream[offset + index];
            }
            cipher.encrypt_block(&mut block);
        }
    }
    for offset in (0..plaintext.len()).step_by(16) {
        let end = core::cmp::min(offset + 16, plaintext.len());
        for index in 0..(end - offset) {
            block[index] ^= plaintext[offset + index];
        }
        cipher.encrypt_block(&mut block);
    }
    let mut tag = [0u8; CCM_TAG];
    tag.copy_from_slice(&block[..CCM_TAG]);
    Ok(tag)
}
fn ccm_keystream_block(cipher: &Aes128, nonce: &[u8], counter: u16) -> [u8; 16] {
    let mut block = [0u8; 16];
    block[0] = 0x01;
    block[1..1 + CCM_NONCE].copy_from_slice(nonce);
    block[14] = (counter >> 8) as u8;
    block[15] = counter as u8;
    cipher.encrypt_block(&mut block);
    block
}
pub fn ccm_encrypt(
    key: &Aes128,
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
    out: &mut [u8],
) -> Result<usize, CryptoError> {
    if nonce.len() != CCM_NONCE {
        return Err(CryptoError::InvalidNonce);
    }
    let needed = plaintext
        .len()
        .checked_add(CCM_TAG)
        .ok_or(CryptoError::InvalidLength)?;
    if out.len() < needed {
        return Err(CryptoError::BufferTooSmall);
    }
    let tag = ccm_cbc_mac(key, nonce, aad, plaintext)?;
    for offset in (0..plaintext.len()).step_by(16) {
        let counter = ccm_keystream_block(key, nonce, (offset / 16 + 1) as u16);
        let end = core::cmp::min(offset + 16, plaintext.len());
        for index in 0..(end - offset) {
            out[offset + index] = plaintext[offset + index] ^ counter[index];
        }
    }
    let mut stream_zero = [0u8; 16];
    stream_zero[0] = 0x01;
    stream_zero[1..1 + CCM_NONCE].copy_from_slice(nonce);
    key.encrypt_block(&mut stream_zero);
    for index in 0..CCM_TAG {
        out[plaintext.len() + index] = tag[index] ^ stream_zero[index];
    }
    Ok(needed)
}
pub fn ccm_decrypt(
    key: &Aes128,
    nonce: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
    out: &mut [u8],
) -> Result<usize, CryptoError> {
    if nonce.len() != CCM_NONCE {
        return Err(CryptoError::InvalidNonce);
    }
    if ciphertext.len() < CCM_TAG {
        return Err(CryptoError::InvalidLength);
    }
    let body_len = ciphertext.len() - CCM_TAG;
    if out.len() < body_len {
        return Err(CryptoError::BufferTooSmall);
    }
    let mut stream_zero = [0u8; 16];
    stream_zero[0] = 0x01;
    stream_zero[1..1 + CCM_NONCE].copy_from_slice(nonce);
    key.encrypt_block(&mut stream_zero);
    let mut expected = [0u8; CCM_TAG];
    for index in 0..CCM_TAG {
        expected[index] = ciphertext[body_len + index] ^ stream_zero[index];
    }
    for offset in (0..body_len).step_by(16) {
        let counter = ccm_keystream_block(key, nonce, (offset / 16 + 1) as u16);
        let end = core::cmp::min(offset + 16, body_len);
        for index in 0..(end - offset) {
            out[offset + index] = ciphertext[offset + index] ^ counter[index];
        }
    }
    let tag = ccm_cbc_mac(key, nonce, aad, &out[..body_len])?;
    if tag != expected {
        for byte in out[..body_len].iter_mut() {
            *byte = 0;
        }
        return Err(CryptoError::AuthenticationFailed);
    }
    Ok(body_len)
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::String;
    fn hex(bytes: &[u8]) -> String {
        let mut text = String::new();
        for byte in bytes {
            text.push_str(&format!("{byte:02x}"));
        }
        text
    }
    #[test]
    fn sha1_matches_fips_180_vectors() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let mut long = Sha1::new();
        let block = [0x61u8; 1_000_000];
        for _ in 0..1_000 {
            long.update(&block[..1000]);
        }
        assert_eq!(
            hex(&long.finalize()),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }
    #[test]
    fn hmac_sha1_matches_rfc_2202_vectors() {
        let key = [0x0bu8; 20];
        assert_eq!(
            hex(&hmac_sha1(&key, b"Hi There")),
            "b617318655057264e28bc0b6fb378c8ef146be00"
        );
        assert_eq!(
            hex(&hmac_sha1(b"Jefe", b"what do ya want for nothing?")),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
        );
        let long_key = [0xaau8; 80];
        assert_eq!(
            hex(&hmac_sha1(
                &long_key,
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112"
        );
    }
    #[test]
    fn pbkdf2_sha1_matches_rfc_6070_vectors() {
        let mut out = [0u8; 20];
        pbkdf2_sha1(b"password", b"salt", 1, &mut out).unwrap();
        assert_eq!(hex(&out), "0c60c80f961f0e71f3a9b524af6012062fe037a6");
        pbkdf2_sha1(b"password", b"salt", 2, &mut out).unwrap();
        assert_eq!(hex(&out), "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957");
        pbkdf2_sha1(b"password", b"salt", 4096, &mut out).unwrap();
        assert_eq!(hex(&out), "4b007901b765489abead49d926f721d065a429c1");
        assert_eq!(
            pbkdf2_sha1(b"password", b"salt", 0, &mut out),
            Err(CryptoError::InvalidLength)
        );
    }
    #[test]
    fn pbkdf2_derives_multiple_blocks() {
        let mut out = [0u8; 40];
        pbkdf2_sha1(b"password", b"salt", 1, &mut out).unwrap();
        assert_eq!(
            hex(&out),
            "0c60c80f961f0e71f3a9b524af6012062fe037a6e0f0eb94fe8fc46bdc637164ac2e7a8e3f9d2e83"
        );
    }
    #[test]
    fn aes128_matches_fips_197_vector() {
        let key: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let mut block: [u8; 16] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let cipher = Aes128::new(&key);
        cipher.encrypt_block(&mut block);
        assert_eq!(hex(&block), "69c4e0d86a7b0430d8cdb78070b4c55a");
    }
    #[test]
    fn ccm_matches_rfc_3610_packet_vector_one() {
        let key: [u8; 16] = [
            0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xcb, 0xcc, 0xcd,
            0xce, 0xcf,
        ];
        let nonce = [
            0x00, 0x00, 0x00, 0x03, 0x02, 0x01, 0x00, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5,
        ];
        let aad = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];
        let plaintext = [
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15,
            0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        ];
        let cipher = Aes128::new(&key);
        let mut out = [0u8; 31];
        let written = ccm_encrypt(&cipher, &nonce, &aad, &plaintext, &mut out).unwrap();
        assert_eq!(written, 31);
        assert_eq!(
            hex(&out),
            "588c979a61c663d2f066d0c2c0f989806d5f6b61dac38417e8d12cfdf926e0"
        );
        let mut recovered = [0u8; 31];
        let length = ccm_decrypt(&cipher, &nonce, &aad, &out, &mut recovered).unwrap();
        assert_eq!(length, plaintext.len());
        assert_eq!(&recovered[..length], &plaintext[..]);
    }
    #[test]
    fn ccm_rejects_modified_ciphertext() {
        let key: [u8; 16] = [7u8; 16];
        let nonce = [9u8; 13];
        let cipher = Aes128::new(&key);
        let mut out = [0u8; 32];
        let written = ccm_encrypt(&cipher, &nonce, b"header", b"payload", &mut out).unwrap();
        out[0] ^= 0x01;
        let mut recovered = [0u8; 32];
        assert_eq!(
            ccm_decrypt(&cipher, &nonce, b"header", &out[..written], &mut recovered),
            Err(CryptoError::AuthenticationFailed)
        );
    }
    #[test]
    fn ccm_rejects_modified_additional_data() {
        let key: [u8; 16] = [3u8; 16];
        let nonce = [4u8; 13];
        let cipher = Aes128::new(&key);
        let mut out = [0u8; 32];
        let written = ccm_encrypt(&cipher, &nonce, b"header", b"payload", &mut out).unwrap();
        let mut recovered = [0u8; 32];
        assert_eq!(
            ccm_decrypt(&cipher, &nonce, b"headex", &out[..written], &mut recovered),
            Err(CryptoError::AuthenticationFailed)
        );
    }
}

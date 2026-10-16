//! Journal on-disk (WAL) para EXT4, con tamaño acotado de log.
//!
//! Layout del journal (área reservada por `format_ext4`):
//!
//! ```text
//! [0]   superblock JBD2  (magic, maxlen, first, sequence, start)
//! [1..] pares (header, data) de transacciones + bloque COMMIT
//! ```
//!
//! Cada transacción escribe primero los pares `header + data` de todas las
//! modificaciones (redo WAL) y después un bloque `COMMIT` con la secuencia.
//! Al arrancar, `replay()` sólo aplica registros con secuencia confirmada, de
//! modo que un corte de energía a mitad de escritura deja el journal en estado
//! "incompleto" y la transacción se descarta sin corromper el volumen.
//!
//! El tamaño del log se acota a `JOURNAL_MAX_BLOCKS` (4 MiB / bloque) para que
//! ni el formateador ni el kernel puedan crecer sin límite.

use alloc::vec;
use alloc::vec::Vec;
pub const JBD2_MAGIC: u32 = 0xC03B_3998;
pub const JBD2_SUPERBLOCK_V2: u32 = 4;
pub const REC_HEADER: u32 = 0x4C49_5848;
pub const REC_COMMIT: u32 = 0x4C49_5843;
/// Tamaño máximo del journal en bloques (4 MiB con bloques de 4 KiB).
pub const JOURNAL_MAX_BLOCKS: u32 = 1024;
/// Tamaño mínimo del journal en bloques (256 KiB con bloques de 4 KiB).
pub const JOURNAL_MIN_BLOCKS: u32 = 64;
pub const HEADER_BYTES: usize = 64;
pub const DATA_BYTES_OFFSET: usize = HEADER_BYTES;
/// Acota el número de bloques de journal a `[JOURNAL_MIN_BLOCKS, JOURNAL_MAX_BLOCKS]`.
pub fn clamp_journal_blocks(blocks: u32) -> u32 {
    blocks.clamp(JOURNAL_MIN_BLOCKS, JOURNAL_MAX_BLOCKS)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalSuperblock {
    pub maxlen: u32,
    pub first: u32,
    pub sequence: u32,
    /// `0` => journal limpio; `1` => hay transacciones sin checkpointear.
    pub start: u32,
}
impl JournalSuperblock {
    pub fn new(maxlen: u32) -> Self {
        Self {
            maxlen: clamp_journal_blocks(maxlen),
            first: 1,
            sequence: 1,
            start: 0,
        }
    }
    pub fn encode(&self) -> [u8; 512] {
        let mut buf = [0u8; 512];
        buf[0..4].copy_from_slice(&JBD2_MAGIC.to_le_bytes());
        buf[4..8].copy_from_slice(&JBD2_SUPERBLOCK_V2.to_le_bytes());
        buf[12..16].copy_from_slice(&self.maxlen.to_le_bytes());
        buf[16..20].copy_from_slice(&self.first.to_le_bytes());
        buf[20..24].copy_from_slice(&self.sequence.to_le_bytes());
        buf[24..28].copy_from_slice(&self.start.to_le_bytes());
        buf
    }
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < 28 {
            return None;
        }
        let magic = u32::from_le_bytes(buf[0..4].try_into().ok()?);
        if magic != JBD2_MAGIC {
            return None;
        }
        Some(Self {
            maxlen: u32::from_le_bytes(buf[12..16].try_into().ok()?),
            first: u32::from_le_bytes(buf[16..20].try_into().ok()?),
            sequence: u32::from_le_bytes(buf[20..24].try_into().ok()?),
            start: u32::from_le_bytes(buf[24..28].try_into().ok()?),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalHeader {
    pub magic: u32,
    pub sequence: u32,
    pub lba: u64,
    pub data_len: u32,
    pub crc32: u32,
}
impl JournalHeader {
    pub fn record(sequence: u32, lba: u64, data_len: u32, crc32: u32) -> Self {
        Self { magic: REC_HEADER, sequence, lba, data_len, crc32 }
    }
    pub fn commit(sequence: u32) -> Self {
        Self { magic: REC_COMMIT, sequence, lba: 0, data_len: 0, crc32: 0 }
    }
    pub fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut buf = [0u8; HEADER_BYTES];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.sequence.to_le_bytes());
        buf[8..16].copy_from_slice(&self.lba.to_le_bytes());
        buf[16..20].copy_from_slice(&self.data_len.to_le_bytes());
        buf[20..24].copy_from_slice(&self.crc32.to_le_bytes());
        buf
    }
    pub fn decode(buf: &[u8]) -> Option<Self> {
        if buf.len() < 24 {
            return None;
        }
        Some(Self {
            magic: u32::from_le_bytes(buf[0..4].try_into().ok()?),
            sequence: u32::from_le_bytes(buf[4..8].try_into().ok()?),
            lba: u64::from_le_bytes(buf[8..16].try_into().ok()?),
            data_len: u32::from_le_bytes(buf[16..20].try_into().ok()?),
            crc32: u32::from_le_bytes(buf[20..24].try_into().ok()?),
        })
    }
}
/// CRC32 (IEEE 802.3, poly 0xEDB88320) para validar datos del journal.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
/// Almacenamiento mínimo que el journal necesita (backends mock o disco real).
pub trait JournalStore {
    fn read_block(&mut self, index: u32, buf: &mut [u8]) -> bool;
    fn write_block(&mut self, index: u32, buf: &[u8]) -> bool;
    fn block_size(&self) -> usize;
}
/// Journal activo sobre un store. Mantiene cursor de escritura y secuencia.
pub struct Journal<S: JournalStore> {
    store: S,
    pub superblock: JournalSuperblock,
    /// Cursor de escritura dentro del área de log (índice absoluto).
    cursor: u32,
    /// Secuencia de la transacción en curso.
    seq: u32,
    /// LBA absoluta de primer bloque del journal.
    base: u32,
}
impl<S: JournalStore> Journal<S> {
    pub fn new(mut store: S, base: u32, maxlen: u32) -> Option<Self> {
        let maxlen = clamp_journal_blocks(maxlen);
        if maxlen < JOURNAL_MIN_BLOCKS {
            return None;
        }
        let mut sb_buf = vec![0u8; store.block_size()];
        let sb = if store.read_block(base, &mut sb_buf) {
            JournalSuperblock::decode(&sb_buf).filter(|s| s.maxlen >= JOURNAL_MIN_BLOCKS)
        } else {
            None
        };
        let sb = match sb {
            Some(s) if s.maxlen == maxlen => s,
            _ => JournalSuperblock::new(maxlen),
        };
        let mut journal = Self {
            store,
            superblock: sb,
            cursor: base + 1,
            seq: sb.sequence,
            base,
        };
        journal.scan();
        Some(journal)
    }
    /// Posición de escritura = primer bloque libre tras el último COMMIT.
    fn scan(&mut self) {
        let bs = self.store.block_size();
        let mut buf = vec![0u8; bs];
        let end = self.base + self.superblock.maxlen;
        let mut idx = self.base + 1;
        let mut last_commit = None;
        while idx + 1 < end {
            if !self.store.read_block(idx, &mut buf) {
                break;
            }
            let header = match JournalHeader::decode(&buf) {
                Some(h) => h,
                None => break,
            };
            if header.magic == REC_COMMIT {
                last_commit = Some(idx);
                idx += 1;
                continue;
            }
            if header.magic != REC_HEADER {
                break;
            }
            idx += 2;
        }
        if let Some(commit) = last_commit {
            self.cursor = commit + 1;
            self.seq = self.seq.wrapping_add(1);
            self.superblock.start = 0;
        } else {
            self.cursor = self.base + 1;
            self.superblock.start = 0;
        }
        if self.cursor + 2 >= self.base + self.superblock.maxlen {
            self.cursor = self.base + 1;
        }
        self.flush_superblock();
    }
    fn flush_superblock(&mut self) {
        self.superblock.sequence = self.seq;
        let sb = self.superblock.encode();
        let bs = self.store.block_size();
        let mut buf = vec![0u8; bs];
        let n = sb.len().min(bs);
        buf[..n].copy_from_slice(&sb[..n]);
        self.store.write_block(self.base, &buf);
    }
    /// Registra `data` (destino `lba`) en el journal. Devuelve `false` si no cabe.
    pub fn log(&mut self, lba: u64, data: &[u8]) -> bool {
        let bs = self.store.block_size();
        if data.is_empty() || data.len() > bs {
            return false;
        }
        let end = self.base + self.superblock.maxlen;
        if self.cursor + 3 > end {
            self.cursor = self.base + 1;
        }
        let header = JournalHeader::record(self.seq, lba, data.len() as u32, crc32(data));
        let mut hbuf = vec![0u8; bs];
        hbuf[..HEADER_BYTES].copy_from_slice(&header.encode());
        if !self.store.write_block(self.cursor, &hbuf) {
            return false;
        }
        let mut dbuf = vec![0u8; bs];
        dbuf[..data.len()].copy_from_slice(data);
        if !self.store.write_block(self.cursor + 1, &dbuf) {
            return false;
        }
        self.cursor += 2;
        self.superblock.start = 1;
        self.flush_superblock();
        true
    }
    /// Confirma la transacción actual escribiendo el bloque COMMIT.
    pub fn commit(&mut self) -> bool {
        if self.superblock.start == 0 {
            return true;
        }
        let bs = self.store.block_size();
        let end = self.base + self.superblock.maxlen;
        if self.cursor + 1 > end {
            self.cursor = self.base + 1;
        }
        let header = JournalHeader::commit(self.seq);
        let mut buf = vec![0u8; bs];
        buf[..HEADER_BYTES].copy_from_slice(&header.encode());
        if !self.store.write_block(self.cursor, &buf) {
            return false;
        }
        self.cursor += 1;
        self.seq = self.seq.wrapping_add(1);
        self.superblock.start = 0;
        self.flush_superblock();
        true
    }
    /// Aplica los registros de las secuencias confirmadas (COMMIT).
    ///
    /// `apply(lba, data)` debe escribir el bloque en su destino real. Sólo se
    /// invoca para registros cuyo CRC32 es correcto **y** que pertenecen a una
    /// transacción con COMMIT; las transacciones a medias se descartan.
    ///
    /// Devuelve el número de registros aplicados.
    pub fn replay(&mut self, mut apply: impl FnMut(u64, &[u8])) -> usize {
        let bs = self.store.block_size();
        let end = self.base + self.superblock.maxlen;
        let mut idx = self.base + 1;
        let mut hbuf = vec![0u8; bs];
        let mut dbuf = vec![0u8; bs];
        let mut applied = 0;
        let mut pending: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut pending_seq: Option<u32> = None;
        while idx + 1 < end {
            if !self.store.read_block(idx, &mut hbuf) {
                break;
            }
            let Some(header) = JournalHeader::decode(&hbuf) else { break };
            if header.magic == REC_COMMIT {
                let ok = pending_seq.map(|s| s == header.sequence).unwrap_or(false);
                if ok {
                    for (lba, data) in core::mem::take(&mut pending) {
                        apply(lba, &data);
                        applied += 1;
                    }
                } else {
                    pending.clear();
                }
                pending_seq = None;
                idx += 1;
                continue;
            }
            if header.magic != REC_HEADER {
                break;
            }
            if !self.store.read_block(idx + 1, &mut dbuf) {
                break;
            }
            let len = (header.data_len as usize).min(bs);
            let data = &dbuf[..len];
            if crc32(data) == header.crc32 {
                pending_seq = Some(header.sequence);
                pending.push((header.lba, data.to_vec()));
            } else {
                pending.clear();
                pending_seq = None;
            }
            idx += 2;
        }
        applied
    }
    pub fn base(&self) -> u32 {
        self.base
    }
    pub fn store(&self) -> &S {
        &self.store
    }
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct MemStore {
        block_size: usize,
        blocks: Vec<Vec<u8>>,
    }
    impl JournalStore for MemStore {
        fn read_block(&mut self, index: u32, buf: &mut [u8]) -> bool {
            match self.blocks.get(index as usize) {
                Some(b) => {
                    let n = buf.len().min(b.len());
                    buf[..n].copy_from_slice(&b[..n]);
                    true
                }
                None => false,
            }
        }
        fn write_block(&mut self, index: u32, buf: &[u8]) -> bool {
            let idx = index as usize;
            if idx >= self.blocks.len() {
                self.blocks.resize(idx + 1, vec![0u8; self.block_size]);
            }
            let dst = &mut self.blocks[idx];
            let n = dst.len().min(buf.len());
            dst[..n].copy_from_slice(&buf[..n]);
            true
        }
        fn block_size(&self) -> usize {
            self.block_size
        }
    }
    fn store() -> MemStore {
        MemStore { block_size: 512, blocks: vec![vec![0u8; 512]; 16] }
    }
    #[test]
    fn clamp_bounds() {
        assert_eq!(clamp_journal_blocks(0), JOURNAL_MIN_BLOCKS);
        assert_eq!(clamp_journal_blocks(999_999), JOURNAL_MAX_BLOCKS);
        assert_eq!(clamp_journal_blocks(512), 512);
    }
    #[test]
    fn superblock_roundtrip() {
        let sb = JournalSuperblock::new(1024);
        let enc = sb.encode();
        let dec = JournalSuperblock::decode(&enc).unwrap();
        assert_eq!(dec.maxlen, 1024);
        assert_eq!(dec.first, 1);
        assert_eq!(dec.start, 0);
    }
    #[test]
    fn bad_magic_rejected() {
        let buf = [0u8; 64];
        assert!(JournalSuperblock::decode(&buf).is_none());
    }
    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
    #[test]
    fn log_commit_cycle() {
        let mut j = Journal::new(store(), 0, 16).unwrap();
        assert!(j.log(42, b"hello"));
        assert_eq!(j.superblock.start, 1);
        assert!(j.commit());
        assert_eq!(j.superblock.start, 0);
    }
    #[test]
    fn journal_rejects_oversized_data() {
        let mut j = Journal::new(store(), 0, 16).unwrap();
        let big = vec![0u8; 1024];
        assert!(!j.log(1, &big));
    }
    #[test]
    fn cursor_wraps_when_full() {
        let mut j = Journal::new(store(), 0, 16).unwrap();
        for i in 0..40u64 {
            assert!(j.log(i, b"x"), "log {i} no cabió");
            assert!(j.commit());
        }
        assert!(j.cursor <= j.base + j.superblock.maxlen);
    }
}

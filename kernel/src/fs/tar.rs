use core::str;

#[derive(Debug)]
pub struct TarFile<'a> {
    pub name: &'a str,
    pub size: usize,
    pub data: &'a [u8],
}

pub struct TarArchive<'a> {
    data: &'a [u8],
}

impl<'a> TarArchive<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn files(&self) -> TarIterator<'a> {
        TarIterator {
            data: self.data,
            offset: 0,
        }
    }
}

pub struct TarIterator<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for TarIterator<'a> {
    type Item = TarFile<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.offset + 512 > self.data.len() {
                return None;
            }

            let header = &self.data[self.offset..self.offset + 512];
            
            if header[0] == 0 {
                return None;
            }

            let name_len = header[0..100].iter().position(|&c| c == 0).unwrap_or(100);
            let name = str::from_utf8(&header[0..name_len]).unwrap_or("<invalid utf8>");

            let size_str = str::from_utf8(&header[124..135]).unwrap_or("0").trim_start_matches('0');
            let size = if size_str.is_empty() {
                0
            } else {
                usize::from_str_radix(size_str, 8).unwrap_or(0)
            };

            let type_flag = header[156];
            let is_file = type_flag == 0 || type_flag == b'0';

            let data_start = self.offset + 512;
            let data_end = data_start + size;

            let padded_size = (size + 511) & !511;
            self.offset += 512 + padded_size;

            if is_file && size > 0 {
                let file_data = if data_end <= self.data.len() {
                    &self.data[data_start..data_end]
                } else {
                    &[]
                };

                return Some(TarFile {
                    name,
                    size,
                    data: file_data,
                });
            }
        }
    }
}
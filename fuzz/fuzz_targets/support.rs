#![allow(dead_code)]

use mp4forge::FourCc;
use mp4forge::walk::BoxPath;

pub struct FuzzInput<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> FuzzInput<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub fn take_u8(&mut self) -> u8 {
        let Some(byte) = self.data.get(self.offset).copied() else {
            return 0;
        };
        self.offset += 1;
        byte
    }

    pub fn take_u16(&mut self) -> u16 {
        u16::from_be_bytes(self.take_exact())
    }

    pub fn take_u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take_exact())
    }

    pub fn take_exact<const N: usize>(&mut self) -> [u8; N] {
        let mut bytes = [0_u8; N];
        for byte in &mut bytes {
            *byte = self.take_u8();
        }
        bytes
    }

    pub fn take_bool(&mut self) -> bool {
        self.take_u8() & 1 != 0
    }

    pub fn take_usize(&mut self, max_inclusive: usize) -> usize {
        if max_inclusive == 0 {
            return 0;
        }
        usize::from(self.take_u8()) % (max_inclusive + 1)
    }

    pub fn take_bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.take_usize(max_len);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(self.take_u8());
        }
        bytes
    }

    pub fn take_fourcc(&mut self) -> FourCc {
        FourCc::from_bytes(self.take_exact())
    }

    pub fn take_path(&mut self, max_depth: usize) -> BoxPath {
        let depth = self.take_usize(max_depth);
        let mut parts = Vec::with_capacity(depth);
        for _ in 0..depth {
            parts.push(if self.take_bool() {
                FourCc::ANY
            } else {
                self.take_fourcc()
            });
        }
        BoxPath::from(parts)
    }

    pub fn choose_fourcc(&mut self, table: &[FourCc]) -> FourCc {
        table[self.take_usize(table.len() - 1)]
    }
}

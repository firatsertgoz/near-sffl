pub trait BytesUtils {
    fn to_u8(&self, start: usize) -> u8;
    fn to_u16(&self, start: usize) -> u16;
    fn to_u32(&self, start: usize) -> u32;
    fn to_u64(&self, start: usize) -> u64;
    fn to_u128(&self, start: usize) -> u128;
    fn to_bytes32(&self, start: usize) -> &[u8];
    fn to_byte_array<const N: usize>(&self, start: usize) -> [u8; N];
}

impl BytesUtils for &[u8] {
    fn to_u8(&self, start: usize) -> u8 {
        self[start]
    }

    fn to_u16(&self, start: usize) -> u16 {
        let mut bytes: [u8; 2] = [0; 2];
        bytes.copy_from_slice(&self[start..start + 2]);
        u16::from_be_bytes(bytes)
    }

    fn to_u32(&self, start: usize) -> u32 {
        let mut bytes: [u8; 4] = [0; 4];
        bytes.copy_from_slice(&self[start..start + 4]);
        u32::from_be_bytes(bytes)
    }

    fn to_u64(&self, start: usize) -> u64 {
        let mut bytes: [u8; 8] = [0; 8];
        bytes.copy_from_slice(&self[start..start + 8]);
        u64::from_be_bytes(bytes)
    }

    fn to_u128(&self, start: usize) -> u128 {
        let mut bytes: [u8; 16] = [0; 16];
        bytes.copy_from_slice(&self[start..start + 16]);
        u128::from_be_bytes(bytes)
    }

    fn to_bytes32(&self, start: usize) -> &[u8] {
        &self[start..start + 32]
    }

    fn to_byte_array<const N: usize>(&self, start: usize) -> [u8; N] {
        let mut bytes: [u8; N] = [0; N];
        bytes.copy_from_slice(&self[start..start + N]);
        bytes
    }
}

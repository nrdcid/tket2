/// Temporary implementation of SIMD vectors for Tableau
/// Uses primitive rust operations to avoid dependencies
/// Use rupauli instead when tket is ready to depend on it

/// A wrapper type to enforce 32-byte alignment for SIMD loads
#[repr(align(32))]
struct SIMDLanes([i32; 8]);

#[derive(Debug, Clone, Copy)]
pub struct SIMDBlock {
    #[cfg(target_feature = "avx2")]
    inner: std::arch::x86_64::__m256i,

    #[cfg(target_feature = "neon")]
    inner: [std::arch::aarch64::int32x4_t; 2],

    #[cfg(not(any(target_feature = "avx2", target_feature = "neon")))]
    inner: [i32; 8],
}

impl SIMDBlock {
    #[cfg(target_feature = "avx2")]
    fn constant(a: i32) -> Self {
        SIMDBlock {
            inner: unsafe { std::arch::x86_64::_mm256_set1_epi32(a) }
        }
    }

    #[cfg(target_feature = "neon")]
    fn constant(a: i32) -> Self {
        SIMDBlock {
            inner: unsafe { [std::arch::aarch64::vdupq_n_s32(a); 2] }
        }
    }

    #[cfg(not(any(target_feature = "avx2", target_feature = "neon")))]
    fn constant(a: i32) -> Self {
        SIMDBlock {
            inner: [a; 8]
        }
    }

    #[cfg(target_feature = "avx2")]
    fn load(arr: &SIMDLanes) -> Self {
        SIMDBlock {
            inner: unsafe { 
                std::arch::x86_64::_mm256_load_si256(arr.0.as_ptr() as *const _)
            }
        }
    }

    #[cfg(target_feature = "neon")]
    fn load(arr: &SIMDLanes) -> Self {
        SIMDBlock {
            inner: unsafe {
                [
                    std::arch::aarch64::vld1q_s32(&arr.0[0]), 
                    std::arch::aarch64::vld1q_s32(&arr.0[4])
                ]
            }
        }
    }

    #[cfg(not(any(target_feature = "avx2", target_feature = "neon")))]
    fn load(arr: &SIMDLanes) -> Self {
        SIMDBlock {
            inner: arr.0.clone()
        }
    }

    #[cfg(target_feature = "avx2")]
    fn zero() -> Self {
        SIMDBlock {
            inner: unsafe { std::arch::x86_64::_mm256_setzero_si256() }
        }
    }

    #[cfg(target_feature = "neon")]
    fn zero() -> Self {
        SIMDBlock::constant(0)
    }   

    #[cfg(not(any(target_feature = "avx2", target_feature = "neon")))]
    fn zero() -> Self {
        Self::constant(0)
    }

    #[cfg(target_feature = "avx2")]
    fn extract(&self) -> [i32; 8] {
        let mut arr = SIMDLanes([0; 8]);
        unsafe {
            std::arch::x86_64::_mm256_store_si256(arr.0.as_mut_ptr() as *mut _, self.inner);
            }
        arr.0
    }

    #[cfg(target_feature = "neon")]
    fn extract(&self) -> [i32; 8] {
        let mut arr = SIMDLanes([0; 8]);
        unsafe {
            std::arch::aarch64::vst1q_s32(arr.0.as_mut_ptr(), self.inner[0]);
            std::arch::aarch64::vst1q_s32(arr.0.as_mut_ptr().add(4), self.inner[1]);
        }
        arr.0
    }

    #[cfg(not(any(target_feature = "avx2", target_feature = "neon")))]
    fn extract(&self) -> [i32; 8] {
        self.inner
    }
}

impl std::ops::BitXorAssign for SIMDBlock {
    #[cfg(target_feature = "avx2")]
    fn bitxor_assign(&mut self, rhs: Self) {
        self.inner = unsafe {
            std::arch::x86_64::_mm256_xor_si256(self.inner, rhs.inner)
        };
    }

    #[cfg(target_feature = "neon")]
    fn bitxor_assign(&mut self, rhs: Self) {
        self.inner[0] = unsafe {
            std::arch::aarch64::veorq_s32(self.inner[0], rhs.inner[0])
        };
        self.inner[1] = unsafe {
            std::arch::aarch64::veorq_s32(self.inner[1], rhs.inner[1])
        };
    }

    #[cfg(not(any(target_feature = "avx2", target_feature = "neon")))]
    fn bitxor_assign(&mut self, rhs: Self) {
        for i in 0..8 {
            self.inner[i] ^= rhs.inner[i];
        }
    }
}

impl std::ops::BitAndAssign for SIMDBlock {
    #[cfg(target_feature = "avx2")]
    fn bitand_assign(&mut self, rhs: Self) {
        self.inner = unsafe {
            std::arch::x86_64::_mm256_and_si256(self.inner, rhs.inner)
        };
    }

    #[cfg(target_feature = "neon")]
    fn bitand_assign(&mut self, rhs: Self) {
        self.inner[0] = unsafe {
            std::arch::aarch64::vandq_s32(self.inner[0], rhs.inner[0])
        };
        self.inner[1] = unsafe {
            std::arch::aarch64::vandq_s32(self.inner[1], rhs.inner[1])
        };
    }


    #[cfg(not(any(target_feature = "avx2", target_feature = "neon")))]
    fn bitand_assign(&mut self, rhs: Self) {
        for i in 0..8 {
            self.inner[i] &= rhs.inner[i];
        }
    }
}

#[derive(Debug, Clone)]
pub struct SIMDVector {
    pub blocks: Vec<SIMDBlock>,
}

impl SIMDVector {
    const LANES: usize = 8;
    const LANE_SIZE: usize = 32;
    const BLOCK_SIZE: usize = 256;

    pub fn new(nb_bits: usize) -> Self {
        SIMDVector {
            blocks: SIMDVector::init_blocks(nb_bits),
        }
    }

    pub fn new_block_size(nb_blocks: usize) -> Self {
        SIMDVector {
            blocks: SIMDVector::init_blocks(nb_blocks * SIMDVector::BLOCK_SIZE - 1),
        }
    }

    pub fn from_integer_vec(vec: Vec<i128>) -> Self {
        let mut bv = SIMDVector::new(vec.len() * 128 - 1);
        let mut arr = SIMDLanes([0; SIMDVector::LANES]);
        let mut block_index = 0;
        let mut index = 0;
        for v in vec {
            let mut val = v.clone();
            for i in 0..4 {
                arr.0[index] = val as u32 as i32;
                index += 1;
                if i < 3 {
                    val = val >> SIMDVector::LANE_SIZE;
                }
            }
            if index == 8 {
                index = 0;
                bv.blocks[block_index] = SIMDBlock::load(&arr);
                block_index += 1;
                arr = SIMDLanes([0; SIMDVector::LANES]);
            }
        }
        if index > 0 {
            bv.blocks[block_index] = SIMDBlock::load(&arr);
        }
        bv
    }

    fn init_blocks(nb_bits: usize) -> Vec<SIMDBlock> {
        let capacity = nb_bits / SIMDVector::BLOCK_SIZE + 1;
        let mut vec: Vec<SIMDBlock> = Vec::with_capacity(capacity);
        for _ in 0..vec.capacity() {
            vec.push(SIMDBlock::zero());
        }
        vec
    }

    pub fn size(&self) -> usize {
        self.blocks.len() * SIMDVector::BLOCK_SIZE
    }

    pub fn xor_bit(&mut self, mut bit: usize) {
        let block_index = bit / SIMDVector::BLOCK_SIZE;
        bit = bit % SIMDVector::BLOCK_SIZE;
        let lane_index = bit / SIMDVector::LANE_SIZE;
        bit = bit % SIMDVector::LANE_SIZE;
        let mut arr = SIMDLanes([0; SIMDVector::LANES]);
        arr.0[lane_index] ^= 1 << bit;
        self.blocks[block_index] ^= SIMDBlock::load(&arr);
    }

    pub fn get(&self, mut bit: usize) -> bool {
        let block_index = bit / SIMDVector::BLOCK_SIZE;
        bit = bit % SIMDVector::BLOCK_SIZE;
        let lane_index = bit / SIMDVector::LANE_SIZE;
        bit = bit % SIMDVector::LANE_SIZE;
        self.extract_block(block_index)[lane_index] & (1 << bit) != 0
    }

    pub fn get_first_one(&self) -> usize {
        for i in 0..self.blocks.len() {
            let block = self.extract_block(i);
            for j in 0..SIMDVector::LANES {
                for k in 0..SIMDVector::LANE_SIZE {
                    if block[j] & (1 << k) != 0 {
                        return i * SIMDVector::BLOCK_SIZE + j * SIMDVector::LANE_SIZE + k;
                    }
                }
            }
        }
        0
    }

    pub fn get_all_ones(&self, nb_bits: usize) -> Vec<usize> {
        let mut index = 0;
        let mut vec = Vec::new();
        for i in 0..self.blocks.len() {
            let block = self.extract_block(i);
            for j in 0..SIMDVector::LANES {
                for k in 0..SIMDVector::LANE_SIZE {
                    if block[j] & (1 << k) != 0 { vec.push(index); }
                    index += 1;
                    if index >= nb_bits { return vec; }
                }
            }
        }
        vec
    }

    pub fn xor(&mut self, bv: &SIMDVector) {
        for i in 0..self.blocks.len() {
            self.blocks[i] ^= bv.blocks[i];
        }
    }

    pub fn and(&mut self, bv: &SIMDVector) {
        for i in 0..self.blocks.len() {
            self.blocks[i] &= bv.blocks[i];
        }
    }

    pub fn negate(&mut self) {
        let a: i32 = !0;
        for i in 0..self.blocks.len() {
            self.blocks[i] ^= SIMDBlock::constant(a);
        }
    }
    
    pub fn get_boolean_vec(&self) -> Vec<bool> {
        let capacity = self.blocks.len() * SIMDVector::BLOCK_SIZE;
        let mut vec: Vec<bool> = Vec::with_capacity(capacity);
        for block_index in 0..self.blocks.len() {
            let arr = self.extract_block(block_index);
            for j in 0..8 {
                for i in 0..32 {
                    vec.push(arr[j] & (1 << i) != 0);
                }
            }
        }
        vec
    }

    pub fn get_integer_vec(&self) -> Vec<i128> {
        let mut vec: Vec<i128> = Vec::with_capacity(self.blocks.len() * 2);
        for block_index in 0..self.blocks.len() {
            let arr = self.extract_block(block_index);
            for k in 0..2 {
                let mut integer: i128 = 0;
                for j in 0..4 {
                    integer ^= (arr[k*4 + j] as u32 as i128) << (32 * j);
                }
                vec.push(integer);
            }
        }
        vec
    }

    pub fn popcount(&self) -> i32 {
        let mut sum: i32 = 0;
        for block_index in 0..self.blocks.len() {
            let arr = self.extract_block(block_index);
            for j in 0..8 {
                sum += arr[j].count_ones() as i32;
            }
        }
        sum
    }

    fn extract_block(&self, block: usize) -> [i32; 8] {
        self.blocks[block].extract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let vec = SIMDVector::new(100);
        assert_eq!(vec.blocks.len(), 1);
        assert_eq!(vec.size(), SIMDVector::BLOCK_SIZE);
    }

    #[test]
    fn test_new_block_size() {
        let vec = SIMDVector::new_block_size(3);
        assert_eq!(vec.blocks.len(), 3);
    }

    #[test]
    fn test_from_integer_vec() {
        let vec = SIMDVector::from_integer_vec(vec![5]);
        assert_eq!(vec.get(0), true);
        assert_eq!(vec.get(1), false);
        assert_eq!(vec.get(2), true);
    }

    #[test]
    fn test_size() {
        let vec = SIMDVector::new_block_size(2);
        assert_eq!(vec.size(), 2 * SIMDVector::BLOCK_SIZE);
    }

    #[test]
    fn test_xor_bit() {
        let mut vec = SIMDVector::new(10);
        vec.xor_bit(5);
        assert!(vec.get(5));
    }

    #[test]
    fn test_get_first_one() {
        let mut vec = SIMDVector::new(100);
        vec.xor_bit(42);
        assert_eq!(vec.get_first_one(), 42);
    }

    #[test]
    fn test_get_all_ones() {
        let mut vec = SIMDVector::new(10);

        vec.xor_bit(1);
        vec.xor_bit(7);
        vec.xor_bit(8);

        assert_eq!(vec.get_all_ones(10), vec![1, 7, 8]);
    }

    #[test]
    fn test_xor() {
        let mut vec1 = SIMDVector::new(10);
        let mut vec2 = SIMDVector::new(10);
        let expected = vec![5, 7];

        vec1.xor_bit(5);
        vec2.xor_bit(7);
        vec1.xor(&vec2);

        assert_eq!(vec1.get_all_ones(10), expected);
    }

    #[test]
    fn test_and() {
        let mut vec1 = SIMDVector::new(10);
        let mut vec2 = SIMDVector::new(10);
        let expected = vec![3];

        vec1.xor_bit(3);
        vec1.xor_bit(8);
        vec2.xor_bit(3);
        vec1.and(&vec2);

        assert_eq!(vec1.get_all_ones(10), expected);
    }

    #[test]
    fn test_negate() {
        let mut vec = SIMDVector::new(5);
        let expected = vec![0, 2, 3];

        vec.xor_bit(1);
        vec.xor_bit(4);
        vec.negate();

        assert_eq!(vec.get_all_ones(5), expected);
    }

    #[test]
    fn test_get_boolean_vec() {
        let mut vec = SIMDVector::new(5);
        vec.xor_bit(1);
        vec.xor_bit(3);

        let bool_vec = vec.get_boolean_vec();
        
        assert!(!bool_vec[0]);
        assert!(bool_vec[1]);
        assert!(!bool_vec[2]);
        assert!(bool_vec[3]);
        assert!(!bool_vec[4]);
    }

    #[test]
    fn test_get_integer_vec() {
        let input = vec![42, 100];
        let vec = SIMDVector::from_integer_vec(input.clone());
        let output = vec.get_integer_vec();
        assert_eq!(output[0], 42);
        assert_eq!(output[1], 100);
    }

    #[test]
    fn test_popcount() {
        let mut vec = SIMDVector::new(10);
        vec.xor_bit(2);
        vec.xor_bit(6);
        vec.xor_bit(7);
        assert_eq!(vec.popcount(), 3);
    }
}


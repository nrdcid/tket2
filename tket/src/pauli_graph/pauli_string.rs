use crate::pauli_graph::simd_vector::SIMDVector;

/// Represents a Pauli string and its sign
#[derive(Debug, Clone)]
pub struct PauliString {
    /// Z component
    pub z: SIMDVector,
    /// X component
    pub x: SIMDVector,
    /// Sign/Phase
    pub sign: bool,
}

impl PauliString {
    pub fn new(z: SIMDVector, x: SIMDVector, sign: bool) -> Self {
        PauliString { z, x, sign }
    }
    
    /// Returns true if self commutes with other pauli string
    pub fn commutes_with(&self, other: &Self) -> bool {
        let mut z1_and_x2 = self.z.clone();
        z1_and_x2.and(&other.x);
        let mut anticommuting = self.x.clone();
        anticommuting.and(&other.z);
        anticommuting.xor(&z1_and_x2);
        anticommuting.popcount() % 2 == 0
    }
    
    /// Multiplies self with another pauli string
    pub fn multiply(&mut self, other: &Self) {
        let mut z1_and_x2 = self.z.clone();
        z1_and_x2.and(&other.x);
        let mut anticommuting = self.x.clone();
        anticommuting.and(&other.z);
        anticommuting.xor(&z1_and_x2);
        
        self.x.xor(&other.x);
        self.z.xor(&other.z);

        z1_and_x2.xor(&self.x);
        z1_and_x2.xor(&self.z);
        z1_and_x2.and(&anticommuting);
        
        let anticommute_count = anticommuting.popcount();
        let cross_count = z1_and_x2.popcount();
        let total_phase = (anticommute_count + 2 * cross_count) % 4;
        let phase_flip = total_phase > 1;

        self.sign ^= other.sign ^ phase_flip;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let z = SIMDVector::from_integer_vec(vec![1]);
        let x = SIMDVector::from_integer_vec(vec![0]);
        let ps = PauliString::new(z, x, false);

        assert!(!ps.sign);
        assert!(ps.z.get(0));
        assert!(!ps.x.get(0));
    }

    #[test]
    fn test_commutes_with() {
        // ZI
        let z_part = SIMDVector::from_integer_vec(vec![1]);
        let x_part = SIMDVector::from_integer_vec(vec![0]);
        let zi = PauliString::new(z_part, x_part, false);

        // XI
        let z_part = SIMDVector::from_integer_vec(vec![0]);
        let x_part = SIMDVector::from_integer_vec(vec![1]);
        let xi = PauliString::new(z_part, x_part, false);
        
        // IX
        let z_part = SIMDVector::from_integer_vec(vec![0]);
        let x_part = SIMDVector::from_integer_vec(vec![2]);
        let ix = PauliString::new(z_part, x_part, false);
        
        // ZI and XI anti-commute
        assert!(!zi.commutes_with(&xi));
        // ZI and IX commute
        assert!(zi.commutes_with(&ix));
    }

    #[test]
    fn test_multiply() {
        // XX
        let z_part = SIMDVector::from_integer_vec(vec![0]);
        let x_part = SIMDVector::from_integer_vec(vec![3]);
        let mut string_1 = PauliString::new(z_part, x_part, false);

        // ZY
        let z_part = SIMDVector::from_integer_vec(vec![3]);
        let x_part = SIMDVector::from_integer_vec(vec![2]);
        let string_2 = PauliString::new(z_part, x_part, false);

        string_1.multiply(&string_2);

        // XX * ZY = YZ
        assert!(string_1.z.get(0));
        assert!(string_1.z.get(1));
        assert!(string_1.x.get(0));
        assert!(!string_1.x.get(1));
        assert!(!string_1.sign);
    }
}


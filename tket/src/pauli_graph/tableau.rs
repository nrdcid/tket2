/// Temporary clifford tableau implementation
/// Use rupauli instead when tket is ready to depend on it
use crate::pauli_graph::simd_vector::SIMDVector;
use crate::pauli_graph::pauli_string::PauliString;
use crate::TketOp;

#[derive(Debug, Clone)]
pub struct RowMajorTableau {
    pub nb_qubits: usize,
    pub z: Vec<SIMDVector>,
    pub x: Vec<SIMDVector>,
    pub signs: SIMDVector,
}

impl RowMajorTableau {
    pub fn new(nb_qubits: usize) -> Self {
        RowMajorTableau {
            nb_qubits: nb_qubits,
            z: RowMajorTableau::init_z(nb_qubits),
            x: RowMajorTableau::init_x(nb_qubits),
            signs: SIMDVector::new(nb_qubits << 1),
        }
    }

    fn init_z(nb_qubits: usize) -> Vec<SIMDVector> {
        let mut vec = Vec::new();
        for i in 0..nb_qubits {
            let mut bv = SIMDVector::new(nb_qubits << 1);
            bv.xor_bit(i);
            vec.push(bv);
        }
        vec
    }

    fn init_x(nb_qubits: usize) -> Vec<SIMDVector> {
        let mut vec = Vec::new();
        for i in 0..nb_qubits {
            let mut bv = SIMDVector::new(nb_qubits << 1);
            bv.xor_bit(i + nb_qubits);
            vec.push(bv);
        }
        vec
    }

    pub fn append(&mut self, gate: TketOp, qubits: Vec<usize>) {
        match gate {
            TketOp::X => {
                self.signs.xor(&self.z[qubits[0]]);
            }
            TketOp::Z => {
                self.signs.xor(&self.x[qubits[0]]);
            }
            TketOp::S => {
                let mut a = self.z[qubits[0]].clone();
                a.and(&self.x[qubits[0]]);
                self.signs.xor(&a);
                self.z[qubits[0]].xor(&self.x[qubits[0]]);
            }
            TketOp::V => {
                let mut a = self.x[qubits[0]].clone();
                a.negate();
                a.and(&self.z[qubits[0]]);
                self.signs.xor(&a);
                self.x[qubits[0]].xor(&self.z[qubits[0]]);
            }
            TketOp::H => {
                self.append(TketOp::S, qubits.clone());
                self.append(TketOp::V, qubits.clone());
                self.append(TketOp::S, qubits.clone());
            }
            TketOp::CX => {
                let mut a =  self.z[qubits[0]].clone();
                a.negate();
                a.xor(&self.x[qubits[1]]);
                a.and(&self.z[qubits[1]]);
                a.and(&self.x[qubits[0]]);
                self.signs.xor(&a);
                let a = self.z[qubits[1]].clone();
                self.z[qubits[0]].xor(&a);
                let a = self.x[qubits[0]].clone();
                self.x[qubits[1]].xor(&a);
            }
            TketOp::CZ => {
                self.append(TketOp::S, vec![qubits[0]]);
                self.append(TketOp::S, vec![qubits[1]]);
                self.append(TketOp::CX, qubits.clone());
                self.append(TketOp::S, vec![qubits[1]]);
                self.append(TketOp::Z, vec![qubits[1]]);
                self.append(TketOp::CX, qubits.clone());
            }
            _ => {
                panic!("Cannot construct a clifford tableau: {:?}", gate);
            }
        }
    }

    pub fn append_cz(&mut self, qubits: Vec<usize>) {
        self.append(TketOp::S, vec![qubits[0]]);
        self.append(TketOp::S, vec![qubits[1]]);
        self.append(TketOp::CX, qubits.to_vec());
        self.append(TketOp::S, vec![qubits[1]]);
        self.append(TketOp::Z, vec![qubits[1]]);
        self.append(TketOp::CX, qubits);
    }
    
    pub fn extract_pauli_string(&self, col: usize) -> PauliString {
        let mut z = SIMDVector::new(self.nb_qubits);
        let mut x = SIMDVector::new(self.nb_qubits);
        for i in 0..self.nb_qubits {
            if self.z[i].get(col) { z.xor_bit(i); }
            if self.x[i].get(col) { x.xor_bit(i); }
        }
        PauliString::new(z, x, self.signs.get(col))
    }

    pub fn insert_pauli_string(&mut self, p: PauliString, col: usize) {
        let p_x = p.x.get_boolean_vec();
        let p_z = p.z.get_boolean_vec();
        for i in 0..self.nb_qubits {
            if p_z[i] ^ self.z[i].get(col) {
                self.z[i].xor_bit(col);
            }
            if p_x[i] ^ self.x[i].get(col) {
                self.x[i].xor_bit(col);
            }
        }
        if p.sign ^ self.signs.get(col) {
            self.signs.xor_bit(col);
        }
    }
    
    pub fn prepend(&mut self, gate: TketOp, qubits: Vec<usize>) {
        match gate {
            TketOp::X => {
                self.signs.xor_bit(qubits[0]);
            }
            TketOp::Z => {
                self.signs.xor_bit(qubits[0] + self.nb_qubits);
            }
            TketOp::S => {
                let stab = self.extract_pauli_string(qubits[0]);
                let mut destab = self.extract_pauli_string(qubits[0] + self.nb_qubits);
                destab.multiply(&stab);
                self.insert_pauli_string(destab, qubits[0] + self.nb_qubits);
            }
            TketOp::H => {
                let stab = self.extract_pauli_string(qubits[0]);
                let destab = self.extract_pauli_string(qubits[0] + self.nb_qubits);
                self.insert_pauli_string(destab, qubits[0]);
                self.insert_pauli_string(stab, qubits[0] + self.nb_qubits);
            }
            TketOp::CX => {
                let stab_ctrl = self.extract_pauli_string(qubits[0]);
                let mut stab_targ = self.extract_pauli_string(qubits[1]);
                let mut destab_ctrl = self.extract_pauli_string(qubits[0] + self.nb_qubits);
                let destab_targ = self.extract_pauli_string(qubits[1] + self.nb_qubits);
                stab_targ.multiply(&stab_ctrl);
                destab_ctrl.multiply(&destab_targ);
                self.insert_pauli_string(stab_targ, qubits[1]);
                self.insert_pauli_string(destab_ctrl, qubits[0] + self.nb_qubits);
            }
            _ => {
                panic!("Cannot construct a clifford tableau: {:?}", gate);
            }
        }
    }
}


#[derive(Debug, Clone)]
pub struct ColMajorTableau {
    pub nb_qubits: usize,
    pub stabs: Vec<PauliString>,
    pub destabs: Vec<PauliString>,
}

impl ColMajorTableau {
    pub fn new(nb_qubits: usize) -> Self {
        ColMajorTableau {
            nb_qubits: nb_qubits,
            stabs: ColMajorTableau::init_stabs(nb_qubits),
            destabs: ColMajorTableau::init_destabs(nb_qubits),
        }
    }

    fn init_stabs(nb_qubits: usize) -> Vec<PauliString> {
        let mut vec = Vec::new();
        for i in 0..nb_qubits {
            let mut bv = SIMDVector::new(nb_qubits);
            bv.xor_bit(i);
            vec.push(PauliString::new(bv, SIMDVector::new(nb_qubits), false));
        }
        vec
    }

    fn init_destabs(nb_qubits: usize) -> Vec<PauliString> {
        let mut vec = Vec::new();
        for i in 0..nb_qubits {
            let mut bv = SIMDVector::new(nb_qubits);
            bv.xor_bit(i);
            vec.push(PauliString::new(SIMDVector::new(nb_qubits), bv, false));
        }
        vec
    }

    pub fn prepend(&mut self, gate: TketOp, qubits: Vec<usize>) {
        match gate {
            TketOp::X => {
                self.stabs[qubits[0]].sign ^= true;
            }
            TketOp::Z => {
                self.destabs[qubits[0]].sign ^= true;
            }
            TketOp::V => {
                self.stabs[qubits[0]].multiply(&self.destabs[qubits[0]]);
            }
            TketOp::S => {
                self.destabs[qubits[0]].multiply(&self.stabs[qubits[0]]);
            }
            TketOp::H => {
                self.prepend(TketOp::S, qubits.clone());
                self.prepend(TketOp::V, qubits.clone());
                self.prepend(TketOp::S, qubits.clone());
            }
            TketOp::CX => {
                let p = self.stabs[qubits[0]].clone();
                self.stabs[qubits[1]].multiply(&p);
                let p = self.destabs[qubits[1]].clone();
                self.destabs[qubits[0]].multiply(&p);
            }
            _ => {
                panic!("Cannot construct a clifford tableau: {:?}", gate);
            }
        }
    }
}

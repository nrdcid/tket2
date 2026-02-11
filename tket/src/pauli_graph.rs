pub mod tableau;
pub mod pauli_string;
pub mod simd_vector;

use hugr::{HugrView, Wire};
use hugr::ops::{OpTag, OpTrait};
use hugr_core::hugr::internal::{HugrInternals, PortgraphNodeMap};
use std::collections::HashMap;
use petgraph::stable_graph::StableDiGraph;
use petgraph::visit as pv;
use crate::{Circuit, TketOp};

use crate::pauli_graph::{
    tableau::ColMajorTableau,
    pauli_string::PauliString,
};

#[derive(Debug, Clone)]
pub struct PauliGadget {
    pub string: PauliString,
    pub angle: f64,
}

impl PauliGadget {
    pub fn new(string: PauliString, angle: f64) -> Self {
        Self { string, angle }
    }

    pub fn is_clifford(&self) -> Option<u8> {
        let normalized = (self.angle % 2.0 + 2.0) % 2.0;
        let quarter = normalized * 2.0;
        let rounded = quarter.round();

        if (quarter - rounded).abs() < 1e-10 {
            Some((rounded as i32 % 4) as u8)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct PauliGraph {
    pub nb_qubits: usize,
    pub pauli_gadgets: Vec<PauliGadget>,
    pub tab: ColMajorTableau,
    pub graph: StableDiGraph::<(), ()>,
}

impl PauliGraph {
    pub fn new(nb_qubits: usize) -> Self {
        PauliGraph {
            nb_qubits: nb_qubits,
            pauli_gadgets: Vec::new(),
            tab: ColMajorTableau::new(nb_qubits),
            graph: StableDiGraph::<(), ()>::new(),
        }
    }

    pub fn from(circ: &Circuit) -> PauliGraph {
        let mut pauli_graph = PauliGraph::new(circ.qubit_count());

        let mut wire_to_qubit: HashMap<Wire, usize> = HashMap::new();
        for (i, (_, port, _)) in circ.qubits().enumerate() {
            let wire = Wire::new(circ.input_node(), port);
            wire_to_qubit.insert(wire, i);
        }

        let (region, node_map) = circ.hugr().region_portgraph(circ.parent());
        let mut topo = pv::Topo::new(&region);
        while let Some(pg_node) = topo.next(&region) {
            let node = node_map.from_portgraph(pg_node);
            let optype = circ.hugr().get_optype(node);

            let tag = optype.tag();
            if tag == OpTag::Input || tag == OpTag::Output {
                continue;
            }

            if let Some(tkop) = optype.cast::<TketOp>() {
                let mut qubits: Vec<usize> = Vec::new();
                for in_port in circ.hugr().node_inputs(node) {
                    if let Some((src_node, src_port)) = circ.hugr().single_linked_output(node, in_port) {
                        let wire = Wire::new(src_node, src_port);
                        if let Some(&qubit_idx) = wire_to_qubit.get(&wire) {
                            qubits.push(qubit_idx);
                        }
                    }
                }

                let mut qubit_iter = qubits.iter();
                for out_port in circ.hugr().node_outputs(node) {
                    if let Some(&qubit_idx) = qubit_iter.next() {
                        let wire = Wire::new(node, out_port);
                        wire_to_qubit.insert(wire, qubit_idx);
                    }
                }

                match tkop {
                    TketOp::H => {
                        pauli_graph.tab.prepend(TketOp::H, vec![qubits[0]]);
                    }
                    TketOp::X => {
                        pauli_graph.tab.prepend(TketOp::X, vec![qubits[0]]);
                    }
                    TketOp::Z => {
                        pauli_graph.tab.prepend(TketOp::Z, vec![qubits[0]]);
                    }
                    TketOp::S => {
                        pauli_graph.tab.prepend(TketOp::S, vec![qubits[0]]);
                        pauli_graph.tab.prepend(TketOp::Z, vec![qubits[0]]);
                    }
                    TketOp::CX => {
                        pauli_graph.tab.prepend(TketOp::CX, qubits.clone());
                    }
                    TketOp::T => {
                        let pauli_gadget = PauliGadget::new(pauli_graph.tab.stabs[qubits[0]].clone(), 0.25);
                        pauli_graph.pauli_gadgets.push(pauli_gadget);
                        let u = pauli_graph.graph.add_node(());
                        let node_indices = pauli_graph.graph.node_indices().collect::<Vec<_>>();
                        for v in node_indices {
                            if !pauli_graph.pauli_gadgets[v.index()].string.commutes_with(&pauli_graph.pauli_gadgets[u.index()].string) {
                                pauli_graph.graph.add_edge(v, u, ());
                            }
                        }
                    }
                    _ => {
                        panic!("Cannot construct a pauli graph with gate: {:?}", tkop);
                    }
                }
            } else {
                panic!("Cannot construct a pauli graph with gate: {:?}", optype);
            }
        }

        pauli_graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::build_simple_circuit;

    #[test]
    fn test_from_circuit_with_t_gates() {
        let circ = build_simple_circuit(2, |circ| {
            circ.append(TketOp::T, [0])?;
            circ.append(TketOp::H, [0])?;
            circ.append(TketOp::CX, [0, 1])?;
            circ.append(TketOp::T, [0])?;
            circ.append(TketOp::T, [1])?;
            Ok(())
        })
        .unwrap();

        let pauli_graph = PauliGraph::from(&circ);
        
        assert_eq!(pauli_graph.nb_qubits, 2);
        assert_eq!(pauli_graph.pauli_gadgets.len(), 3);
        assert_eq!(pauli_graph.graph.node_count(), 3);


        // gadget_0 = {ZI, 0.25}
        let gadget_0 = &pauli_graph.pauli_gadgets[0];
        assert!(gadget_0.string.z.get(0));
        assert!(!gadget_0.string.x.get(0));
        assert!(!gadget_0.string.z.get(1));
        assert!(!gadget_0.string.x.get(1));
        assert_eq!(gadget_0.angle, 0.25);

        // gadget_1 = {XZ, 0.25}
        let gadget_1 = &pauli_graph.pauli_gadgets[1];
        assert!(!gadget_1.string.z.get(0));
        assert!(gadget_1.string.x.get(0));
        assert!(gadget_1.string.z.get(1));
        assert!(!gadget_1.string.x.get(1));
        assert_eq!(gadget_1.angle, 0.25);

        // gadget_2 = {XI, 0.25}
        let gadget_2 = &pauli_graph.pauli_gadgets[2];
        assert!(!gadget_2.string.z.get(0));
        assert!(gadget_2.string.x.get(0));
        assert!(!gadget_2.string.z.get(1));
        assert!(!gadget_2.string.x.get(1));
        assert_eq!(gadget_2.angle, 0.25);

        use petgraph::stable_graph::NodeIndex;
        let n0 = NodeIndex::new(0);
        let n1 = NodeIndex::new(1);
        let n2 = NodeIndex::new(2);
        
        // Should have edges (0->1) and (0->2), but not (1->2)
        assert_eq!(pauli_graph.graph.edge_count(), 2);
        assert!(pauli_graph.graph.contains_edge(n0, n1));
        assert!(pauli_graph.graph.contains_edge(n0, n2));
        assert!(!pauli_graph.graph.contains_edge(n1, n2));
    }
}


use crate::passes::guppy::NormalizeGuppyErrors;
use crate::passes::NormalizeGuppy;
use crate::serialize::pytket::{
    default_decoder_config,
    default_encoder_config,
    EncodeOptions,
    EncodedCircuit,
    PytketDecodeError, PytketEncodeError,
};
use crate::{Circuit, CircuitError};
use hugr::algorithms::inline_funcs::InlineFuncsError;
use hugr::hugr::ValidationError;

use hugr_core::hugr::internal::{HugrInternals, PortgraphNodeMap};
use pauli_graph::{BlackBoxData, GateData, GateType, Op, PauliGraph, PauliGraphPass};
use basic_passes::CanonicalFormPass;
use greedy_synth::{GreedySynthPass, RebaseTQEToZXPass};
use petgraph::visit as pv;
use pg_optimise::{GroupCommutingOpsPass, RotationMergingPass};
use fast_todd::FastTODDPass;

use hugr_passes::inline_acyclic;
use hugr::algorithms::ComposablePass;
use hugr::{Hugr, Node};
use hugr::HugrView;
use hugr::hugr::OpType as TketOp;

use tket_json_rs::circuit_json::{Command, Operation};
use tket_json_rs::register::{Bit, ElementId, Qubit};
use tket_json_rs::{OpType, SerialCircuit};

use std::sync::Arc;

pub fn count_t_gates_in_mermaid_string(input: &str) ->  usize {
    input.matches("tket.quantum.T").count()
}

#[derive(Clone, Debug)]
pub struct GlobalTResynthesis {
    ancilla_budget: usize,
}

impl Default for GlobalTResynthesis {
    fn default() -> Self {
        Self { ancilla_budget: 0 }
    }
}

impl GlobalTResynthesis {
    pub fn with_ancilla_budget(&mut self, ancilla_budget: usize) -> &mut Self {
        self.ancilla_budget = ancilla_budget;
        self
    }
}

impl ComposablePass<Hugr> for GlobalTResynthesis {
    type Error = GlobalTResynthesisErrors;
    type Result = ();
    fn run(&self, hugr: &mut Hugr) -> Result<Self::Result, Self::Error> {
        inline_acyclic(hugr, |_, _| true).unwrap();
        NormalizeGuppy::default().run(hugr)?;

        let mut circ = Circuit::try_new(hugr.clone())?;

        let mermaid_string = circ.mermaid_string();
        println!("Post inline and normalize T count {:?}", count_t_gates_in_mermaid_string(&mermaid_string));
        
        let encode_options = EncodeOptions::new()
            .with_subcircuits(true)
            .with_config(default_encoder_config());

        let mut encoded_circs = EncodedCircuit::new(&circ, encode_options)?;

        for (_, serial_circ) in encoded_circs.iter_mut() {
            let pauli_graph = serial_circuit_to_pauli_graph(serial_circ)?;

            let mut t_count = 0;
            for cmd in serial_circ.commands.iter() {
                if cmd.op.op_type == OpType::T || cmd.op.op_type == OpType::Tdg {
                    t_count += 1;
                }
            }
            println!("T-count inside encoded region: {}", t_count);

            let canonical_pass = CanonicalFormPass::new().with_forward(true);
            let grouping_pass = GroupCommutingOpsPass::new();
            let rotation_merging_pass = RotationMergingPass::new();
            let fast_todd_pass = FastTODDPass::new();
            let synth_pass = GreedySynthPass::new(100, 100, 100);
            let rebase_pass = RebaseTQEToZXPass::new()
                .with_allowed_tqes(vec![GateType::ZX]);

            let pauli_graph = canonical_pass.transform(&pauli_graph);
            let pauli_graph = rotation_merging_pass.transform(&pauli_graph);
            let pauli_graph = grouping_pass.transform(&pauli_graph);
            let pauli_graph = synth_pass.transform(&pauli_graph);
            let pauli_graph = rebase_pass.transform(&pauli_graph);

            serial_circ.commands = pauli_graph_to_cmds(pauli_graph, serial_circ)?;

            t_count = 0;
            for cmd in serial_circ.commands.iter() {
                if cmd.op.op_type == OpType::T || cmd.op.op_type == OpType::Tdg {
                    t_count += 1;
                }
            }

            println!("T-count inside encoded region after optimisation: {}", t_count);
        }

        encoded_circs
            .reassemble_inplace(
                circ.hugr_mut(),
                Some(Arc::new(default_decoder_config())),
            )?;

        circ.hugr().validate()?;

        let mermaid_string = circ.mermaid_string();
        println!("ReassembledT count {:?}", count_t_gates_in_mermaid_string(&mermaid_string));

        *hugr = circ.into_hugr();

        Ok(())
    }
}

/// Errors that can occur during the global-t resynthesis
#[derive(derive_more::Error, Debug, derive_more::Display, derive_more::From)]
pub enum GlobalTResynthesisErrors {
    /// Error inlining functions
    #[from]
    InlineError(InlineFuncsError),
    /// Error normalizing the hugr
    #[from]
    NormalizeError(NormalizeGuppyErrors),
    /// Error loading the circuit.
    #[display("Error loading the circuit: {_0}")]
    #[from]
    CircuitLoadError(CircuitError),
    /// Error encoding the circuit.
    #[display("Error encoding the circuit: {_0}")]
    #[from]
    CircuitEncodeError(PytketEncodeError<Node>),
    /// Error converting between pauli graph and serial circuit
    #[from]
    ConversionError(ConversionError),
    /// Error reassembling the circuit
    #[display("Error reassembling the circuit: {_0}")]
    #[from]
    ReassemblyError(PytketDecodeError),
    /// Error validating the reassembled circuit.
    #[from]
    ValidationError(ValidationError<Node>),
}

fn serial_circuit_to_pauli_graph(
    serial_circuit: &mut SerialCircuit,
) -> Result<PauliGraph, ConversionError> {
    let num_qubits = serial_circuit.qubits.len();
    let qubits = &serial_circuit.qubits;
    let bits = &serial_circuit.bits;
    let ops: Vec<Op> = serial_circuit.commands
        .iter()
        .try_fold(Vec::new(), |mut acc, cmd| {
            acc.extend(cmd_to_op(cmd, qubits, bits)?);
            Ok(acc)
        })?;

    Ok(PauliGraph::new(num_qubits).with_ops(ops))
}

fn pauli_graph_to_cmds(
    pauli_graph: PauliGraph,
    serial_circuit: &mut SerialCircuit,
) -> Result<Vec<Command<String>>, ConversionError> {
    let qubits = &serial_circuit.qubits;
    let bits = &serial_circuit.bits;

    pauli_graph
        .get_ops()
        .iter()
        .try_fold(Vec::new(), |mut acc, op| {
            acc.extend(op_to_cmd(op, qubits, bits)?);
            Ok(acc)
        })
}

fn cmd_to_op(
    cmd: &Command<String>,
    qubits: &[Qubit],
    bits: &[Bit],
) -> Result<Vec<Op>, ConversionError> {
    let qubits = cmd
        .args
        .iter()
        .filter_map(|id| qubits.iter().position(|q| q.id == *id))
        .collect();
    let _bits: Vec<usize> = cmd
        .args
        .iter()
        .filter_map(|id| bits.iter().position(|b| b.id == *id))
        .collect();
    let params = cmd.op.params.clone();

    // TODO: add support for classical gates
    match cmd.op.op_type {
        OpType::H => Ok(vec![Op::Gate {
            data: GateData::new(GateType::H, qubits),
        }]),
        OpType::CX => Ok(vec![Op::Gate {
            data: GateData::new(GateType::ZX, qubits),
        }]),
        OpType::CY => Ok(vec![Op::Gate {
            data: GateData::new(GateType::ZY, qubits),
        }]),
        OpType::CZ => Ok(vec![Op::Gate {
            data: GateData::new(GateType::ZZ, qubits),
        }]),
        OpType::CRz => {
            let angle_string = params
                .ok_or(ConversionError::RotationAngleRequired(cmd.op.op_type))?;
            
            // TODO: this is a bit cumbersome, think about refactoring to parse_float
            // and then putting the float into a vec
            let half_angle: Vec<f64> = parse_floats(angle_string)?
                .iter()
                .map(|a| a / 2.0)
                .collect();

            let negative_half_angle: Vec<f64> = half_angle
                .iter()
                .map(|a| -a)
                .collect();

            Ok(vec![
                Op::Gate { data: GateData::new(GateType::RZ, vec![qubits[1]]).with_params(half_angle) },
                Op::Gate { data: GateData::new(GateType::ZX, qubits.clone()) },
                Op::Gate { data: GateData::new(GateType::RZ, vec![qubits[1]]).with_params(negative_half_angle) },
                Op::Gate { data: GateData::new(GateType::ZX, qubits) }]
                )
        },
        OpType::T => Ok(vec![Op::Gate {
            data: GateData::new(GateType::RZ, qubits).with_params(vec![0.25]),
        }]),
        OpType::Tdg => Ok(vec![Op::Gate {
            data: GateData::new(GateType::RZ, qubits).with_params(vec![-0.25]),
        }]),
        OpType::S => Ok(vec![Op::Gate {
            data: GateData::new(GateType::S, qubits),
        }]),
        OpType::Sdg => Ok(vec![Op::Gate {
            data: GateData::new(GateType::Sdg, qubits),
        }]),
        OpType::V => Ok(vec![Op::Gate {
            data: GateData::new(GateType::V, qubits),
        }]),
        OpType::Vdg => Ok(vec![Op::Gate {
            data: GateData::new(GateType::Vdg, qubits),
        }]),
        OpType::X => Ok(vec![Op::Gate {
            data: GateData::new(GateType::X, qubits),
        }]),
        OpType::Y => Ok(vec![Op::Gate {
            data: GateData::new(GateType::Y, qubits),
        }]),
        OpType::Z => Ok(vec![Op::Gate {
            data: GateData::new(GateType::Z, qubits),
        }]),
        OpType::Rx => {
            let angle_string = params
                .ok_or(ConversionError::RotationAngleRequired(cmd.op.op_type))?;
            let angle = parse_floats(angle_string)?;
            
            Ok(vec![Op::Gate {
                data: GateData::new(GateType::RX, qubits).with_params(angle),
            }])
        }
        OpType::Ry => {
            let angle_string = params
                .ok_or(ConversionError::RotationAngleRequired(cmd.op.op_type))?;
            let angle = parse_floats(angle_string)?;
            
            Ok(vec![Op::Gate {
                data: GateData::new(GateType::RY, qubits).with_params(angle),
            }])
        }
        OpType::Rz => {
            let angle_string = params
                .ok_or(ConversionError::RotationAngleRequired(cmd.op.op_type))?;
            let angle = parse_floats(angle_string)?;
            
            Ok(vec![Op::Gate {
                data: GateData::new(GateType::RZ, qubits).with_params(angle),
            }])
        }
        // TODO: it may be possible to improve T-count with given ancilla budget by 
        // pushing CCX decomposition into the FastTODD pass
        // this requires CCX as a gate in pauli graph interface
        OpType::CCX => Ok(vec![
            Op::Gate { data: GateData::new(GateType::H, vec![qubits[2]]) },
            Op::Gate { data: GateData::new(GateType::ZX, vec![qubits[1], qubits[2]]) },
            Op::Gate { data: GateData::new(GateType::Z, vec![qubits[2]]).with_params(vec![-0.25]) },
            Op::Gate { data: GateData::new(GateType::ZX, vec![qubits[0], qubits[2]]) },
            Op::Gate { data: GateData::new(GateType::Z, vec![qubits[2]]).with_params(vec![0.25]) },
            Op::Gate { data: GateData::new(GateType::ZX, vec![qubits[1], qubits[2]]) },
            Op::Gate { data: GateData::new(GateType::Z, vec![qubits[2]]).with_params(vec![-0.25]) },
            Op::Gate { data: GateData::new(GateType::ZX, vec![qubits[0], qubits[2]]) },
            Op::Gate { data: GateData::new(GateType::Z, vec![qubits[1]]).with_params(vec![0.25]) },
            Op::Gate { data: GateData::new(GateType::Z, vec![qubits[2]]).with_params(vec![0.25]) },
            Op::Gate { data: GateData::new(GateType::H, vec![qubits[2]]) },
            Op::Gate { data: GateData::new(GateType::ZX, vec![qubits[0], qubits[1]]) },
            Op::Gate { data: GateData::new(GateType::Z, vec![qubits[0]]).with_params(vec![0.25]) },
            Op::Gate { data: GateData::new(GateType::Z, vec![qubits[1]]).with_params(vec![-0.25]) },
            Op::Gate { data: GateData::new(GateType::ZX, vec![qubits[0], qubits[1]]) }
        ]),
        // TODO: check if we can have barriers with classical bits
        OpType::Barrier => {
            Ok(vec![Op::BlackBox {
            data: BlackBoxData::new(qubits, "".to_string())
        }])}
        OpType::Barrier => Ok(vec![]),
        _ => Err(ConversionError::UnsupportedOpType(cmd.op.op_type)),
    }
}

fn op_to_cmd(
    op: &Op,
    qubit_map: &[Qubit],
    bit_map: &[Bit],
) -> Result<Vec<Command<String>>, ConversionError> {
    // TODO: add support for other gates
    match op {
        Op::Gate { data } => match data.get_gate_type() {
            // TODO: think about refactoring the single qubit gates
            // (they all have the same conversion)
            GateType::H => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::H),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::S => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::S),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::Sdg => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::Sdg),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::Z => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::Z),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::V => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::V),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::Vdg => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::Vdg),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::X => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::X),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::Y => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::Y),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::Z => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::Z),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::ZX => {
                let qubits = apply_map(qubit_map, data.get_args());
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::CX),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::XZ => {
                let mut qubits = apply_map(qubit_map, data.get_args());
                qubits.reverse();
                Ok(vec![Command {
                    op: Operation::from_optype(OpType::CX),
                    args: qubits,
                    opgroup: None,
                }])
            }
            GateType::RX => {
                let qubits = apply_map(qubit_map, data.get_args());
                let params = data.get_params();

                if params.len() != 1 {
                    let msg = format!("RX must have 1 parameter, found {}", params.len());
                    return Err(ConversionError::ImpossibleParams(msg));
                }

                match params[0] {
                    0.25 => Ok(vec![
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::T),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits,
                            opgroup: None,
                        }
                    ]),
                    -0.25 => Ok(vec![
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::T),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits,
                            opgroup: None,
                        }
                    ]),
                    0.5 => Ok(vec![Command {
                        op: Operation::from_optype(OpType::V),
                        args: qubits,
                        opgroup: None,
                    }]),
                    -0.5 => Ok(vec![Command {
                        op: Operation::from_optype(OpType::Vdg),
                        args: qubits,
                        opgroup: None,
                    }]),
                    _ => {
                        panic!("RX {} not in Clifford + T", params[0]);
                    }
                }
            }
            GateType::RY => {
                let qubits = apply_map(qubit_map, data.get_args());
                let params = data.get_params();

                if params.len() != 1 {
                    let msg = format!("RX must have 1 parameter, found {}", params.len());
                    return Err(ConversionError::ImpossibleParams(msg));
                }

                match params[0] {
                    0.25 => Ok(vec![
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::Sdg),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::T),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::S),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits,
                            opgroup: None,
                        }
                    ]),
                -0.25 => Ok(vec![
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::Sdg),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::Tdg),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::S),
                            args: qubits.clone(),
                            opgroup: None,
                        },
                        Command {
                            op: Operation::from_optype(OpType::H),
                            args: qubits,
                            opgroup: None,
                        }
                    ]),
                    _ => { panic!("RY {} not in Clifford + T", params[0]); }
                }
            }
            GateType::RZ => {
                let qubits = apply_map(qubit_map, data.get_args());
                let params = data.get_params();

                if params.len() != 1 {
                    let msg = format!("RZ must have 1 parameter, found {}", params.len());
                    return Err(ConversionError::ImpossibleParams(msg));
                }

                match params[0] {
                    0.25 => Ok(vec![Command {
                        op: Operation::from_optype(OpType::T),
                        args: qubits,
                        opgroup: None,
                    }]),
                    -0.25 => Ok(vec![Command {
                        op: Operation::from_optype(OpType::Tdg),
                        args: qubits,
                        opgroup: None,
                    }]),
                    0.5 => Ok(vec![Command {
                        op: Operation::from_optype(OpType::S),
                        args: qubits,
                        opgroup: None,
                    }]),
                    -0.5 => Ok(vec![Command {
                        op: Operation::from_optype(OpType::Sdg),
                        args: qubits,
                        opgroup: None,
                    }]),
                    _ => panic!("arbitrary RZ gate not in Clifford + T"),
                }
            }
            // TODO: implement this with relabelling
            GateType::SWAP => {
                let qubits = apply_map(qubit_map, data.get_args());
                let reversed_qubits = qubits.iter().rev().cloned().collect();
                Ok(vec![
                    Command {
                        op: Operation::from_optype(OpType::CX),
                        args: qubits.clone(),
                        opgroup: None
                    },
                    Command {
                        op: Operation::from_optype(OpType::CX),
                        args: reversed_qubits,
                        opgroup: None
                    },
                    Command {
                        op: Operation::from_optype(OpType::CX),
                        args: qubits,
                        opgroup: None
                    }
                ])
            }
            // TODO: check the difference between BlackBox gate and Op
            GateType::BlackBox => {
                let qubits = apply_map(qubit_map, data.get_args());

                Ok(vec![Command {
                    op: Operation::from_optype(OpType::Barrier),
                    args: qubits,
                    opgroup: None,
                }])
            }
            gate => Err(ConversionError::UnsupportedGate(gate.clone())),
        },
        Op::BlackBox  { data } => {
            let qubits = apply_map(qubit_map, data.get_qubits());
            let content = data.get_content().to_string();

            if content != "" {
                return Err(ConversionError::UnsupportedBlackBox(content));
            }

            Ok(vec![Command {
                op: Operation::from_optype(OpType::Barrier),
                args: qubits,
                opgroup: None,
            }])
        },
        _ => Err(ConversionError::UnsupportedOp(op.clone())),
    }
}

fn parse_floats(values: Vec<String>) -> Result<Vec<f64>, ConversionError> {
    values
        .into_iter()
        .map(|s| {
            s.parse::<f64>()
                .map_err(|_| ConversionError::SymbolicParameter(s))
        })
        .collect()
}

fn apply_map(map: &[Qubit], indices: &[usize]) -> Vec<ElementId> {
    indices
        .iter()
        .map(|i| map[*i].clone().into())
        .collect()
}

/// Errors that can occur when converting between serial circuit and pauli graph
// TODO: check usage of derive_more::Error and error(ignore)
#[derive(derive_more::Error, Debug, derive_more::Display)]
pub enum ConversionError {
    /// Circuit contains symbolic parameter
    #[display("Error converting to pauli graph: Circuit contains symbolic parameter: {_0}")]
    #[error(ignore)]
    SymbolicParameter(String),
    /// Rotation angle is not specified
    #[display("Error converting to pauli graph: {_0} gate requires a rotation angle")]
    #[error(ignore)]
    RotationAngleRequired(OpType),
    /// Unsupported OpType
    #[display("Error converting to pauli graph: Unsupported OpType: {_0}")]
    #[error(ignore)]
    UnsupportedOpType(OpType),
    /// Unsupported Op
    #[display("Error converting to serial circuit: Unsupported Op: {:?}", _0)]
    #[error(ignore)]
    UnsupportedOp(Op),
    /// Unsupported Gate
    #[display("Error converting to serial circuit: Unsupported Gate: {:?}", _0)]
    #[error(ignore)]
    UnsupportedGate(GateType),
    /// Impossible Params
    #[display("Error converting to serial circuit: {_0}")]
    #[error(ignore)]
    ImpossibleParams(String),
    /// Unsupported BlackBox
    #[display("Error converting to serial circuit: Unsupported BlackBox content: {_0}")]
    #[error(ignore)]
    UnsupportedBlackBox(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    use crate::utils::build_simple_circuit;
    use crate::TketOp;

    #[fixture]
    fn simple_circ() -> Circuit {
        build_simple_circuit(2, |circ| {
            circ.append(TketOp::Z, [0])?;
            circ.append(TketOp::CX, [0, 1])?;
            Ok(())
        })
        .unwrap()
    }

    #[rstest]
    fn initial_test(mut simple_circ: Circuit) {
        GlobalTResynthesis::default()
            .with_ancilla_budget(0)
            .run(simple_circ.hugr_mut())
            .unwrap();

        println!("{}", simple_circ.mermaid_string());
    }
}

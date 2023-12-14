//! Intermediate structure for converting decoding [`SerialCircuit`]s into [`Hugr`]s.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::mem;

use hugr::builder::{CircuitBuilder, Container, DFGBuilder, Dataflow, DataflowHugr};
use hugr::extension::prelude::QB_T;

use hugr::ops::Const;
use hugr::std_extensions::arithmetic::float_types::FLOAT64_TYPE;
use hugr::types::FunctionType;
use hugr::CircuitUnit;
use hugr::{Hugr, Wire};

use serde_json::json;
use tket_json_rs::circuit_json;
use tket_json_rs::circuit_json::SerialCircuit;

use super::op::JsonOp;
use super::{try_param_to_constant, METADATA_IMPLICIT_PERM, METADATA_PHASE};
use crate::extension::{LINEAR_BIT, REGISTRY};
use crate::json::{METADATA_B_REGISTERS, METADATA_Q_REGISTERS};
use crate::symbolic_constant_op;

/// The state of an in-progress [`DFGBuilder`] being built from a [`SerialCircuit`].
///
/// Mostly used to define helper internal methods.
#[derive(Debug, PartialEq)]
pub(super) struct JsonDecoder {
    /// The Hugr being built.
    pub hugr: DFGBuilder<Hugr>,
    /// The dangling wires of the builder.
    /// Used to generate [`CircuitBuilder`]s.
    dangling_wires: Vec<Wire>,
    /// A map from the json registers to flat wire indices.
    register_wire: HashMap<RegisterHash, usize>,
    /// The number of qubits in the circuit.
    num_qubits: usize,
    /// The number of bits in the circuit.
    num_bits: usize,
}

impl JsonDecoder {
    /// Initialize a new [`JsonDecoder`], using the metadata from a [`SerialCircuit`].
    pub fn new(serialcirc: &SerialCircuit) -> Self {
        let num_qubits = serialcirc.qubits.len();
        let num_bits = serialcirc.bits.len();

        // Map each (register name, index) pair to an offset in the signature.
        let mut wire_map: HashMap<RegisterHash, usize> =
            HashMap::with_capacity(num_bits + num_qubits);
        for (i, register) in serialcirc
            .qubits
            .iter()
            .chain(serialcirc.bits.iter())
            .enumerate()
        {
            if register.1.len() != 1 {
                // TODO: Support multi-index registers?
                panic!("Register {} has more than one index", register.0);
            }
            wire_map.insert((register, 0).into(), i);
        }
        let sig = FunctionType::new_endo(
            [vec![QB_T; num_qubits], vec![LINEAR_BIT.clone(); num_bits]].concat(),
        );
        // .with_extension_delta(&ExtensionSet::singleton(&TKET1_EXTENSION_ID));

        let mut dfg = DFGBuilder::new(sig).unwrap();

        // Metadata. The circuit requires "name", and we store other things that
        // should pass through the serialization roundtrip.
        dfg.set_metadata("name", json!(serialcirc.name));
        dfg.set_metadata(METADATA_PHASE, json!(serialcirc.phase));
        dfg.set_metadata(
            METADATA_IMPLICIT_PERM,
            json!(serialcirc.implicit_permutation),
        );
        dfg.set_metadata(METADATA_Q_REGISTERS, json!(serialcirc.qubits));
        dfg.set_metadata(METADATA_B_REGISTERS, json!(serialcirc.bits));

        let dangling_wires = dfg.input_wires().collect::<Vec<_>>();
        JsonDecoder {
            hugr: dfg,
            dangling_wires,
            register_wire: wire_map,
            num_qubits,
            num_bits,
        }
    }

    /// Finish building the [`Hugr`].
    pub fn finish(self) -> Hugr {
        // TODO: Throw validation error?
        self.hugr
            .finish_hugr_with_outputs(self.dangling_wires, &REGISTRY)
            .unwrap()
    }

    /// Add a [`Command`] from the serial circuit to the [`JsonDecoder`].
    ///
    /// - [`Command`]: circuit_json::Command
    pub fn add_command(&mut self, command: circuit_json::Command) {
        // TODO Store the command's `opgroup` in the metadata.
        let circuit_json::Command { op, args, .. } = command;
        let num_qubits = args
            .iter()
            .take_while(|&arg| self.reg_wire(arg, 0) < self.num_qubits)
            .count();
        let num_bits = args.len() - num_qubits;
        let op = JsonOp::new_from_op(op, num_qubits, num_bits);

        let args: Vec<_> = args.into_iter().map(|reg| self.reg_wire(&reg, 0)).collect();

        let param_wires: Vec<Wire> = op
            .param_inputs()
            .map(|p| self.create_param_wire(p))
            .collect();

        let append_wires = args
            .into_iter()
            .map(CircuitUnit::Linear)
            .chain(param_wires.into_iter().map(CircuitUnit::Wire));

        self.with_circ_builder(|circ| {
            circ.append_and_consume(&op, append_wires).unwrap();
        });
    }

    /// Apply a function to the internal hugr builder viewed as a [`CircuitBuilder`].
    fn with_circ_builder(&mut self, f: impl FnOnce(&mut CircuitBuilder<DFGBuilder<Hugr>>)) {
        let mut circ = self.hugr.as_circuit(mem::take(&mut self.dangling_wires));
        f(&mut circ);
        self.dangling_wires = circ.finish();
    }

    /// Returns the wire carrying a parameter.
    ///
    /// If the parameter is a constant, a constant definition is added to the Hugr.
    ///
    /// TODO: If the parameter is a variable, returns the corresponding wire from the input.
    fn create_param_wire(&mut self, param: &str) -> Wire {
        match try_param_to_constant(param) {
            Some(c) => {
                let const_type = FLOAT64_TYPE;
                let const_op = Const::new(c, const_type).unwrap();
                self.hugr.add_load_const(const_op).unwrap()
            }
            None => {
                // store string in custom op.
                let symb_op = symbolic_constant_op(param);
                let o = self.hugr.add_dataflow_op(symb_op, []).unwrap();
                o.out_wire(0)
            }
        }
    }

    /// Return the wire index for the `elem`th value of a given register.
    ///
    /// Relies on TKET1 constraint that all registers have unique names.
    fn reg_wire(&self, register: &circuit_json::Register, elem: usize) -> usize {
        self.register_wire[&(register, elem).into()]
    }
}

/// A hashed register, used to identify registers in the [`JsonDecoder::register_wire`] map,
/// avoiding string clones on lookup.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RegisterHash {
    hash: u64,
}

impl From<(&circuit_json::Register, usize)> for RegisterHash {
    fn from((reg, elem): (&circuit_json::Register, usize)) -> Self {
        let mut hasher = DefaultHasher::new();
        reg.0.hash(&mut hasher);
        reg.1[elem].hash(&mut hasher);
        Self {
            hash: hasher.finish(),
        }
    }
}

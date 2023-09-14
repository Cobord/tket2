use std::collections::HashMap;
use std::path::Path;

use hugr::builder::{DFGBuilder, Dataflow, DataflowHugr};
use hugr::extension::prelude::QB_T;
use hugr::hugr::CircuitUnit;
use hugr::ops::OpType as Op;
use hugr::std_extensions::arithmetic::float_types::FLOAT64_TYPE;
use hugr::types::{FunctionType, Type};
use hugr::Hugr as Circuit;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::T2Op;

#[derive(Debug, Serialize, Deserialize)]
struct RepCircOp {
    opstr: String,
    outputs: Vec<String>,
    inputs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RepCirc(Vec<RepCircOp>);

#[derive(Debug, Serialize, Deserialize)]
struct MetaData {
    n_qb: usize,
    n_input_param: usize,
    n_total_param: usize,
    num_gates: u64,
    id: Vec<String>,
    fingerprint: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RepCircData {
    meta: MetaData,
    circ: RepCirc,
}

fn map_op(opstr: &str) -> Op {
    // TODO, more
    match opstr {
        "h" => T2Op::H,
        "cx" => T2Op::CX,
        "t" => T2Op::T,
        "s" => T2Op::S,
        "x" => T2Op::X,
        "y" => T2Op::Y,
        "z" => T2Op::Z,
        "tdg" => T2Op::Tdg,
        "sdg" => T2Op::Sdg,
        "rz" => T2Op::RzF64,
        "add" => T2Op::AngleAdd,
        x => panic!("unknown op {x}"),
    }
    .into()
}

// TODO change to TryFrom
impl From<RepCircData> for Circuit {
    fn from(RepCircData { circ: rc, meta }: RepCircData) -> Self {
        let qb_types: Vec<Type> = vec![QB_T; meta.n_qb];
        let param_types: Vec<Type> = vec![FLOAT64_TYPE; meta.n_input_param];
        let mut builder = DFGBuilder::new(FunctionType::new(
            [qb_types.clone(), param_types].concat(),
            qb_types,
        ))
        .unwrap();

        // Current map between quartz qubit and parameter identifiers, and
        // circuit units. Since quartz defines output wires arbitrarily for each
        // command, these may be altered mid-circuit.
        let param_wires = builder.input_wires().skip(meta.n_qb);
        let mut input_units: HashMap<String, CircuitUnit> =
            HashMap::with_capacity(builder.num_inputs());
        input_units.extend((0..meta.n_qb).map(|i| (format!("Q{}", i), CircuitUnit::Linear(i))));
        input_units.extend(
            param_wires
                .enumerate()
                .map(|(i, w)| (format!("P{}", i), CircuitUnit::Wire(w))),
        );

        let circ_inputs = builder.input_wires().take(meta.n_qb).collect_vec();
        let mut circ = builder.as_circuit(circ_inputs);

        for RepCircOp {
            opstr,
            inputs,
            outputs,
        } in rc.0
        {
            let op = map_op(&opstr);

            // Translate the quartz inputs into circuit units.
            let inputs = inputs.iter().map(|inp| *input_units.get(inp).unwrap());
            let hugr_outputs = circ.append_with_outputs(op, inputs).unwrap();

            for (idx, wire) in outputs.iter().zip(hugr_outputs) {
                input_units.insert(idx.to_string(), CircuitUnit::Wire(wire));
            }
        }

        let circ_outputs = circ.finish();
        builder
            .finish_hugr_with_outputs(circ_outputs, &crate::extension::REGISTRY)
            .unwrap()
    }
}

pub(super) fn load_ecc_set(path: impl AsRef<Path>) -> HashMap<String, Vec<Circuit>> {
    let jsons = std::fs::read_to_string(path).unwrap();
    let (_, ecc_map): (Vec<()>, HashMap<String, Vec<RepCircData>>) =
        serde_json::from_str(&jsons).unwrap();

    ecc_map
        .into_values()
        .map(|datmap| {
            let id = datmap[0].meta.id[0].clone();
            let circs = datmap.into_iter().map(|rcd| rcd.into()).collect();
            (id, circs)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // use crate::validate::check_soundness;

    use hugr::HugrView;

    use super::*;
    fn load_representative_set(path: &str) -> HashMap<String, Circuit> {
        let jsons = std::fs::read_to_string(path).unwrap();
        // read_rep_json(&jsons).unwrap();
        let st: Vec<RepCircData> = serde_json::from_str(&jsons).unwrap();
        st.into_iter()
            .map(|mut rcd| (rcd.meta.id.remove(0), rcd.into()))
            .collect()
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Opening files is not supported in (isolated) miri
    fn test_read_rep() {
        let rep_map: HashMap<String, Circuit> =
            load_representative_set("test_files/h_rz_cxrepresentative_set.json");

        for c in rep_map.values().take(1) {
            println!("{}", c.dot_string());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Opening files is not supported in (isolated) miri
    fn test_read_complete() {
        let _ecc: HashMap<String, Vec<Circuit>> =
            load_ecc_set("test_files/h_rz_cxcomplete_ECC_set.json");

        // ecc.values()
        //     .flatten()
        //     .for_each(|c| check_soundness(c).unwrap());
    }
}

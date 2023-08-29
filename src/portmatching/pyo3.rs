//! Python bindings for portmatching features

use std::fmt;

use derive_more::{From, Into};
use hugr::hugr::views::{DescendantsGraph, HierarchyView};
use hugr::ops::handle::DfgID;
use hugr::{Hugr, HugrView};
use pyo3::{create_exception, exceptions::PyException, prelude::*, types::PyIterator};
use tket_json_rs::circuit_json::SerialCircuit;

use super::{CircuitMatch, CircuitMatcher, CircuitPattern, CircuitRewrite};
use crate::json::TKETDecode;

create_exception!(pyrs, PyValidateError, PyException);
create_exception!(pyrs, PyInvalidReplacement, PyException);

#[pymethods]
impl CircuitPattern {
    /// Construct a pattern from a TKET1 circuit
    #[new]
    pub fn py_from_circuit(circ: PyObject) -> PyResult<CircuitPattern> {
        let hugr = pyobj_as_hugr(circ)?;
        let circ = hugr_as_view(&hugr);
        Ok(CircuitPattern::from_circuit(&circ))
    }

    /// A string representation of the pattern.
    pub fn __repr__(&self) -> String {
        format!("{:?}", self)
    }
}

#[pymethods]
impl CircuitMatcher {
    /// Construct a matcher from a list of patterns.
    #[new]
    pub fn py_from_patterns(patterns: &PyIterator) -> PyResult<Self> {
        Ok(CircuitMatcher::from_patterns(
            patterns
                .iter()?
                .map(|p| p?.extract::<CircuitPattern>())
                .collect::<PyResult<Vec<_>>>()?,
        ))
    }
    /// A string representation of the pattern.
    pub fn __repr__(&self) -> PyResult<String> {
        Ok(format!("{:?}", self))
    }

    /// Find all convex matches in a circuit.
    #[pyo3(name = "find_matches")]
    pub fn py_find_matches(&self, circ: PyObject) -> PyResult<Vec<PyCircuitMatch>> {
        let hugr = pyobj_as_hugr(circ)?;
        let circ = hugr_as_view(&hugr);
        Ok(self
            .find_matches(&circ)
            .into_iter()
            .map(|m| PyCircuitMatch::new(m.pattern.clone(), hugr.clone(), Node(m.root)))
            .collect())
    }
}

/// Python equivalent of [`CircuitMatch`].
///
/// A convex pattern match in a circuit, available from Python.
///
/// This object removes the lifetime constraints of its Rust counterpart by
/// cloning the pattern and circuit data. It is provided for convenience and
/// not recommended when performance is a key concern.
///
/// TODO: can this be a wrapper for a [`CircuitMatch`] instead?
#[pyclass]
#[derive(Debug, Clone)]
pub struct PyCircuitMatch {
    /// The pattern that was matched.
    pub pattern: CircuitPattern,
    /// The circuit that contains the match.
    pub circuit: Hugr,
    /// The root of the pattern within the circuit.
    pub root: Node,
}

#[pymethods]
impl PyCircuitMatch {
    /// A string representation of the pattern.
    pub fn __repr__(&self) -> String {
        let circ = hugr_as_view(&self.circuit);
        format!(
            "CircuitMatch {:?}",
            self.pattern
                .get_match_map(self.root.0, &circ)
                .expect("Invalid PyCircuitMatch object")
        )
    }
}

impl PyCircuitMatch {
    pub fn new(pattern: CircuitPattern, circuit: Hugr, root: Node) -> Self {
        Self {
            pattern,
            circuit,
            root,
        }
    }

    /// Obtain as a [`CircuitMatch`] object.
    pub fn to_rewrite(&self, replacement: PyObject) -> PyResult<CircuitRewrite> {
        let circ = hugr_as_view(&self.circuit);
        CircuitMatch::try_from_root_match(self.root.0, &self.pattern, &circ)
            .expect("Invalid PyCircuitMatch object")
            .to_rewrite(pyobj_as_hugr(replacement)?)
            .map_err(|e| PyInvalidReplacement::new_err(e.to_string()))
    }
}

/// A [`hugr::Node`] wrapper for Python.
///
/// Note: this will probably be useful outside of portmatching
#[pyclass]
#[derive(From, Into, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Node(hugr::Node);

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[pymethods]
impl Node {
    /// A string representation of the pattern.
    pub fn __repr__(&self) -> String {
        format!("{:?}", self)
    }
}

fn pyobj_as_hugr(circ: PyObject) -> PyResult<Hugr> {
    let ser_c = SerialCircuit::_from_tket1(circ);
    let hugr: Hugr = ser_c
        .decode()
        .map_err(|e| PyValidateError::new_err(e.to_string()))?;
    Ok(hugr)
}

fn hugr_as_view(hugr: &Hugr) -> DescendantsGraph<'_, DfgID> {
    DescendantsGraph::new(hugr, hugr.root())
}
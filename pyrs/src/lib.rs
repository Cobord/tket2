use pyo3::prelude::*;
use tket2::portmatching::{CircuitPattern, PatternMatcher};

/// The Python bindings to TKET2.
#[pymodule]
fn pyrs(py: Python, m: &PyModule) -> PyResult<()> {
    add_patterns_module(py, m)?;
    Ok(())
}

fn add_patterns_module(py: Python, parent: &PyModule) -> PyResult<()> {
    let m = PyModule::new(py, "patterns")?;
    m.add_class::<CircuitPattern>()?;
    m.add_class::<PatternMatcher>()?;
    parent.add_submodule(m)?;
    Ok(())
}

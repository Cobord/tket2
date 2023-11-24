//! Passes for optimising circuits.

pub mod chunks;

use std::{cmp::min, convert::TryInto, fs, num::NonZeroUsize, path::PathBuf};

use pyo3::{prelude::*, types::IntoPyDict};
use tket2::{op_matches, passes::apply_greedy_commutation, Circuit, Tk2Op};

use crate::utils::{create_py_exception, ConvertPyErr};
use crate::{
    circuit::{try_update_hugr, try_with_hugr},
    optimiser::PyBadgerOptimiser,
};

/// The module definition
///
/// This module is re-exported from the python module with the same name.
pub fn module(py: Python) -> PyResult<&PyModule> {
    let m = PyModule::new(py, "_passes")?;
    m.add_function(wrap_pyfunction!(greedy_depth_reduce, m)?)?;
    m.add_function(wrap_pyfunction!(badger_optimise, m)?)?;
    m.add_class::<self::chunks::PyCircuitChunks>()?;
    m.add_function(wrap_pyfunction!(self::chunks::chunks, m)?)?;
    m.add("PullForwardError", py.get_type::<PyPullForwardError>())?;
    Ok(m)
}

create_py_exception!(tket2::passes::PullForwardError, PyPullForwardError, "");

#[pyfunction]
fn greedy_depth_reduce(circ: &PyAny) -> PyResult<(&PyAny, u32)> {
    let py = circ.py();
    try_with_hugr(circ, |mut h, typ| {
        let n_moves = apply_greedy_commutation(&mut h).convert_pyerrs()?;
        let circ = typ.convert(py, h)?;
        PyResult::Ok((circ, n_moves))
    })
}

/// Rebase a circuit to the Nam gate set (CX, Rz, H) using TKET1.
///
/// Equivalent to running the following code:
/// ```python
/// from pytket.passes.auto_rebase import auto_rebase_pass
/// from pytket import OpType
/// auto_rebase_pass({OpType.CX, OpType.Rz, OpType.H}).apply(circ)"
// ```
fn rebase_nam(circ: &PyAny) -> PyResult<()> {
    let py = circ.py();
    let auto_rebase = py
        .import("pytket.passes.auto_rebase")?
        .getattr("auto_rebase_pass")?;
    let optype = py.import("pytket")?.getattr("OpType")?;
    let locals = [("OpType", &optype)].into_py_dict(py);
    let op_set = py.eval("{OpType.CX, OpType.Rz, OpType.H}", None, Some(locals))?;
    let rebase_pass = auto_rebase.call1((op_set,))?.getattr("apply")?;
    rebase_pass.call1((circ,)).map(|_| ())
}

/// Badger optimisation pass.
///
/// HyperTKET's best attempt at optimising a circuit using circuit rewriting
/// and the given Badger optimiser.
///
/// By default, the input circuit will be rebased to Nam, i.e. CX + Rz + H before
/// optimising. This can be deactivated by setting `rebase` to `false`, in which
/// case the circuit is expected to be in the Nam gate set.
///
/// Will use at most `max_threads` threads (plus a constant) and take at most
/// `timeout` seconds (plus a constant). Default to the number of cpus and
/// 15min respectively.
///
/// Log files will be written to the directory `log_dir` if specified.
#[pyfunction]
fn badger_optimise<'py>(
    circ: &'py PyAny,
    optimiser: &PyBadgerOptimiser,
    max_threads: Option<NonZeroUsize>,
    timeout: Option<u64>,
    log_dir: Option<PathBuf>,
    rebase: Option<bool>,
) -> PyResult<&'py PyAny> {
    // Default parameter values
    let rebase = rebase.unwrap_or(true);
    let max_threads = max_threads.unwrap_or(num_cpus::get().try_into().unwrap());
    let timeout = timeout.unwrap_or(30);
    // Create log directory if necessary
    if let Some(log_dir) = log_dir.as_ref() {
        fs::create_dir_all(log_dir)?;
    }
    // Rebase circuit
    if rebase {
        rebase_nam(circ)?;
    }
    // Logic to choose how to split the circuit
    let badger_splits = |n_threads: NonZeroUsize| match n_threads.get() {
        n if n >= 7 => (
            vec![n, 3, 1],
            vec![timeout / 2, timeout / 10 * 3, timeout / 10 * 2],
        ),
        n if n >= 4 => (
            vec![n, 2, 1],
            vec![timeout / 2, timeout / 10 * 3, timeout / 10 * 2],
        ),
        n if n > 1 => (vec![n, 1], vec![timeout / 2, timeout / 2]),
        1 => (vec![1], vec![timeout]),
        _ => unreachable!(),
    };
    // Optimise
    try_update_hugr(circ, |mut circ, _| {
        let n_cx = circ
            .commands()
            .filter(|c| op_matches(c.optype(), Tk2Op::CX))
            .count();
        let n_threads = min(
            (n_cx / 50).try_into().unwrap_or(1.try_into().unwrap()),
            max_threads,
        );
        let (split_threads, split_timeouts) = badger_splits(n_threads);
        for (i, (n_threads, timeout)) in split_threads.into_iter().zip(split_timeouts).enumerate() {
            let log_file = log_dir.as_ref().map(|log_dir| {
                let mut log_file = log_dir.clone();
                log_file.push(format!("cycle-{i}.log"));
                log_file
            });
            circ = optimiser.optimise(
                circ,
                Some(timeout),
                Some(n_threads.try_into().unwrap()),
                Some(true),
                log_file,
                None,
            );
        }
        PyResult::Ok(circ)
    })
}
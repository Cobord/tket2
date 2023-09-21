//! TASO circuit optimiser.
//!
//! This module implements the TASO circuit optimiser. It relies on a rewriter
//! and a RewriteStrategy instance to repeatedly rewrite a circuit and optimising
//! it according to some cost metric (typically gate count).
//!
//! The optimiser is implemented as a priority queue of circuits to be processed.
//! On top of the queue are the circuits with the lowest cost. They are popped
//! from the queue and replaced by the new circuits obtained from the rewriter
//! and the rewrite strategy. A hash of every circuit computed is stored to
//! detect and ignore duplicates. The priority queue is truncated whenever
//! it gets too large.

mod eq_circ_class;
mod hugr_pchannel;
mod hugr_pqueue;
mod qtz_circuit;
mod worker;

use crossbeam_channel::select;
pub use eq_circ_class::{load_eccs_json_file, EqCircClass};

use std::num::NonZeroUsize;
use std::time::{Duration, Instant};
use std::{fs, io};

use fxhash::FxHashSet;
use hugr::Hugr;

use crate::circuit::CircuitHash;
use crate::json::save_tk1_json_writer;
use crate::rewrite::strategy::RewriteStrategy;
use crate::rewrite::Rewriter;
use hugr_pqueue::{Entry, HugrPQ};

use self::hugr_pchannel::HugrPriorityChannel;

/// Logging configuration for the TASO optimiser.
#[derive(Default)]
pub struct LogConfig<'w> {
    final_circ_json: Option<Box<dyn io::Write + 'w>>,
    circ_candidates_csv: Option<Box<dyn io::Write + 'w>>,
    progress_log: Option<Box<dyn io::Write + 'w>>,
}

impl<'w> LogConfig<'w> {
    /// Create a new logging configuration.
    ///
    /// Three writer objects must be provided:
    /// - best_circ_json: for the final optimised circuit, in TK1 JSON format,
    /// - circ_candidates_csv: for a log of the successive best candidate circuits,
    /// - progress_log: for a log of the progress of the optimisation.
    pub fn new(
        best_circ_json: impl io::Write + 'w,
        circ_candidates_csv: impl io::Write + 'w,
        progress_log: impl io::Write + 'w,
    ) -> Self {
        Self {
            final_circ_json: Some(Box::new(best_circ_json)),
            circ_candidates_csv: Some(Box::new(circ_candidates_csv)),
            progress_log: Some(Box::new(progress_log)),
        }
    }
}

/// The TASO optimiser.
///
/// Adapted from [Quartz][], and originally [TASO][].
///
/// Using a rewriter and a rewrite strategy, the optimiser
/// will repeatedly rewrite the circuit, optimising the circuit according to
/// the cost function provided.
///
/// Optimisation is done by maintaining a priority queue of circuits and
/// always processing the circuit with the lowest cost first. Rewrites are
/// computed for that circuit and all new circuit obtained are added to the queue.
///
/// This optimiser is single-threaded.
///
/// [Quartz]: https://arxiv.org/abs/2204.09033
/// [TASO]: https://dl.acm.org/doi/10.1145/3341301.3359630
#[derive(Clone, Debug)]
pub struct TasoOptimiser<R, S, C> {
    rewriter: R,
    strategy: S,
    cost: C,
}

impl<R, S, C> TasoOptimiser<R, S, C> {
    /// Create a new TASO optimiser.
    pub fn new(rewriter: R, strategy: S, cost: C) -> Self {
        Self {
            rewriter,
            strategy,
            cost,
        }
    }
}

impl<R, S, C> TasoOptimiser<R, S, C>
where
    R: Rewriter + Send + Clone + 'static,
    S: RewriteStrategy + Send + Clone + 'static,
    C: Fn(&Hugr) -> usize + Send + Sync + Clone + 'static,
{
    /// Run the TASO optimiser on a circuit.
    ///
    /// A timeout (in seconds) can be provided.
    pub fn optimise(&self, circ: &Hugr, timeout: Option<u64>, n_threads: NonZeroUsize) -> Hugr {
        self.optimise_with_log(circ, Default::default(), timeout, n_threads)
    }

    /// Run the TASO optimiser on a circuit with logging activated.
    ///
    /// A timeout (in seconds) can be provided.
    pub fn optimise_with_log(
        &self,
        circ: &Hugr,
        log_config: LogConfig,
        timeout: Option<u64>,
        n_threads: NonZeroUsize,
    ) -> Hugr {
        match n_threads.get() {
            1 => self.taso(circ, log_config, timeout),
            _ => self.taso_multithreaded(circ, log_config, timeout, n_threads),
        }
    }

    /// Run the TASO optimiser on a circuit with default logging.
    ///
    /// The following files will be created:
    ///  - `final_circ.json`: the final optimised circuit, in TK1 JSON format,
    ///  - `best_circs.csv`: a log of the successive best candidate circuits,
    ///  - `taso-optimisation.log`: a log of the progress of the optimisation.
    ///
    /// If the creation of any of these files fails, an error is returned.
    ///
    /// A timeout (in seconds) can be provided.
    pub fn optimise_with_default_log(
        &self,
        circ: &Hugr,
        timeout: Option<u64>,
        n_threads: NonZeroUsize,
    ) -> io::Result<Hugr> {
        let final_circ_json = fs::File::create("final_circ.json")?;
        let circ_candidates_csv = fs::File::create("best_circs.csv")?;
        let progress_log = fs::File::create("taso-optimisation.log")?;
        let log_config = LogConfig::new(final_circ_json, circ_candidates_csv, progress_log);
        Ok(self.optimise_with_log(circ, log_config, timeout, n_threads))
    }

    fn taso(&self, circ: &Hugr, mut log_config: LogConfig, timeout: Option<u64>) -> Hugr {
        let start_time = Instant::now();

        let mut log_candidates = log_config.circ_candidates_csv.map(csv::Writer::from_writer);

        let mut best_circ = circ.clone();
        let mut best_circ_cost = (self.cost)(circ);
        log_best(best_circ_cost, log_candidates.as_mut()).unwrap();

        // Hash of seen circuits. Dot not store circuits as this map gets huge
        let mut seen_hashes: FxHashSet<_> = FromIterator::from_iter([(circ.circuit_hash())]);

        // The priority queue of circuits to be processed (this should not get big)
        const PRIORITY_QUEUE_CAPACITY: usize = 10_000;
        let mut pq = HugrPQ::with_capacity(&self.cost, PRIORITY_QUEUE_CAPACITY);
        pq.push(circ.clone());

        let mut circ_cnt = 1;
        while let Some(Entry { circ, cost, .. }) = pq.pop() {
            if cost < best_circ_cost {
                best_circ = circ.clone();
                best_circ_cost = cost;
                log_best(best_circ_cost, log_candidates.as_mut()).unwrap();
            }

            let rewrites = self.rewriter.get_rewrites(&circ);
            for new_circ in self.strategy.apply_rewrites(rewrites, &circ) {
                let new_circ_hash = new_circ.circuit_hash();
                circ_cnt += 1;
                if circ_cnt % 1000 == 0 {
                    log_progress(
                        log_config.progress_log.as_mut(),
                        circ_cnt,
                        Some(&pq),
                        &seen_hashes,
                    )
                    .expect("Failed to write to progress log");
                }
                if seen_hashes.contains(&new_circ_hash) {
                    continue;
                }
                pq.push_with_hash_unchecked(new_circ, new_circ_hash);
                seen_hashes.insert(new_circ_hash);
            }

            if pq.len() >= PRIORITY_QUEUE_CAPACITY {
                // Haircut to keep the queue size manageable
                pq.truncate(PRIORITY_QUEUE_CAPACITY / 2);
            }

            if let Some(timeout) = timeout {
                if start_time.elapsed().as_secs() > timeout {
                    println!("Timeout");
                    break;
                }
            }
        }

        log_processing_end(circ_cnt, false);

        log_final(
            &best_circ,
            log_config.progress_log.as_mut(),
            log_config.final_circ_json.as_mut(),
            &self.cost,
        )
        .expect("Failed to write to progress log and/or final circuit JSON");

        best_circ
    }

    /// Run the TASO optimiser on a circuit, using multiple threads.
    ///
    /// This is the multi-threaded version of [`taso`]. See [`TasoOptimiser`] for
    /// more details.
    fn taso_multithreaded(
        &self,
        circ: &Hugr,
        mut log_config: LogConfig,
        timeout: Option<u64>,
        n_threads: NonZeroUsize,
    ) -> Hugr {
        let n_threads: usize = n_threads.get();
        const PRIORITY_QUEUE_CAPACITY: usize = 10_000;

        let mut log_candidates = log_config.circ_candidates_csv.map(csv::Writer::from_writer);

        // multi-consumer priority channel for queuing circuits to be processed by the workers
        let (tx_work, rx_work) =
            HugrPriorityChannel::init((self.cost).clone(), PRIORITY_QUEUE_CAPACITY * n_threads);
        // channel for sending circuits from threads back to main
        let (tx_result, rx_result) = crossbeam_channel::unbounded();

        let initial_circ_hash = circ.circuit_hash();
        let mut best_circ = circ.clone();
        let mut best_circ_cost = (self.cost)(&best_circ);
        log_best(best_circ_cost, log_candidates.as_mut()).unwrap();

        // Hash of seen circuits. Dot not store circuits as this map gets huge
        let mut seen_hashes: FxHashSet<_> = FromIterator::from_iter([(initial_circ_hash)]);

        // Each worker waits for circuits to scan for rewrites using all the
        // patterns and sends the results back to main.
        let joins: Vec<_> = (0..n_threads)
            .map(|_| {
                worker::spawn_pattern_matching_thread(
                    rx_work.clone(),
                    tx_result.clone(),
                    self.rewriter.clone(),
                    self.strategy.clone(),
                )
            })
            .collect();
        // Drop our copy of the worker channels, so we don't count as a
        // connected worker.
        drop(rx_work);
        drop(tx_result);

        // Queue the initial circuit
        tx_work
            .send(vec![(initial_circ_hash, circ.clone())])
            .unwrap();

        // A counter of circuits seen.
        let mut circ_cnt = 1;

        // A counter of jobs sent to the workers.
        #[allow(unused)]
        let mut jobs_sent = 0usize;
        // A counter of completed jobs received from the workers.
        #[allow(unused)]
        let mut jobs_completed = 0usize;
        // TODO: Report dropped jobs in the queue, so we can check for termination.

        // Deadline for the optimization timeout
        let timeout_event = match timeout {
            None => crossbeam_channel::never(),
            Some(t) => crossbeam_channel::at(Instant::now() + Duration::from_secs(t)),
        };

        // Process worker results until we have seen all the circuits, or we run
        // out of time.
        loop {
            select! {
                recv(rx_result) -> msg => {
                    match msg {
                        Ok(hashed_circs) => {
                            jobs_completed += 1;
                            for (circ_hash, circ) in &hashed_circs {
                                circ_cnt += 1;
                                if circ_cnt % 1000 == 0 {
                                    // TODO: Add a minimum time between logs
                                    log_progress::<_,u64,usize>(log_config.progress_log.as_mut(), circ_cnt, None, &seen_hashes)
                                        .expect("Failed to write to progress log");
                                }
                                if !seen_hashes.insert(*circ_hash) {
                                    continue;
                                }

                                let cost = (self.cost)(circ);

                                // Check if we got a new best circuit
                                if cost < best_circ_cost {
                                    best_circ = circ.clone();
                                    best_circ_cost = cost;
                                    log_best(best_circ_cost, log_candidates.as_mut()).unwrap();
                                }
                                jobs_sent += 1;
                            }
                            // Fill the workqueue with data from pq
                            if tx_work.send(hashed_circs).is_err() {
                                eprintln!("All our workers panicked. Stopping optimisation.");
                                break;
                            };

                            // If there is no more data to process, we are done.
                            //
                            // TODO: Report dropped jobs in the workers, so we can check for termination.
                            //if jobs_sent == jobs_completed {
                            //    break 'main;
                            //};
                        },
                        Err(crossbeam_channel::RecvError) => {
                            eprintln!("All our workers panicked. Stopping optimisation.");
                            break;
                        }
                    }
                }
                recv(timeout_event) -> _ => {
                    println!("Timeout");
                    break;
                }
            }
        }

        log_processing_end(circ_cnt, true);

        // Drop the channel so the threads know to stop.
        drop(tx_work);
        let _ = joins; // joins.into_iter().for_each(|j| j.join().unwrap());

        log_final(
            &best_circ,
            log_config.progress_log.as_mut(),
            log_config.final_circ_json.as_mut(),
            &self.cost,
        )
        .expect("Failed to write to progress log and/or final circuit JSON");

        best_circ
    }
}

#[cfg(feature = "portmatching")]
mod taso_default {
    use crate::circuit::Circuit;
    use crate::rewrite::strategy::ExhaustiveRewriteStrategy;
    use crate::rewrite::ECCRewriter;

    use super::*;

    impl TasoOptimiser<ECCRewriter, ExhaustiveRewriteStrategy, fn(&Hugr) -> usize> {
        /// A sane default optimiser using the given ECC sets.
        pub fn default_with_eccs_json_file(
            eccs_path: impl AsRef<std::path::Path>,
        ) -> io::Result<Self> {
            let rewriter = ECCRewriter::try_from_eccs_json_file(eccs_path)?;
            let strategy = ExhaustiveRewriteStrategy::default();
            Ok(Self::new(rewriter, strategy, |c| c.num_gates()))
        }
    }
}

/// A helper struct for logging improvements in circuit size seen during the
/// TASO execution.
//
// TODO: Replace this fixed logging. Report back intermediate results.
#[derive(serde::Serialize, Clone, Debug)]
struct BestCircSer {
    circ_len: usize,
    time: String,
}

impl BestCircSer {
    fn new(circ_len: usize) -> Self {
        let time = chrono::Local::now().to_rfc3339();
        Self { circ_len, time }
    }
}

fn log_best<W: io::Write>(cbest: usize, wtr: Option<&mut csv::Writer<W>>) -> io::Result<()> {
    let Some(wtr) = wtr else {
        return Ok(());
    };
    println!("new best of size {}", cbest);
    wtr.serialize(BestCircSer::new(cbest)).unwrap();
    wtr.flush()
}

fn log_processing_end(circuit_count: usize, needs_joining: bool) {
    println!("END");
    println!("Tried {circuit_count} circuits");
    if needs_joining {
        println!("Joining");
    }
}

fn log_progress<W: io::Write, P: Ord, C>(
    wr: Option<&mut W>,
    circ_cnt: usize,
    pq: Option<&HugrPQ<P, C>>,
    seen_hashes: &FxHashSet<u64>,
) -> io::Result<()> {
    if let Some(wr) = wr {
        writeln!(wr, "{circ_cnt} circuits...")?;
        if let Some(pq) = pq {
            writeln!(wr, "Queue size: {} circuits", pq.len())?;
        }
        writeln!(wr, "Total seen: {} circuits", seen_hashes.len())?;
    }
    Ok(())
}

fn log_final<W1: io::Write, W2: io::Write>(
    best_circ: &Hugr,
    log: Option<&mut W1>,
    final_circ: Option<&mut W2>,
    cost: impl Fn(&Hugr) -> usize,
) -> io::Result<()> {
    if let Some(log) = log {
        writeln!(log, "END RESULT: {}", cost(best_circ))?;
    }
    if let Some(circ_writer) = final_circ {
        save_tk1_json_writer(best_circ, circ_writer).unwrap();
    }
    Ok(())
}

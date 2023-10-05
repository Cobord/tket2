//! Pattern and matcher objects for circuit matching

use std::{
    fmt::Debug,
    fs::File,
    io,
    path::{Path, PathBuf},
};

use super::{CircuitPattern, NodeID, PEdge, PNode};
use hugr::hugr::views::sibling_subgraph::{
    ConvexChecker, InvalidReplacement, InvalidSubgraph, InvalidSubgraphBoundary,
};
use hugr::hugr::PortIndex;
use hugr::{hugr::views::SiblingSubgraph, ops::OpType, Hugr, Node, Port};
use itertools::Itertools;
use portmatching::{
    automaton::{LineBuilder, ScopeAutomaton},
    EdgeProperty, PatternID,
};
use thiserror::Error;

#[cfg(feature = "pyo3")]
use pyo3::prelude::*;

use crate::{
    circuit::Circuit,
    ops::NotT2Op,
    rewrite::{CircuitRewrite, Subcircuit},
    T2Op,
};

/// Matchable operations in a circuit.
///
/// We currently support [`T2Op`] and a the HUGR load constant operation.
// TODO: Support OpType::Const, but blocked by use of F64 (Eq support required)
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub(crate) enum MatchOp {
    /// A TKET2 operation.
    Op(T2Op),
    /// A HUGR load constant operation.
    LoadConstant,
}

impl From<T2Op> for MatchOp {
    fn from(op: T2Op) -> Self {
        Self::Op(op)
    }
}

impl TryFrom<OpType> for MatchOp {
    type Error = NotT2Op;

    fn try_from(value: OpType) -> Result<Self, Self::Error> {
        match value {
            OpType::LeafOp(op) => Ok(Self::Op(op.try_into()?)),
            OpType::LoadConstant(_) => Ok(Self::LoadConstant),
            _ => Err(NotT2Op),
        }
    }
}

/// A convex pattern match in a circuit.
///
/// The pattern is identified by a [`PatternID`] that can be used to retrieve the
/// pattern from the matcher.
#[derive(Clone)]
pub struct PatternMatch {
    position: Subcircuit,
    pattern: PatternID,
    /// The root of the pattern in the circuit.
    ///
    /// This is redundant with the position attribute, but is a more concise
    /// representation of the match useful for `PyPatternMatch` or serialisation.
    pub(super) root: Node,
}

impl PatternMatch {
    /// The matcher's pattern ID of the match.
    pub fn pattern_id(&self) -> PatternID {
        self.pattern
    }

    /// Create a pattern match from the image of a pattern root.
    ///
    /// This checks at construction time that the match is convex. This will
    /// have runtime linear in the size of the circuit.
    ///
    /// For repeated convexity checking on the same circuit, use
    /// [`PatternMatch::try_from_root_match_with_checker`] instead.
    ///
    /// Returns an error if
    ///  - the match is not convex
    ///  - the subcircuit does not match the pattern
    ///  - the subcircuit is empty
    ///  - the subcircuit obtained is not a valid circuit region
    pub fn try_from_root_match(
        root: Node,
        pattern: PatternID,
        circ: &impl Circuit,
        matcher: &PatternMatcher,
    ) -> Result<Self, InvalidPatternMatch> {
        let mut checker = ConvexChecker::new(circ);
        Self::try_from_root_match_with_checker(root, pattern, circ, matcher, &mut checker)
    }

    /// Create a pattern match from the image of a pattern root with a checker.
    ///
    /// This is the same as [`PatternMatch::try_from_root_match`] but takes a
    /// checker object to speed up convexity checking.
    ///
    /// See [`PatternMatch::try_from_root_match`] for more details.
    pub fn try_from_root_match_with_checker<'c, C: Circuit>(
        root: Node,
        pattern: PatternID,
        circ: &'c C,
        matcher: &PatternMatcher,
        checker: &mut ConvexChecker<'c, C>,
    ) -> Result<Self, InvalidPatternMatch> {
        let pattern_ref = matcher
            .get_pattern(pattern)
            .ok_or(InvalidPatternMatch::MatchNotFound)?;
        let map = pattern_ref
            .get_match_map(root, circ)
            .ok_or(InvalidPatternMatch::MatchNotFound)?;
        let inputs = pattern_ref
            .inputs
            .iter()
            .map(|p| p.iter().map(|(n, p)| (map[n], *p)).collect_vec())
            .collect_vec();
        let outputs = pattern_ref
            .outputs
            .iter()
            .map(|(n, p)| (map[n], *p))
            .collect_vec();
        Self::try_from_io_with_checker(root, pattern, circ, inputs, outputs, checker)
    }

    /// Create a pattern match from the subcircuit boundaries.
    ///
    /// The position of the match is given by a list of incoming boundary
    /// ports and outgoing boundary ports. See [`SiblingSubgraph`] for more
    /// details.
    ///
    /// This checks at construction time that the match is convex. This will
    /// have runtime linear in the size of the circuit.
    ///
    /// For repeated convexity checking on the same circuit, use
    /// [`PatternMatch::try_from_io_with_checker`] instead.
    pub fn try_from_io(
        root: Node,
        pattern: PatternID,
        circ: &impl Circuit,
        inputs: Vec<Vec<(Node, Port)>>,
        outputs: Vec<(Node, Port)>,
    ) -> Result<Self, InvalidPatternMatch> {
        let mut checker = ConvexChecker::new(circ);
        Self::try_from_io_with_checker(root, pattern, circ, inputs, outputs, &mut checker)
    }

    /// Create a pattern match from the subcircuit boundaries.
    ///
    /// The position of the match is given by a list of incoming boundary
    /// ports and outgoing boundary ports. See [`SiblingSubgraph`] for more
    /// details.
    ///
    /// This checks at construction time that the match is convex. This will
    /// have runtime linear in the size of the circuit.
    pub fn try_from_io_with_checker<'c, C: Circuit>(
        root: Node,
        pattern: PatternID,
        circ: &'c C,
        inputs: Vec<Vec<(Node, Port)>>,
        outputs: Vec<(Node, Port)>,
        checker: &mut ConvexChecker<'c, C>,
    ) -> Result<Self, InvalidPatternMatch> {
        let subgraph = SiblingSubgraph::try_new_with_checker(inputs, outputs, circ, checker)?;
        Ok(Self {
            position: subgraph.into(),
            pattern,
            root,
        })
    }

    /// Construct a rewrite to replace `self` with `repl`.
    pub fn to_rewrite(
        &self,
        source: &Hugr,
        target: Hugr,
    ) -> Result<CircuitRewrite, InvalidReplacement> {
        CircuitRewrite::try_new(&self.position, source, target)
    }
}

impl Debug for PatternMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatternMatch")
            .field("root", &self.root)
            .field("nodes", &self.position.subgraph.nodes())
            .finish()
    }
}

/// A matcher object for fast pattern matching on circuits.
///
/// This uses a state automaton internally to match against a set of patterns
/// simultaneously.
#[cfg_attr(feature = "pyo3", pyclass)]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PatternMatcher {
    automaton: ScopeAutomaton<PNode, PEdge, Port>,
    patterns: Vec<CircuitPattern>,
}

impl Debug for PatternMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatternMatcher")
            .field("patterns", &self.patterns)
            .finish()
    }
}

impl PatternMatcher {
    /// Construct a matcher from a set of patterns
    pub fn from_patterns(patterns: impl Into<Vec<CircuitPattern>>) -> Self {
        let patterns = patterns.into();
        let line_patterns = patterns
            .iter()
            .map(|p| {
                p.pattern
                    .clone()
                    .try_into_line_pattern(compatible_offsets)
                    .expect("Failed to express pattern as line pattern")
            })
            .collect_vec();
        let builder = LineBuilder::from_patterns(line_patterns);
        let automaton = builder.build();
        Self {
            automaton,
            patterns,
        }
    }

    /// Find all convex pattern matches in a circuit.
    pub fn find_matches<C: Circuit + Clone>(&self, circuit: &C) -> Vec<PatternMatch> {
        let mut checker = ConvexChecker::new(circuit);
        circuit
            .commands()
            .flat_map(|cmd| self.find_rooted_matches(circuit, cmd.node(), &mut checker))
            .collect()
    }

    /// Find all convex pattern matches in a circuit rooted at a given node.
    fn find_rooted_matches<'c, C: Circuit + Clone>(
        &self,
        circ: &'c C,
        root: Node,
        checker: &mut ConvexChecker<'c, C>,
    ) -> Vec<PatternMatch> {
        self.automaton
            .run(
                root.into(),
                // Node weights (none)
                validate_circuit_node(circ),
                // Check edge exist
                validate_circuit_edge(circ),
            )
            .filter_map(|pattern_id| {
                handle_match_error(
                    PatternMatch::try_from_root_match_with_checker(
                        root, pattern_id, circ, self, checker,
                    ),
                    root,
                )
            })
            .collect()
    }

    /// Get a pattern by ID.
    pub fn get_pattern(&self, id: PatternID) -> Option<&CircuitPattern> {
        self.patterns.get(id.0)
    }

    /// Get the number of patterns in the matcher.
    pub fn n_patterns(&self) -> usize {
        self.patterns.len()
    }

    /// Serialise a matcher into an IO stream.
    ///
    /// Precomputed matchers can be serialised as binary and then loaded
    /// later using [`PatternMatcher::load_binary_io`].
    pub fn save_binary_io<W: io::Write>(
        &self,
        writer: &mut W,
    ) -> Result<(), MatcherSerialisationError> {
        rmp_serde::encode::write(writer, &self)?;
        Ok(())
    }

    /// Loads a matcher from an IO stream.
    ///
    /// Loads streams as created by [`PatternMatcher::save_binary_io`].
    pub fn load_binary_io<R: io::Read>(reader: &mut R) -> Result<Self, MatcherSerialisationError> {
        let matcher: Self = rmp_serde::decode::from_read(reader)?;
        Ok(matcher)
    }

    /// Save a matcher as a binary file.
    ///
    /// Precomputed matchers can be saved as binary files and then loaded
    /// later using [`PatternMatcher::load_binary`].
    ///
    /// The extension of the file name will always be set or amended to be
    /// `.bin`.
    ///
    /// If successful, returns the path to the newly created file.
    pub fn save_binary(
        &self,
        name: impl AsRef<Path>,
    ) -> Result<PathBuf, MatcherSerialisationError> {
        let mut file_name = PathBuf::from(name.as_ref());
        file_name.set_extension("bin");
        let mut file = File::create(&file_name)?;
        self.save_binary_io(&mut file)?;
        Ok(file_name)
    }

    /// Loads a matcher saved using [`PatternMatcher::save_binary`].
    pub fn load_binary(name: impl AsRef<Path>) -> Result<Self, MatcherSerialisationError> {
        let file = File::open(name)?;
        let mut reader = std::io::BufReader::new(file);
        Self::load_binary_io(&mut reader)
    }
}

/// Errors that can occur when constructing matches.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InvalidPatternMatch {
    /// The match is not convex.
    #[error("match is not convex")]
    NotConvex,
    /// The subcircuit does not match the pattern.
    #[error("invalid circuit region")]
    MatchNotFound,
    /// The subcircuit matched is not valid.
    ///
    /// This is typically a logic error in the code.
    #[error("invalid circuit region")]
    InvalidSubcircuit,
    /// Empty matches are not supported.
    ///
    /// This should never happen is the pattern is not itself empty (in which
    /// case an error would have been raised earlier on).
    #[error("empty match")]
    EmptyMatch,
}

/// Errors that can occur when (de)serialising a matcher.
#[derive(Debug, Error)]
pub enum MatcherSerialisationError {
    /// An IO error occured
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    /// An error occured during deserialisation
    #[error("Deserialisation error: {0}")]
    Deserialisation(#[from] rmp_serde::decode::Error),
    /// An error occured during serialisation
    #[error("Serialisation error: {0}")]
    Serialisation(#[from] rmp_serde::encode::Error),
}

impl From<InvalidSubgraph> for InvalidPatternMatch {
    fn from(value: InvalidSubgraph) -> Self {
        match value {
            // A non-convex subgraph might show itself as a disconnected boundary
            // in the subgraph
            InvalidSubgraph::NotConvex
            | InvalidSubgraph::InvalidBoundary(
                InvalidSubgraphBoundary::DisconnectedBoundaryPort(_, _),
            ) => InvalidPatternMatch::NotConvex,
            InvalidSubgraph::EmptySubgraph => InvalidPatternMatch::EmptyMatch,
            InvalidSubgraph::NoSharedParent | InvalidSubgraph::InvalidBoundary(_) => {
                InvalidPatternMatch::InvalidSubcircuit
            }
        }
    }
}

fn compatible_offsets(e1: &PEdge, e2: &PEdge) -> bool {
    let PEdge::InternalEdge { dst: dst1, .. } = e1 else {
        return false;
    };
    let src2 = e2.offset_id();
    dst1.direction() != src2.direction() && dst1.index() == src2.index()
}

/// Returns a predicate checking that an edge at `src` satisfies `prop` in `circ`.
pub(super) fn validate_circuit_edge(
    circ: &impl Circuit,
) -> impl for<'a> Fn(NodeID, &'a PEdge) -> Option<NodeID> + '_ {
    move |src, &prop| {
        let NodeID::HugrNode(src) = src else {
            return None;
        };
        match prop {
            PEdge::InternalEdge {
                src: src_port,
                dst: dst_port,
                ..
            } => {
                let (next_node, next_port) = circ.linked_ports(src, src_port).exactly_one().ok()?;
                (dst_port == next_port).then_some(NodeID::HugrNode(next_node))
            }
            PEdge::InputEdge { src: src_port } => {
                let (next_node, next_port) = circ.linked_ports(src, src_port).exactly_one().ok()?;
                Some(NodeID::CopyNode(next_node, next_port))
            }
        }
    }
}

/// Returns a predicate checking that `node` satisfies `prop` in `circ`.
pub(crate) fn validate_circuit_node(
    circ: &impl Circuit,
) -> impl for<'a> Fn(NodeID, &PNode) -> bool + '_ {
    move |node, prop| {
        let NodeID::HugrNode(node) = node else {
            return false;
        };
        let v_weight = MatchOp::try_from(circ.get_optype(node).clone());
        v_weight.is_ok_and(|w| &w == prop)
    }
}

/// Unwraps match errors, ignoring benign errors and panicking otherwise.
///
/// Benign errors are non-convex matches, which are expected to occur.
/// Other errors are considered logic errors and should never occur.
fn handle_match_error<T>(match_res: Result<T, InvalidPatternMatch>, root: Node) -> Option<T> {
    match_res
        .map_err(|err| match err {
            InvalidPatternMatch::NotConvex => InvalidPatternMatch::NotConvex,
            InvalidPatternMatch::MatchNotFound
            | InvalidPatternMatch::InvalidSubcircuit
            | InvalidPatternMatch::EmptyMatch => {
                panic!("invalid match at root node {root:?}")
            }
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use hugr::Hugr;
    use itertools::Itertools;
    use rstest::{fixture, rstest};

    use crate::utils::build_simple_circuit;
    use crate::T2Op;

    use super::{CircuitPattern, PatternMatcher};

    fn h_cx() -> Hugr {
        build_simple_circuit(2, |circ| {
            circ.append(T2Op::CX, [0, 1]).unwrap();
            circ.append(T2Op::H, [0]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    fn cx_xc() -> Hugr {
        build_simple_circuit(2, |circ| {
            circ.append(T2Op::CX, [0, 1]).unwrap();
            circ.append(T2Op::CX, [1, 0]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    fn cx_cx_3() -> Hugr {
        build_simple_circuit(3, |circ| {
            circ.append(T2Op::CX, [0, 1]).unwrap();
            circ.append(T2Op::CX, [2, 1]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    #[fixture]
    fn cx_cx() -> Hugr {
        build_simple_circuit(2, |circ| {
            circ.append(T2Op::CX, [0, 1]).unwrap();
            circ.append(T2Op::CX, [0, 1]).unwrap();
            Ok(())
        })
        .unwrap()
    }

    #[test]
    fn construct_matcher() {
        let circ = h_cx();

        let p = CircuitPattern::try_from_circuit(&circ).unwrap();
        let m = PatternMatcher::from_patterns(vec![p]);

        let matches = m.find_matches(&circ);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn serialise_round_trip() {
        let circs = [h_cx(), cx_xc()];
        let patterns = circs
            .iter()
            .map(|circ| CircuitPattern::try_from_circuit(circ).unwrap())
            .collect_vec();

        // Estimate the size of the buffer based on the number of patterns and the size of each pattern
        let mut buf = Vec::with_capacity(patterns[0].n_edges() + patterns[1].n_edges());
        let m = PatternMatcher::from_patterns(patterns);
        m.save_binary_io(&mut buf).unwrap();

        let m2 = PatternMatcher::load_binary_io(&mut buf.as_slice()).unwrap();
        let mut buf2 = Vec::with_capacity(buf.len());
        m2.save_binary_io(&mut buf2).unwrap();

        assert_eq!(buf, buf2);
    }

    #[rstest]
    fn cx_cx_replace_to_id(cx_cx: Hugr, cx_cx_3: Hugr) {
        let p = CircuitPattern::try_from_circuit(&cx_cx_3).unwrap();
        let m = PatternMatcher::from_patterns(vec![p]);

        let matches = m.find_matches(&cx_cx);
        assert_eq!(matches.len(), 0);
    }
}

use std::collections::BTreeMap;

use portgraph::graph::{Direction, EdgeIndex, Graph, NodeIndex, DIRECTIONS};
use rayon::prelude::*;
struct MatchFail();

/*
A pattern for the pattern matcher with a fixed graph structure but arbitrary comparison at nodes.
 */
#[derive(Clone)]
pub struct FixedStructPattern<N, E, F> {
    pub graph: Graph<N, E>,
    pub boundary: [NodeIndex; 2],
    pub node_comp_closure: F,
}

impl<N, E, F> FixedStructPattern<N, E, F> {
    pub fn new(graph: Graph<N, E>, boundary: [NodeIndex; 2], node_comp_closure: F) -> Self {
        Self {
            graph,
            boundary,
            node_comp_closure,
        }
    }
}

pub trait NodeCompClosure<N, E>: Fn(&Graph<N, E>, NodeIndex, &N) -> bool {}

impl<N, E, T> NodeCompClosure<N, E> for T where T: Fn(&Graph<N, E>, NodeIndex, &N) -> bool {}

pub fn node_equality<N: PartialEq, E>() -> impl NodeCompClosure<N, E> + Clone {
    |pattern_graph: &Graph<N, E>, pattern_idx: NodeIndex, target_node: &N| {
        let pattern_node = pattern_graph.node_weight(pattern_idx).unwrap();
        pattern_node == target_node
    }
}
pub type Match = BTreeMap<NodeIndex, NodeIndex>;

#[derive(Clone)]
pub struct PatternMatcher<'g, N, E, F> {
    pattern: FixedStructPattern<N, E, F>,
    target: &'g Graph<N, E>,
}

impl<'g, N, E, F> PatternMatcher<'g, N, E, F> {
    pub fn new(pattern: FixedStructPattern<N, E, F>, target: &'g Graph<N, E>) -> Self {
        Self { pattern, target }
    }

    pub fn set_target(&mut self, target: &'g Graph<N, E>) {
        self.target = target
    }
}

impl<'f: 'g, 'g, N: PartialEq, E: PartialEq, F: NodeCompClosure<N, E> + 'f>
    PatternMatcher<'g, N, E, F>
{
    fn node_match(&self, pattern_node: NodeIndex, target_node: NodeIndex) -> Result<(), MatchFail> {
        match self.target.node_weight(target_node) {
            Some(y) if (self.pattern.node_comp_closure)(&self.pattern.graph, pattern_node, y) => {
                Ok(())
            }
            _ => Err(MatchFail()),
        }
    }

    fn edge_match(&self, pattern_edge: EdgeIndex, target_edge: EdgeIndex) -> Result<(), MatchFail> {
        let err = Err(MatchFail());
        if self.target.edge_weight(target_edge) != self.pattern.graph.edge_weight(pattern_edge) {
            return err;
        }
        match DIRECTIONS.map(|direction| {
            (
                self.target.edge_endpoint(target_edge, direction),
                self.pattern.graph.edge_endpoint(pattern_edge, direction),
            )
        }) {
            [(None, None), (None, None)] => (),
            [(Some(_ts), Some(_tt)), (Some(_ps), Some(_pt))] => (),
            // {
            // let [i, o] = self.pattern.bounda ry;
            // // if (ps.node != i && ps.port != ts.port) || (pt.node != o && pt.port != tt.port) {
            // // return err;
            // // }
            // if (ps != i) || (pt != o) {
            //     return err;
            // }
            // }
            _ => return err,
        }
        Ok(())
    }

    fn all_node_edges(g: &Graph<N, E>, n: NodeIndex) -> impl Iterator<Item = EdgeIndex> + '_ {
        g.node_edges(n, Direction::Incoming)
            .chain(g.node_edges(n, Direction::Outgoing))
    }

    // fn match_from_recurse(
    //     &self,
    //     pattern_node: NodeIndex,
    //     target_node: NodeIndex,
    //     start_edge: EdgeIndex,
    //     match_map: &mut Match<Ix>,
    // ) -> Result<(), MatchFail> {
    //     let err = Err(MatchFail());
    //     self.node_match(pattern_node, target_node)?;
    //     match_map.insert(pattern_node, target_node);

    //     let p_edges = Self::cycle_node_edges(&self.pattern.graph, pattern_node);
    //     let t_edges = Self::cycle_node_edges(self.target, target_node);

    //     if p_edges.len() != t_edges.len() {
    //         return err;
    //     }
    //     let mut eiter = p_edges
    //         .iter()
    //         .zip(t_edges.iter())
    //         .cycle()
    //         .skip_while(|(p, _): &(&EdgeIndex, _)| **p != start_edge);

    //     // TODO verify that it is valid to skip edge_start (it's not at the start)
    //     // WARNING THIS IS PROPERLY HANDLED IN THE match_from, either fix or
    //     // remove this recursive version
    //     eiter.next();
    //     // circle the edges of both nodes starting at the start edge
    //     for (e_p, e_t) in eiter.take(p_edges.len() - 1) {
    //         self.edge_match(*e_p, *e_t)?;

    //         let [e_p_source, e_p_target] =
    //             self.pattern.graph.edge_endpoints(*e_p).ok_or(MatchFail())?;
    //         if e_p_source.node == self.pattern.boundary[0]
    //             || e_p_target.node == self.pattern.boundary[1]
    //         {
    //             continue;
    //         }

    //         let (next_pattern_node, next_target_node) = if e_p_source.node == pattern_node {
    //             (
    //                 e_p_target.node,
    //                 self.target.edge_endpoints(*e_t).ok_or(MatchFail())?[1].node,
    //             )
    //         } else {
    //             (
    //                 e_p_source.node,
    //                 self.target.edge_endpoints(*e_t).ok_or(MatchFail())?[0].node,
    //             )
    //         };

    //         if let Some(matched_node) = match_map.get(&next_pattern_node) {
    //             if *matched_node == next_target_node {
    //                 continue;
    //             } else {
    //                 return err;
    //             }
    //         }
    //         self.match_from_recurse(next_pattern_node, next_target_node, *e_p, match_map)?;
    //     }

    //     Ok(())
    // }

    fn match_from(
        &self,
        pattern_start_node: NodeIndex,
        target_start_node: NodeIndex,
    ) -> Result<Match, MatchFail> {
        let err = Err(MatchFail());
        let mut match_map = Match::new();
        let start_edge = self
            .pattern
            .graph
            .node_edges(pattern_start_node, Direction::Incoming)
            .next()
            .ok_or(MatchFail())?;
        let mut visit_stack: Vec<_> = vec![(pattern_start_node, target_start_node, start_edge)];

        while !visit_stack.is_empty() {
            let (curr_p, curr_t, curr_e) = visit_stack.pop().unwrap();

            self.node_match(curr_p, curr_t)?;
            match_map.insert(curr_p, curr_t);

            let mut p_edges = Self::all_node_edges(&self.pattern.graph, curr_p);
            let mut t_edges = Self::all_node_edges(self.target, curr_t);

            // iterate over edges of both nodes
            loop {
                let (e_p, e_t) = match (p_edges.next(), t_edges.next()) {
                    (None, None) => break,
                    // mismatched boundary sizes
                    (None, Some(_)) | (Some(_), None) => return err,
                    (Some(e_p), Some(e_t)) => (e_p, e_t),
                };
                // optimisation, apart from in the case of the entry to the
                // pattern, the first edge in the iterator is the incoming edge
                // and the destination node has been checked
                if e_p == curr_e && curr_p != pattern_start_node {
                    continue;
                }
                self.edge_match(e_p, e_t)?;

                let [e_p_source, e_p_target] = DIRECTIONS
                    .map(|direction| self.pattern.graph.edge_endpoint(e_p, direction.reverse()));
                let e_p_source = e_p_source.ok_or(MatchFail())?;
                let e_p_target = e_p_target.ok_or(MatchFail())?;
                if e_p_source == self.pattern.boundary[0] || e_p_target == self.pattern.boundary[1]
                {
                    continue;
                }

                let (next_pattern_node, next_target_node) = if e_p_source == curr_p {
                    (
                        e_p_target,
                        self.target
                            .edge_endpoint(e_t, Direction::Incoming)
                            .ok_or(MatchFail())?,
                    )
                } else {
                    (
                        e_p_source,
                        self.target
                            .edge_endpoint(e_t, Direction::Outgoing)
                            .ok_or(MatchFail())?,
                    )
                };

                if let Some(matched_node) = match_map.get(&next_pattern_node) {
                    if *matched_node == next_target_node {
                        continue;
                    } else {
                        return err;
                    }
                }
                visit_stack.push((next_pattern_node, next_target_node, e_p));
            }
        }

        Ok(match_map)
    }

    fn start_pattern_node_edge(&self) -> NodeIndex {
        // as a heuristic starts in the highest degree node of the pattern
        // alternatives could be: rarest label, ...?

        self.pattern
            .graph
            .node_indices()
            .max_by_key(|n| {
                DIRECTIONS
                    .map(|d| self.pattern.graph.node_edges(*n, d).count())
                    .iter()
                    .sum::<usize>()
            })
            .unwrap()
    }

    // pub fn find_matches_recurse(&'g self) -> impl Iterator<Item = Match<Ix>> + 'g {
    //     let (start, start_edge) = self.start_pattern_node_edge();
    //     self.target.nodes().filter_map(move |candidate| {
    //         if self.node_match(start, candidate).is_err() {
    //             return None;
    //         }
    //         let mut bijection = Match::new();
    //         self.match_from_recurse(start, candidate, start_edge, &mut bijection)
    //             .ok()
    //             .map(|()| bijection)
    //     })
    // }

    pub fn find_matches(&'g self) -> impl Iterator<Item = Match> + 'g {
        let start = self.start_pattern_node_edge();
        self.target.node_indices().filter_map(move |candidate| {
            if self.node_match(start, candidate).is_err() {
                None
            } else {
                self.match_from(start, candidate).ok()
            }
        })
    }

    pub fn into_matches(self) -> impl Iterator<Item = Match> + 'g {
        let start = self.start_pattern_node_edge();
        self.target.node_indices().filter_map(move |candidate| {
            if self.node_match(start, candidate).is_err() {
                None
            } else {
                self.match_from(start, candidate).ok()
            }
        })
    }
}

impl<'g, N, E, F> PatternMatcher<'g, N, E, F>
where
    N: PartialEq + Send + Sync,
    E: PartialEq + Send + Sync,
    F: NodeCompClosure<N, E> + Sync + Send,
{
    pub fn find_par_matches(&'g self) -> impl ParallelIterator<Item = Match> + 'g {
        let start = self.start_pattern_node_edge();
        let candidates: Vec<_> = self
            .target
            .node_indices()
            .filter(|n| self.node_match(start, *n).is_ok())
            .collect();
        candidates
            .into_par_iter()
            .filter_map(move |candidate| self.match_from(start, candidate).ok())
    }
}

#[cfg(test)]
mod tests {
    use rayon::iter::ParallelIterator;
    use rstest::{fixture, rstest};

    use super::{node_equality, FixedStructPattern, Match, PatternMatcher};
    use crate::circuit::circuit::{Circuit, UnitID};
    use crate::circuit::dag::{Dag, VertexProperties};
    use crate::circuit::operation::{Op, WireType};
    use portgraph::graph::NodeIndex;

    #[fixture]
    fn simple_circ() -> Circuit {
        let mut circ1 = Circuit::new();
        // let [i, o] = circ1.boundary();
        for _ in 0..2 {
            let i = circ1.new_input(WireType::Qubit);
            let o = circ1.new_output(WireType::Qubit);
            let _noop = circ1.add_vertex_with_edges(Op::Noop(WireType::Qubit), vec![i], vec![o]);
            // circ1.tup_add_edge((i, p), (noop, 0), WireType::Qubit);
            // circ1.tup_add_edge((noop, 0), (o, p), WireType::Qubit);
        }
        circ1
    }
    #[fixture]
    fn simple_isomorphic_circ() -> Circuit {
        let mut circ1 = Circuit::new();
        // let [i, o] = circ1.boundary();
        let o0 = circ1.new_output(WireType::Qubit);
        let i0 = circ1.new_input(WireType::Qubit);

        let o1 = circ1.new_output(WireType::Qubit);
        let i1 = circ1.new_input(WireType::Qubit);

        circ1.add_vertex_with_edges(Op::Noop(WireType::Qubit), vec![i1], vec![o1]);
        circ1.add_vertex_with_edges(Op::Noop(WireType::Qubit), vec![i0], vec![o0]);
        // for p in (0..2).rev() {

        //     // let noop = circ1.add_vertex(Op::Noop(WireType::Qubit));
        //     // circ1.tup_add_edge((noop, 0), (o, p), WireType::Qubit);
        //     // circ1.tup_add_edge((i, p), (noop, 0), WireType::Qubit);
        // }
        circ1
    }

    #[fixture]
    fn noop_pattern_circ() -> Circuit {
        let mut circ1 = Circuit::new();
        let i = circ1.new_input(WireType::Qubit);
        let o = circ1.new_output(WireType::Qubit);
        let _noop = circ1.add_vertex_with_edges(Op::Noop(WireType::Qubit), vec![i], vec![o]);

        // let [i, o] = circ1.boundary();
        // let noop = circ1.add_vertex(Op::Noop(WireType::Qubit));
        // circ1.tup_add_edge((i, 0), (noop, 0), WireType::Qubit);
        // circ1.tup_add_edge((noop, 0), (o, 0), WireType::Qubit);
        circ1
    }

    #[rstest]
    fn test_node_match(simple_circ: Circuit, simple_isomorphic_circ: Circuit) {
        let [i, o] = simple_circ.boundary();
        let pattern_boundary = simple_isomorphic_circ.boundary();
        let dag1 = simple_circ.dag;
        let dag2 = simple_isomorphic_circ.dag;
        let pattern = FixedStructPattern::new(dag2, pattern_boundary, node_equality());
        let matcher = PatternMatcher::new(pattern, &dag1);
        for (n1, n2) in dag1
            .node_indices()
            .zip(matcher.pattern.graph.node_indices())
        {
            assert!(matcher.node_match(n1, n2).is_ok());
        }

        assert!(matcher.node_match(i, o).is_err());
    }

    #[rstest]
    fn test_edge_match(simple_circ: Circuit) {
        let fedges: Vec<_> = simple_circ.dag.edge_indices().collect();
        let pattern_boundary = simple_circ.boundary();

        let mut dag1 = simple_circ.dag.clone();
        let dag2 = simple_circ.dag;

        let pattern = FixedStructPattern::new(dag2, pattern_boundary, node_equality());

        let matcher = PatternMatcher::new(pattern.clone(), &dag1);
        for (e1, e2) in dag1
            .edge_indices()
            .zip(matcher.pattern.graph.edge_indices())
        {
            assert!(matcher.edge_match(e1, e2).is_ok());
        }

        dag1.remove_node(pattern_boundary[0]);
        let matcher = PatternMatcher::new(pattern, &dag1);

        assert!(matcher
            .edge_match(fedges[0], dag1.edge_indices().next().unwrap())
            .is_err());
    }

    fn match_maker(it: impl IntoIterator<Item = (usize, usize)>) -> Match {
        Match::from_iter(
            it.into_iter()
                .map(|(i, j)| (NodeIndex::new(i), NodeIndex::new(j))),
        )
    }

    #[rstest]
    fn test_pattern(mut simple_circ: Circuit, noop_pattern_circ: Circuit) {
        let i = simple_circ.new_input(WireType::Qubit);
        let o = simple_circ.new_output(WireType::Qubit);
        let _xop = simple_circ.add_vertex_with_edges(Op::H, vec![i], vec![o]);
        // let [i, o] = simple_circ.boundary();
        // simple_circ.tup_add_edge((i, 3), (xop, 0), WireType::Qubit);
        // simple_circ.tup_add_edge((xop, 0), (o, 3), WireType::Qubit);

        let pattern_boundary = noop_pattern_circ.boundary();
        let pattern =
            FixedStructPattern::new(noop_pattern_circ.dag, pattern_boundary, node_equality());

        let matcher = PatternMatcher::new(pattern, &simple_circ.dag);

        let matches: Vec<_> = matcher.find_matches().collect();

        // match noop to two noops in target
        assert_eq!(matches[0], match_maker([(2, 2)]));
        assert_eq!(matches[1], match_maker([(2, 3)]));
    }

    #[fixture]
    fn cx_h_pattern() -> Circuit {
        // a CNOT surrounded by hadamards
        let qubits = vec![
            UnitID::Qubit {
                reg_name: "q".into(),
                index: vec![0],
            },
            UnitID::Qubit {
                reg_name: "q".into(),
                index: vec![1],
            },
        ];
        let mut pattern_circ = Circuit::with_uids(qubits);
        pattern_circ.append_op(Op::H, &[0]).unwrap();
        pattern_circ.append_op(Op::H, &[1]).unwrap();
        pattern_circ.append_op(Op::CX, &[0, 1]).unwrap();
        pattern_circ.append_op(Op::H, &[0]).unwrap();
        pattern_circ.append_op(Op::H, &[1]).unwrap();

        pattern_circ
    }
    #[rstest]
    fn test_cx_sequence(cx_h_pattern: Circuit) {
        let qubits = vec![
            UnitID::Qubit {
                reg_name: "q".into(),
                index: vec![0],
            },
            UnitID::Qubit {
                reg_name: "q".into(),
                index: vec![1],
            },
        ];
        let mut target_circ = Circuit::with_uids(qubits);
        target_circ.append_op(Op::H, &[0]).unwrap();
        target_circ.append_op(Op::H, &[1]).unwrap();
        target_circ.append_op(Op::CX, &[0, 1]).unwrap();
        target_circ.append_op(Op::H, &[0]).unwrap();
        target_circ.append_op(Op::H, &[1]).unwrap();
        target_circ.append_op(Op::CX, &[0, 1]).unwrap();
        target_circ.append_op(Op::H, &[0]).unwrap();
        target_circ.append_op(Op::H, &[1]).unwrap();
        target_circ.append_op(Op::CX, &[1, 0]).unwrap();
        target_circ.append_op(Op::H, &[0]).unwrap();
        target_circ.append_op(Op::H, &[1]).unwrap();

        let pattern_boundary = cx_h_pattern.boundary();

        let pattern = FixedStructPattern::new(
            cx_h_pattern.dag,
            pattern_boundary,
            |_: &Dag, pattern_idx: NodeIndex, op2: &VertexProperties| {
                matches!(
                    (pattern_idx.index(), &op2.op,),
                    (2 | 3 | 5 | 6, Op::H) | (4, Op::CX)
                )
            },
        );
        let matcher = PatternMatcher::new(pattern, &target_circ.dag);

        let matches: Vec<_> = matcher.find_matches().collect();

        assert_eq!(matches.len(), 3);
        assert_eq!(
            matches[0],
            match_maker([(2, 2), (3, 3), (4, 4), (5, 5), (6, 6)])
        );
        assert_eq!(
            matches[1],
            match_maker([(2, 5), (3, 6), (4, 7), (5, 8), (6, 9)])
        );
        // check flipped match happens
        assert_eq!(
            matches[2],
            match_maker([(2, 9), (3, 8), (4, 10), (5, 12), (6, 11)])
        );
    }

    #[rstest]
    fn test_cx_ladder(cx_h_pattern: Circuit) {
        let qubits = vec![
            UnitID::Qubit {
                reg_name: "q".into(),
                index: vec![0],
            },
            UnitID::Qubit {
                reg_name: "q".into(),
                index: vec![1],
            },
            UnitID::Qubit {
                reg_name: "q".into(),
                index: vec![3],
            },
        ];

        // use Noop and H, allow matches between either
        let mut target_circ = Circuit::with_uids(qubits);
        let h_0_0 = target_circ
            .append_op(Op::Noop(WireType::Qubit), &[0])
            .unwrap();
        let h_1_0 = target_circ.append_op(Op::H, &[1]).unwrap();
        let cx_0 = target_circ.append_op(Op::CX, &[0, 1]).unwrap();
        let h_0_1 = target_circ.append_op(Op::H, &[0]).unwrap();
        let h_1_1 = target_circ
            .append_op(Op::Noop(WireType::Qubit), &[1])
            .unwrap();
        let h_2_0 = target_circ.append_op(Op::H, &[2]).unwrap();
        let cx_1 = target_circ.append_op(Op::CX, &[2, 1]).unwrap();
        let h_1_2 = target_circ.append_op(Op::H, &[1]).unwrap();
        let h_2_1 = target_circ.append_op(Op::H, &[2]).unwrap();
        let cx_2 = target_circ.append_op(Op::CX, &[0, 1]).unwrap();
        let h_0_2 = target_circ.append_op(Op::H, &[0]).unwrap();
        let h_1_3 = target_circ
            .append_op(Op::Noop(WireType::Qubit), &[1])
            .unwrap();

        // use portgraph::dot::dot_string;
        // println!("{}", dot_string(&target_circ.dag));

        let pattern_boundary = cx_h_pattern.boundary();
        let asym_match = |dag: &Dag, op1, op2: &crate::circuit::dag::VertexProperties| {
            let op1 = dag.node_weight(op1).unwrap();
            match (&op1.op, &op2.op) {
                (x, y) if x == y => true,
                (Op::H, Op::Noop(WireType::Qubit)) | (Op::Noop(WireType::Qubit), Op::H) => true,
                _ => false,
            }
        };

        let pattern = FixedStructPattern::new(cx_h_pattern.dag, pattern_boundary, asym_match);
        let matcher = PatternMatcher::new(pattern, &target_circ.dag);
        let matches_seq: Vec<_> = matcher.find_par_matches().collect();
        let matches: Vec<_> = matcher.find_matches().collect();
        assert_eq!(matches_seq, matches);
        assert_eq!(matches.len(), 3);
        assert_eq!(
            matches[0],
            match_maker([
                (2, h_0_0.index()),
                (3, h_1_0.index()),
                (4, cx_0.index()),
                (5, h_0_1.index()),
                (6, h_1_1.index())
            ])
        );
        // flipped match
        assert_eq!(
            matches[2],
            match_maker([
                (2, h_0_1.index()),
                (3, h_1_2.index()),
                (4, cx_2.index()),
                (5, h_0_2.index()),
                (6, h_1_3.index())
            ])
        );
        assert_eq!(
            matches[1],
            match_maker([
                (2, h_2_0.index()),
                (3, h_1_1.index()),
                (4, cx_1.index()),
                (5, h_2_1.index()),
                (6, h_1_2.index())
            ])
        );
    }
}

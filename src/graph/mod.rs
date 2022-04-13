pub mod dot;
pub mod graph;
pub mod substitute;
pub mod toposort;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::graph::dot::dot_string;

    use super::graph::Graph;

    #[test]
    fn test_insert_graph() {
        let mut g = Graph::<i8, i8, u8>::with_capacity(3, 2);

        let n0 = g.add_node(0);
        let n1 = g.add_node(1);
        let n2 = g.add_node(2);

        let _e1 = g.add_edge((n0, 0), (n1, 0), -1);
        let _e2 = g.add_edge((n1, 0), (n2, 0), -2);

        let mut g2 = Graph::<i8, i8, u8>::with_capacity(2, 1);

        let g20 = g2.add_node(3);
        let g21 = g2.add_node(4);

        let _g2e1 = g2.add_edge((g20, 0), (g21, 0), -3);

        g.insert_graph(g2);

        let correct_weights: HashSet<_> = HashSet::from_iter([0, 1, 2, 3, 4].into_iter());
        assert_eq!(
            HashSet::from_iter(g.nodes().map(|n| *g.node_weight(n).unwrap())),
            correct_weights
        );

        let correct_weights: HashSet<_> = HashSet::from_iter([-1, -2, -3].into_iter());
        assert_eq!(
            HashSet::from_iter(g.edges().map(|e| *g.edge_weight(e).unwrap())),
            correct_weights
        );
    }

    #[test]
    fn test_remove_invalid() {
        let mut g = Graph::<u8, u8, u8>::with_capacity(3, 2);

        let n0 = g.add_node(0);
        let n1 = g.add_node(1);
        let n2 = g.add_node(2);

        let _e1 = g.add_edge((n0, 0), (n1, 0), 3);
        let _e2 = g.add_edge((n1, 0), (n2, 0), 4);
        let _e3 = g.add_edge((n0, 1), (n2, 1), 5);

        assert_eq!(g.node_count(), 3);
        assert_eq!(g.edge_count(), 3);

        assert_eq!(g.remove_node(n1), Some(1));

        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);

        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.edges.len(), 3);

        let g = g.remove_invalid();

        assert_eq!(g.node_count(), 2);
        assert_eq!(g.edge_count(), 1);
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);

        // TODO some better test of validity (check graph correctness conditions)
        let _s = dot_string(&g);
    }
}

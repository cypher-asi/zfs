use super::harness::TestNetwork;

#[test]
fn happy_path_5_nodes() {
    let mut net = TestNetwork::new(5, 0);
    net.run_rounds(20);
    net.assert_converged();
    assert_eq!(net.nodes[0].engine.height(), 20);
}

#[test]
fn one_node_partitioned_then_heals() {
    let mut net = TestNetwork::new(5, 0);
    net.partition(&[4]);
    net.run_rounds(30);

    let majority_height = net.nodes[0].engine.height();
    assert!(
        majority_height >= 5,
        "majority should make progress, got {majority_height}"
    );
    assert_eq!(
        net.nodes[4].engine.height(),
        0,
        "partitioned node should not advance"
    );

    net.heal();
    net.nodes[4].engine.enable_fork_recovery();
    net.run_rounds(30);

    net.assert_converged();
    assert!(
        net.nodes[0].engine.height() > majority_height,
        "should keep advancing after heal"
    );
}

#[test]
fn genesis_fork_recovery() {
    let mut net = TestNetwork::new(5, 0);

    net.partition(&[4]);
    net.run_rounds(1);
    assert_eq!(net.nodes[4].engine.height(), 0);

    net.heal();
    net.run_rounds(5);

    let heights = net.heights();
    assert!(
        heights.iter().all(|&h| h >= 1),
        "all nodes should advance after heal: {heights:?}"
    );
}

#[test]
fn minority_catches_up_via_pending_certs() {
    let mut net = TestNetwork::new(5, 0);

    net.partition(&[3]);
    net.run_rounds(20);

    let majority_height = net.nodes[0].engine.height();
    assert!(
        majority_height >= 5,
        "majority should make progress, got {majority_height}"
    );
    assert_eq!(net.nodes[3].engine.height(), 0);

    net.heal();
    net.nodes[3].engine.enable_fork_recovery();
    net.run_rounds(20);

    let h3 = net.nodes[3].engine.height();
    assert!(
        h3 >= majority_height,
        "node 3 should have caught up to at least {majority_height}, got {h3}"
    );
}

#[test]
fn all_nodes_diverge_then_recover() {
    let mut net = TestNetwork::new(5, 0);

    net.partition(&[0, 1, 2, 3, 4]);
    for node in &mut net.nodes {
        for _ in 0..3 {
            node.engine.tick();
        }
    }
    let heights = net.heights();
    assert!(
        heights.iter().all(|&h| h == 0),
        "no progress while fully partitioned: {heights:?}"
    );

    net.heal();
    for node in &mut net.nodes {
        node.engine.enable_fork_recovery();
    }
    net.run_rounds(20);

    net.assert_converged();
    assert!(
        net.nodes[0].engine.height() >= 5,
        "should have made substantial progress"
    );
}

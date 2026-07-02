//! Multi-node (N>=2) post-quantum cluster tests.
//!
//! These stand up a real LocalCluster of separate in-process validators (each with its own
//! TVU/turbine, gossip, and TPU) and prove the ML-DSA-44 signing surfaces propagate across
//! *distinct* nodes — something the single-node demos cannot exercise (a node's own shreds
//! bypass turbine verify, and single-node "consensus" never observes a peer's votes).
//!
//! All three tests put *every* validator in genesis with equal stake, which is the
//! configuration that most stresses small-N gossip/consensus (root-bank stakes are momentarily
//! empty at genesis). `spend_and_verify_all_nodes` requires each transfer to reach
//! `VOTE_THRESHOLD_DEPTH + 1` confirmations on *all* nodes, so a passing test means the cluster
//! is producing rooted blocks under supermajority voting — not merely that one node is alive.

use {
    serial_test::serial,
    solana_core::validator::ValidatorConfig,
    solana_local_cluster::{
        cluster_tests,
        integration_tests::{DEFAULT_CLUSTER_LAMPORTS, DEFAULT_NODE_STAKE, RUST_LOG_FILTER},
        local_cluster::{ClusterConfig, LocalCluster},
        validator_configs::make_identical_validator_configs,
    },
    solana_sdk::{
        ml_dsa_keypair::MlDsaKeypair, pubkey::Pubkey, signature::Keypair, system_program,
    },
    solana_streamer::socket::SocketAddrSpace,
    std::{collections::HashSet, sync::Arc},
};

const NUM_NODES: usize = 2;

/// Build an all-in-genesis `ClusterConfig` with `NUM_NODES` equally-staked validators, applying
/// `customize` to the per-node validator configs (used to attach ML-DSA voter / shred keys).
fn all_genesis_config(customize: impl Fn(&mut [ValidatorConfig])) -> ClusterConfig {
    let mut validator_configs =
        make_identical_validator_configs(&ValidatorConfig::default_for_test(), NUM_NODES);
    customize(&mut validator_configs);

    // Every node in genesis (the `true` flag) so all vote accounts exist at genesis and can be
    // repointed to an ML-DSA authorized voter, and every node carries stake from slot 0.
    let validator_keys: Vec<(Arc<Keypair>, bool)> = (0..NUM_NODES)
        .map(|_| (Arc::new(Keypair::new()), true))
        .collect();

    ClusterConfig {
        node_stakes: vec![DEFAULT_NODE_STAKE; NUM_NODES],
        cluster_lamports: DEFAULT_CLUSTER_LAMPORTS,
        validator_configs,
        validator_keys: Some(validator_keys),
        ..ClusterConfig::default()
    }
}

fn spend_and_verify(cluster: &LocalCluster) {
    cluster_tests::spend_and_verify_all_nodes(
        &cluster.entry_point_info,
        &cluster.funding_keypair,
        NUM_NODES,
        HashSet::new(),
        SocketAddrSpace::Unspecified,
        &cluster.connection_cache,
    );
}

/// Baseline: an *all-in-genesis* Ed25519 N=2 cluster produces rooted blocks. This is the
/// configuration a small-N cluster is most likely to stall in (empty root-bank stakes at
/// genesis); it must pass before layering ML-DSA on top, otherwise a stall is a
/// genesis/consensus artifact, not an ML-DSA regression.
#[test]
#[serial]
fn test_ed25519_all_genesis_roots() {
    solana_logger::setup_with_default(RUST_LOG_FILTER);
    let mut config = all_genesis_config(|_configs| {});
    let cluster = LocalCluster::new(&mut config, SocketAddrSpace::Unspecified);
    spend_and_verify(&cluster);
}

/// Phase 2a across a real cluster: every validator signs its consensus votes with ML-DSA-44 and
/// its vote account's authorized voter is the ML-DSA address (repointed at genesis by
/// `LocalCluster::new`). Ed25519 voting is therefore *impossible* — the Ed25519 vote keypair is
/// no longer an authorized voter — so a cluster that still reaches supermajority confirmations
/// would, by construction, be finalizing on ML-DSA votes.
///
/// KNOWN BLOCKER (multi-node): both nodes DO sign ML-DSA votes and the leader includes them in
/// blocks, but a *peer* cannot finalize. An ML-DSA (0x00) vote rides the regular TPU over UDP and
/// is bridged in banking to a `VersionedTransaction` whose only signature is a 64-byte *synthetic*
/// id (`MlDsaTransaction::synthetic_signature`); the real 1312-byte pubkey + 2420-byte ML-DSA
/// signature are dropped once TPU sigverify passes. When a peer replays that block,
/// `blockstore_processor` calls `bank.verify_transaction(.., FullVerification)` →
/// `VersionedTransaction::verify_and_hash_message`, which Ed25519-verifies the synthetic id
/// against the message and fails with `SignatureFailure`, marking the slot DEAD (replay emits a
/// `replay-stage-mark_dead_slot` datapoint whose error is `InvalidTransaction(SignatureFailure)`).
/// No slot carrying a PQ vote can be rooted by a peer, so `spend_and_verify` never confirms and
/// this test hangs.
///
/// A secure fix requires the recorded block to carry the ML-DSA proof and the replay/entry
/// verification path to ML-DSA-verify it (a consensus block-format change), which is out of scope
/// here. Ignored until then; run explicitly with `--ignored` to reproduce the dead-slot blocker.
/// See docs/ml-dsa-44/remaining-work.md.
#[test]
#[serial]
#[ignore = "multi-node PQ vote finalization blocked: peers fail replay sigverify of 0x00 vote txs \
            (synthetic Ed25519 sig); needs a block-format/replay-verify change. See test doc."]
fn test_mldsa_votes_finalize_cluster() {
    solana_logger::setup_with_default(RUST_LOG_FILTER);
    let mut config = all_genesis_config(|configs| {
        for cfg in configs.iter_mut() {
            let voter = Arc::new(MlDsaKeypair::new().expect("mint ML-DSA voter"));
            cfg.ml_dsa_voter = Some(voter);
        }
    });
    let cluster = LocalCluster::new(&mut config, SocketAddrSpace::Unspecified);
    spend_and_verify(&cluster);
}

/// Phase 3 across a real cluster under strict enforcement: every leader ML-DSA-signs the Merkle
/// root of every FEC set it broadcasts, and every node runs turbine sigverify in *gating* mode
/// (`--ml-dsa-shred-strict`), dropping any peer ML-DSA shred that fails post-quantum
/// verification. A node's own shreds bypass turbine verify, so the only way the cluster keeps
/// rooting is if each node's real ML-DSA shreds verify on its *peer* across turbine. (Votes stay
/// Ed25519 here to isolate the shred path.)
#[test]
#[serial]
fn test_mldsa_shred_strict_turbine_verify() {
    solana_logger::setup_with_default(RUST_LOG_FILTER);
    let mut config = all_genesis_config(|configs| {
        for cfg in configs.iter_mut() {
            let shred_key = Arc::new(MlDsaKeypair::new().expect("mint ML-DSA shred key"));
            cfg.ml_dsa_shred = Some(shred_key);
            cfg.ml_dsa_shred_strict = true;
        }
    });
    let cluster = LocalCluster::new(&mut config, SocketAddrSpace::Unspecified);
    spend_and_verify(&cluster);
}

/// CI-executable coverage of the harness change itself: the genesis repoint in `LocalCluster::new`
/// must actually run and fund each node's ML-DSA voter address (the full-finalization proof lives
/// in the `#[ignore]`d test above, blocked by replay sigverify). Building a PQ-voter cluster
/// succeeds — startup and gossip discovery are unaffected by the vote blocker — so we build it,
/// assert the genesis side effects, and tear down without waiting for roots.
#[test]
#[serial]
fn test_mldsa_genesis_repoint_funds_voter_addresses() {
    solana_logger::setup_with_default(RUST_LOG_FILTER);
    let voters: Vec<Arc<MlDsaKeypair>> = (0..NUM_NODES)
        .map(|_| Arc::new(MlDsaKeypair::new().expect("mint ML-DSA voter")))
        .collect();
    let expected_addrs: Vec<Pubkey> = voters.iter().map(|voter| voter.address()).collect();
    let voters_for_config = voters.clone();
    let mut config = all_genesis_config(move |configs| {
        for (cfg, voter) in configs.iter_mut().zip(voters_for_config.iter()) {
            cfg.ml_dsa_voter = Some(voter.clone());
        }
    });
    let cluster = LocalCluster::new(&mut config, SocketAddrSpace::Unspecified);

    // The repoint loop funds each ML-DSA voter address with a system account; its absence means
    // the loop never ran (or ran on the wrong node), which would silently disable PQ voting.
    for addr in &expected_addrs {
        let account = cluster.genesis_config.accounts.get(addr).unwrap_or_else(|| {
            panic!("ML-DSA voter address {addr} was not funded in genesis; repoint did not run")
        });
        assert_eq!(account.owner, system_program::id());
        assert!(account.lamports > 0);
    }
}

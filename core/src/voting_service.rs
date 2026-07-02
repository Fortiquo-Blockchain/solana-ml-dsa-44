use {
    crate::{
        consensus::tower_storage::{SavedTowerVersions, TowerStorage},
        next_leader::next_leader_tpu_vote,
    },
    crossbeam_channel::Receiver,
    solana_gossip::cluster_info::ClusterInfo,
    solana_measure::measure::Measure,
    solana_poh::poh_recorder::PohRecorder,
    solana_sdk::{clock::Slot, transaction::Transaction},
    std::{
        sync::{Arc, RwLock},
        thread::{self, Builder, JoinHandle},
    },
};

pub enum VoteOp {
    PushVote {
        tx: Transaction,
        tower_slots: Vec<Slot>,
        saved_tower: SavedTowerVersions,
    },
    RefreshVote {
        tx: Transaction,
        last_voted_slot: Slot,
    },
    // Post-quantum ML-DSA-44 votes: a bincoded standard `VersionedTransaction`
    // carrying the ML-DSA proof as a carrier precompile instruction (see
    // `solana_sdk::ml_dsa_envelope`), submitted to the node's own regular TPU.
    // It must NOT go to the vote-only port: there sigverify runs with
    // `reject_non_vote=true`, which disables the envelope fallback in `verify_packet`
    // (and the 2-instruction carrier tx is not a simple-vote anyway), so the vote
    // would be dropped at ingress. Not gossip CRDS either (that is Ed25519-typed).
    PushMlDsaVote {
        wire: Vec<u8>,
        tower_slots: Vec<Slot>,
        saved_tower: SavedTowerVersions,
    },
    RefreshMlDsaVote {
        wire: Vec<u8>,
        last_voted_slot: Slot,
    },
}

pub struct VotingService {
    thread_hdl: JoinHandle<()>,
}

impl VotingService {
    pub fn new(
        vote_receiver: Receiver<VoteOp>,
        cluster_info: Arc<ClusterInfo>,
        poh_recorder: Arc<RwLock<PohRecorder>>,
        tower_storage: Arc<dyn TowerStorage>,
    ) -> Self {
        let thread_hdl = Builder::new()
            .name("solVoteService".to_string())
            .spawn(move || {
                for vote_op in vote_receiver.iter() {
                    Self::handle_vote(
                        &cluster_info,
                        &poh_recorder,
                        tower_storage.as_ref(),
                        vote_op,
                    );
                }
            })
            .unwrap();
        Self { thread_hdl }
    }

    pub fn handle_vote(
        cluster_info: &ClusterInfo,
        poh_recorder: &RwLock<PohRecorder>,
        tower_storage: &dyn TowerStorage,
        vote_op: VoteOp,
    ) {
        // Persist the tower for any push vote (Ed25519 or ML-DSA).
        let saved_tower = match &vote_op {
            VoteOp::PushVote { saved_tower, .. } | VoteOp::PushMlDsaVote { saved_tower, .. } => {
                Some(saved_tower)
            }
            _ => None,
        };
        if let Some(saved_tower) = saved_tower {
            let mut measure = Measure::start("tower_save-ms");
            if let Err(err) = tower_storage.store(saved_tower) {
                error!("Unable to save tower to storage: {:?}", err);
                std::process::exit(1);
            }
            measure.stop();
            inc_new_counter_info!("tower_save-ms", measure.as_ms() as usize);
        }

        match vote_op {
            VoteOp::PushVote {
                tx, tower_slots, ..
            } => {
                let _ = cluster_info.send_transaction(
                    &tx,
                    next_leader_tpu_vote(cluster_info, poh_recorder)
                        .map(|(_pubkey, target_addr)| target_addr),
                );
                cluster_info.push_vote(&tower_slots, tx);
            }
            VoteOp::RefreshVote {
                tx,
                last_voted_slot,
            } => {
                let _ = cluster_info.send_transaction(
                    &tx,
                    next_leader_tpu_vote(cluster_info, poh_recorder)
                        .map(|(_pubkey, target_addr)| target_addr),
                );
                cluster_info.refresh_vote(tx, last_voted_slot);
            }
            // ML-DSA votes go to the node's OWN regular TPU (None target), never the
            // vote-only port (its reject_non_vote sigverify disables the envelope
            // fallback) nor gossip CRDS. See `VoteOp::PushMlDsaVote`.
            VoteOp::PushMlDsaVote { wire, .. } | VoteOp::RefreshMlDsaVote { wire, .. } => {
                let _ = cluster_info.send_transaction_raw(&wire, None);
            }
        }
    }

    pub fn join(self) -> thread::Result<()> {
        self.thread_hdl.join()
    }
}

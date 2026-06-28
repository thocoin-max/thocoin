use crate::core::consensus::*;
use crate::core::hash::*;
use crate::core::tx::*;

#[test]
fn reward_halving() {
    assert_eq!(block_reward(0, 0), INITIAL_REWARD);
    assert_eq!(block_reward(HALVING_INTERVAL, 0), INITIAL_REWARD / 2);
    assert_eq!(block_reward(HALVING_INTERVAL * 2, 0), INITIAL_REWARD / 4);
    assert_eq!(block_reward(0, MAX_SUPPLY), 0);
}

#[test]
fn reward_capped_by_supply() {
    let near = MAX_SUPPLY - 10;
    assert_eq!(block_reward(0, near), 10);
}

#[test]
fn work_increases_with_difficulty() {
    let easy = work_from_bits(0x1f00ffff);
    let hard = work_from_bits(0x1d00ffff);
    assert_eq!(target_cmp(&hard, &easy), std::cmp::Ordering::Greater);
}

#[test]
fn work_accumulates() {
    let w = work_from_bits(GENESIS_BITS);
    let sum = target_add(&w, &w);
    assert_eq!(target_cmp(&sum, &w), std::cmp::Ordering::Greater);
}

#[test]
fn lwma_within_pow_limit() {
    let n = LWMA_WINDOW as usize + 1;
    let times: Vec<u64> = (0..n as u64).map(|i| i * TARGET_BLOCK_TIME).collect();
    let bits: Vec<u32> = vec![GENESIS_BITS; n];
    let next = lwma_next_bits(&times, &bits);
    let limit = bits_to_target(POW_LIMIT_BITS);
    let nt = bits_to_target(next);
    assert_ne!(next, 0);
    assert!(target_cmp(&nt, &limit) != std::cmp::Ordering::Greater);
}

#[test]
fn lwma_fast_blocks_raise_difficulty() {
    let n = LWMA_WINDOW as usize + 1;
    let times: Vec<u64> = (0..n as u64).map(|i| i * (TARGET_BLOCK_TIME / 5)).collect();
    let bits: Vec<u32> = vec![GENESIS_BITS; n];
    let next = lwma_next_bits(&times, &bits);
    let next_t = bits_to_target(next);
    let base_t = bits_to_target(GENESIS_BITS);
    assert_eq!(target_cmp(&next_t, &base_t), std::cmp::Ordering::Less);
}

fn dummy_in(sig: Vec<u8>, pk: Vec<u8>) -> TxIn {
    TxIn { prev: OutPoint { txid: [1u8; 32], vout: 0 }, signature: sig, pubkey: pk, sequence: 0 }
}

#[test]
fn txid_ignores_witness() {
    let base = Transaction {
        version: 1,
        inputs: vec![dummy_in(vec![9, 9], vec![1, 2, 3])],
        outputs: vec![TxOut { value: 5, script_pubkey: vec![0xAA] }],
        lock_time: 0,
    };
    let mut other_sig = base.clone();
    other_sig.inputs[0].signature = vec![7, 7, 7, 7];
    let mut other_pk = base.clone();
    other_pk.inputs[0].pubkey = vec![9, 9, 9];
    assert_eq!(base.txid(), other_sig.txid());
    assert_eq!(base.txid(), other_pk.txid());
    assert_ne!(base.wtxid(), other_sig.wtxid());
}

#[test]
fn txid_depends_on_outputs() {
    let a = Transaction {
        version: 1, inputs: vec![dummy_in(vec![], vec![])],
        outputs: vec![TxOut { value: 5, script_pubkey: vec![0xAA] }], lock_time: 0,
    };
    let mut b = a.clone();
    b.outputs[0].value = 6;
    assert_ne!(a.txid(), b.txid());
}

#[test]
fn coinbase_txid_unique_per_height() {
    let a = Transaction::coinbase(10, 100, vec![0xAA]);
    let b = Transaction::coinbase(11, 100, vec![0xAA]);
    assert_ne!(a.txid(), b.txid());
}

#[test]
fn genesis_is_pinned() {
    let g = crate::core::chain::genesis_block();
    let hex = hash_to_hex(&g.hash());
    println!("GENESIS_HASH={hex}");
    assert!(crate::core::block::check_witness_commitment(&g));
    assert_eq!(g.header.merkle_root, crate::core::block::Block::merkle_root(&g.transactions));
    if !GENESIS_HASH_HEX.is_empty() {
        assert_eq!(hex, GENESIS_HASH_HEX);
    }
}

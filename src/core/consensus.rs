use crate::core::hash::{Hash, bits_to_target, target_to_bits, target_add, target_mul_div, target_cmp};

pub const COIN: u64 = 100_000_000;
pub const MAX_SUPPLY: u64 = 198_700_000 * COIN;
pub const INITIAL_REWARD: u64 = 22_020_000_000;
pub const HALVING_INTERVAL: u64 = 458_440;
pub const TARGET_BLOCK_TIME: u64 = 275;
pub const LWMA_WINDOW: u64 = 90;
pub const COINBASE_MATURITY: u64 = 100;
pub const MAX_BLOCK_SIZE: usize = 1_000_000;
pub const MAX_BLOCK_SIGOPS: usize = 4_000;
pub const GENESIS_BITS: u32 = 0x1d00ffff;
pub const POW_LIMIT_BITS: u32 = 0x1d00ffff;
pub const NETWORK_MAGIC: u32 = 0xC222C222;
pub const P2P_PORT: u16 = 22221;
pub const RPC_PORT: u16 = 22222;
pub const ADDRESS_PREFIX: u8 = 0x32;
pub const MIN_RELAY_FEE_PER_KB: u64 = 1_000;

pub const SEED_NODES: &[&str] = &[
];

pub const CHECKPOINTS: &[(u64, &str)] = &[
];

pub fn checkpoint_hash(height: u64) -> Option<Hash> {
    for (h, hexs) in CHECKPOINTS {
        if *h == height {
            let mut bytes = hex::decode(hexs).ok()?;
            if bytes.len() != 32 { return None; }
            bytes.reverse();
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            return Some(out);
        }
    }
    None
}

pub fn last_checkpoint_height() -> u64 {
    CHECKPOINTS.iter().map(|(h, _)| *h).max().unwrap_or(0)
}

pub fn block_reward(height: u64, current_supply: u64) -> u64 {
    if current_supply >= MAX_SUPPLY { return 0; }
    let halvings = height / HALVING_INTERVAL;
    if halvings >= 64 { return 0; }
    let reward = INITIAL_REWARD >> halvings;
    let remaining = MAX_SUPPLY - current_supply;
    reward.min(remaining)
}

pub fn lwma_next_bits(times: &[u64], bits: &[u32]) -> u32 {
    let n = times.len().saturating_sub(1);
    if n == 0 || bits.len() != times.len() {
        return *bits.last().unwrap_or(&GENESIS_BITS);
    }
    let t = TARGET_BLOCK_TIME as i128;
    let mut weighted: i128 = 0;
    let mut sum_target = [0u8; 32];
    for i in 1..=n {
        let mut st = times[i] as i128 - times[i - 1] as i128;
        if st < 1 { st = 1; }
        if st > 6 * t { st = 6 * t; }
        weighted += st * (i as i128);
        sum_target = target_add(&sum_target, &bits_to_target(bits[i]));
    }
    let k = (n as i128) * (n as i128 + 1) / 2;
    let avg = target_mul_div(&sum_target, 1, n as u64);
    let denom = (k * t) as u64;
    let mut next = target_mul_div(&avg, weighted as u64, denom);
    let limit = bits_to_target(POW_LIMIT_BITS);
    if target_cmp(&next, &limit) == std::cmp::Ordering::Greater {
        next = limit;
    }
    let bits_out = target_to_bits(&next);
    if bits_out == 0 { POW_LIMIT_BITS } else { bits_out }
}

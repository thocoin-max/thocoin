use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
#[cfg(feature = "gpu")]
use std::sync::atomic::Ordering;
#[cfg(feature = "gpu")]
use std::thread;
#[cfg(feature = "gpu")]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(feature = "gpu")]
use ocl::{Buffer, MemFlags, ProQue};

#[cfg(feature = "gpu")]
use crate::core::block::Block;
use crate::core::chain::ChainState;
#[cfg(feature = "gpu")]
use crate::core::consensus::*;
use crate::core::mempool::Mempool;
#[cfg(feature = "gpu")]
use crate::core::tx::Transaction;
use crate::wallet::Wallet;

#[cfg(feature = "gpu")]
const KERNEL_SRC: &str = r#"
__constant uint K[64] = {
    0x428a2f98U,0x71374491U,0xb5c0fbcfU,0xe9b5dba5U,0x3956c25bU,0x59f111f1U,0x923f82a4U,0xab1c5ed5U,
    0xd807aa98U,0x12835b01U,0x243185beU,0x550c7dc3U,0x72be5d74U,0x80deb1feU,0x9bdc06a7U,0xc19bf174U,
    0xe49b69c1U,0xefbe4786U,0x0fc19dc6U,0x240ca1ccU,0x2de92c6fU,0x4a7484aaU,0x5cb0a9dcU,0x76f988daU,
    0x983e5152U,0xa831c66dU,0xb00327c8U,0xbf597fc7U,0xc6e00bf3U,0xd5a79147U,0x06ca6351U,0x14292967U,
    0x27b70a85U,0x2e1b2138U,0x4d2c6dfcU,0x53380d13U,0x650a7354U,0x766a0abbU,0x81c2c92eU,0x92722c85U,
    0xa2bfe8a1U,0xa81a664bU,0xc24b8b70U,0xc76c51a3U,0xd192e819U,0xd6990624U,0xf40e3585U,0x106aa070U,
    0x19a4c116U,0x1e376c08U,0x2748774cU,0x34b0bcb5U,0x391c0cb3U,0x4ed8aa4aU,0x5b9cca4fU,0x682e6ff3U,
    0x748f82eeU,0x78a5636fU,0x84c87814U,0x8cc70208U,0x90befffaU,0xa4506cebU,0xbef9a3f7U,0xc67178f2U
};

#define ROTR(x,n) (((x) >> (n)) | ((x) << (32-(n))))
#define BS0(x) (ROTR(x,2) ^ ROTR(x,13) ^ ROTR(x,22))
#define BS1(x) (ROTR(x,6) ^ ROTR(x,11) ^ ROTR(x,25))
#define SS0(x) (ROTR(x,7) ^ ROTR(x,18) ^ ((x) >> 3))
#define SS1(x) (ROTR(x,17) ^ ROTR(x,19) ^ ((x) >> 10))

inline void sha256_compress(uint *state, const uint *w_in) {
    uint w[64];
    for (int i = 0; i < 16; i++) w[i] = w_in[i];
    for (int i = 16; i < 64; i++)
        w[i] = SS1(w[i-2]) + w[i-7] + SS0(w[i-15]) + w[i-16];
    uint a = state[0], b = state[1], c = state[2], d = state[3];
    uint e = state[4], f = state[5], g = state[6], h = state[7];
    for (int i = 0; i < 64; i++) {
        uint t1 = h + BS1(e) + ((e & f) ^ (~e & g)) + K[i] + w[i];
        uint t2 = BS0(a) + ((a & b) ^ (a & c) ^ (b & c));
        h = g; g = f; f = e; e = d + t1;
        d = c; c = b; b = a; a = t1 + t2;
    }
    state[0] += a; state[1] += b; state[2] += c; state[3] += d;
    state[4] += e; state[5] += f; state[6] += g; state[7] += h;
}

inline uint bswap32(uint v) {
    return ((v & 0xFFU) << 24) | ((v & 0xFF00U) << 8) |
           ((v & 0xFF0000U) >> 8) | ((v >> 24) & 0xFFU);
}

__kernel void mine(
    __global const uint *header_be,
    uint nonce_base,
    __global const uint *target_be,
    __global uint *result
) {
    uint gid = get_global_id(0);
    uint nonce = nonce_base + gid;
    uint w1[16];
    for (int i = 0; i < 16; i++) w1[i] = header_be[i];
    uint state[8] = {
        0x6a09e667U,0xbb67ae85U,0x3c6ef372U,0xa54ff53aU,
        0x510e527fU,0x9b05688cU,0x1f83d9abU,0x5be0cd19U
    };
    sha256_compress(state, w1);
    uint w2[16];
    w2[0]  = header_be[16]; w2[1] = header_be[17]; w2[2] = header_be[18];
    w2[3]  = nonce;
    w2[4]  = 0x80000000U;
    for (int i = 5; i < 15; i++) w2[i] = 0;
    w2[15] = 640;
    sha256_compress(state, w2);
    uint w3[16];
    for (int i = 0; i < 8; i++) w3[i] = state[i];
    w3[8]  = 0x80000000U;
    for (int i = 9; i < 15; i++) w3[i] = 0;
    w3[15] = 256;
    uint state2[8] = {
        0x6a09e667U,0xbb67ae85U,0x3c6ef372U,0xa54ff53aU,
        0x510e527fU,0x9b05688cU,0x1f83d9abU,0x5be0cd19U
    };
    sha256_compress(state2, w3);
    uint hash_be[8];
    for (int i = 0; i < 8; i++) hash_be[i] = bswap32(state2[7 - i]);
    bool meets = false;
    for (int i = 0; i < 8; i++) {
        if (hash_be[i] < target_be[i]) { meets = true; break; }
        if (hash_be[i] > target_be[i]) { meets = false; break; }
        if (i == 7) meets = true;
    }
    if (meets) {
        if (atomic_cmpxchg(&result[0], 0u, 1u) == 0u) result[1] = nonce;
    }
}
"#;

#[cfg(feature = "gpu")]
pub struct GpuMiner {
    pub chain: Arc<ChainState>,
    pub mempool: Mempool,
    pub wallet: Arc<Wallet>,
    pub running: Arc<AtomicBool>,
    pub hashrate: Arc<AtomicU64>,
    pub blocks_found: Arc<AtomicU64>,
    pub device_name: Arc<parking_lot::RwLock<String>>,
    pub available: Arc<AtomicBool>,
}

#[cfg(feature = "gpu")]
impl GpuMiner {
    pub fn new(chain: Arc<ChainState>, mempool: Mempool, wallet: Arc<Wallet>) -> Self {
        let m = Self {
            chain, mempool, wallet,
            running: Arc::new(AtomicBool::new(false)),
            hashrate: Arc::new(AtomicU64::new(0)),
            blocks_found: Arc::new(AtomicU64::new(0)),
            device_name: Arc::new(parking_lot::RwLock::new(String::new())),
            available: Arc::new(AtomicBool::new(false)),
        };
        m.probe();
        m
    }

    fn probe(&self) {
        match ProQue::builder().src(KERNEL_SRC).dims(1024usize).build() {
            Ok(pq) => {
                let name = pq.device().name().unwrap_or_else(|_| "Unknown GPU".to_string());
                *self.device_name.write() = name;
                self.available.store(true, Ordering::SeqCst);
            }
            Err(err) => {
                *self.device_name.write() = format!("OpenCL unavailable: {}", err);
                self.available.store(false, Ordering::SeqCst);
            }
        }
    }

    pub fn start(self: &Arc<Self>) {
        if self.running.load(Ordering::SeqCst) { return; }
        if !self.available.load(Ordering::SeqCst) { return; }
        self.running.store(true, Ordering::SeqCst);
        let me = self.clone();
        thread::spawn(move || me.mine_loop());
    }

    pub fn stop(&self) { self.running.store(false, Ordering::SeqCst); }

    fn build_candidate(&self) -> Option<(Block, Vec<[u8; 32]>)> {
        let prev = *self.chain.tip.read();
        let height = *self.chain.height.read() + 1;
        let supply = *self.chain.supply.read();
        let reward = block_reward(height, supply);
        if reward == 0 { return None; }
        let (snap, fees) = self.mempool.snapshot_with_fees(500);
        let txids: Vec<[u8; 32]> = snap.iter().map(|t| t.txid()).collect();
        let coinbase = Transaction::coinbase(height, reward.saturating_add(fees),
            self.wallet.key.read().script_pubkey());
        let mut txs = vec![coinbase];
        txs.extend(snap);
        crate::core::block::add_witness_commitment(&mut txs);
        let bits = self.chain.current_bits();
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        Some((Block::new(prev, txs, bits, ts), txids))
    }

    fn mine_loop(self: Arc<Self>) {
        let pq = match ProQue::builder().src(KERNEL_SRC).dims(1usize << 20).build() {
            Ok(pq) => pq,
            Err(_) => { self.running.store(false, Ordering::SeqCst); return; }
        };

        while self.running.load(Ordering::SeqCst) {
            let Some((mut block, txids)) = self.build_candidate() else {
                thread::sleep(Duration::from_secs(1));
                continue;
            };

            let mut header_buf = [0u8; 76];
            header_buf[0..4].copy_from_slice(&block.header.version.to_be_bytes());
            header_buf[4..36].copy_from_slice(&block.header.prev_hash);
            header_buf[36..68].copy_from_slice(&block.header.merkle_root);
            header_buf[68..72].copy_from_slice(&(block.header.timestamp as u32).to_be_bytes());
            header_buf[72..76].copy_from_slice(&block.header.bits.to_be_bytes());

            let mut header_words = [0u32; 19];
            for i in 0..19 {
                header_words[i] = u32::from_be_bytes([
                    header_buf[i*4], header_buf[i*4+1], header_buf[i*4+2], header_buf[i*4+3]
                ]);
            }

            let target = crate::core::hash::bits_to_target(block.header.bits);
            let mut target_be = [0u32; 8];
            for i in 0..8 {
                target_be[i] = u32::from_be_bytes([
                    target[i*4], target[i*4+1], target[i*4+2], target[i*4+3]
                ]);
            }

            let header_cl = match Buffer::<u32>::builder()
                .queue(pq.queue().clone())
                .flags(MemFlags::new().read_only().copy_host_ptr())
                .len(19).copy_host_slice(&header_words).build() { Ok(b)=>b, Err(_)=>continue };
            let target_cl = match Buffer::<u32>::builder()
                .queue(pq.queue().clone())
                .flags(MemFlags::new().read_only().copy_host_ptr())
                .len(8).copy_host_slice(&target_be).build() { Ok(b)=>b, Err(_)=>continue };
            let result_buf = match Buffer::<u32>::builder()
                .queue(pq.queue().clone())
                .flags(MemFlags::new().read_write())
                .len(2).build() { Ok(b)=>b, Err(_)=>continue };

            let mut nonce_base: u32 = 0;
            let batch: u32 = 1 << 20;
            let tip_at_start = *self.chain.tip.read();

            loop {
                if !self.running.load(Ordering::SeqCst) { return; }
                if *self.chain.tip.read() != tip_at_start { break; }

                let zero = [0u32; 2];
                if result_buf.write(&zero[..]).enq().is_err() { break; }

                let kernel = match pq.kernel_builder("mine")
                    .arg(&header_cl).arg(nonce_base).arg(&target_cl).arg(&result_buf)
                    .build() { Ok(k)=>k, Err(_)=>break };

                unsafe {
                    if kernel.cmd().global_work_size(batch as usize).enq().is_err() { break; }
                }
                if pq.queue().finish().is_err() { break; }
                self.hashrate.fetch_add(batch as u64, Ordering::Relaxed);

                let mut out = [0u32; 2];
                if result_buf.read(&mut out[..]).enq().is_err() { break; }

                if out[0] == 1 {
                    block.header.nonce = out[1];
                    if block.header.meets_pow() {
                        let h = *self.chain.height.read() + 1;
                        if self.chain.apply_block(&block, h).is_ok() {
                            self.blocks_found.fetch_add(1, Ordering::Relaxed);
                            self.mempool.remove_txs(&txids);
                        }
                    }
                    break;
                }

                nonce_base = nonce_base.wrapping_add(batch);
                if nonce_base == 0 { break; }
            }
        }
    }
}

#[cfg(not(feature = "gpu"))]
pub struct GpuMiner {
    pub chain: Arc<ChainState>,
    pub mempool: Mempool,
    pub wallet: Arc<Wallet>,
    pub running: Arc<AtomicBool>,
    pub hashrate: Arc<AtomicU64>,
    pub blocks_found: Arc<AtomicU64>,
    pub device_name: Arc<parking_lot::RwLock<String>>,
    pub available: Arc<AtomicBool>,
}

#[cfg(not(feature = "gpu"))]
impl GpuMiner {
    pub fn new(chain: Arc<ChainState>, mempool: Mempool, wallet: Arc<Wallet>) -> Self {
        Self {
            chain, mempool, wallet,
            running: Arc::new(AtomicBool::new(false)),
            hashrate: Arc::new(AtomicU64::new(0)),
            blocks_found: Arc::new(AtomicU64::new(0)),
            device_name: Arc::new(parking_lot::RwLock::new(
                "GPU mining disabled in this build".to_string())),
            available: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn start(self: &Arc<Self>) {}
    pub fn stop(&self) {}
}
# ThoCoin

**Post-quantum Proof-of-Work cryptocurrency** written in Rust. ThoCoin uses
ML-DSA-87 (FIPS-204, NIST Level 5) lattice signatures instead of ECDSA, a SHA-256d Proof-of-Work
header, per-block LWMA difficulty, and heaviest-chain consensus with checkpoints.
It ships as a single desktop app that is wallet, full node, solo miner and pool
miner at once.

Website: **https://thocoin.org**

---

## Download (end users)

Get the latest build from **https://thocoin.org/download** or the
[Releases](../../releases) page:

- `ThoCoin-Setup.exe` — Windows installer (GUI wallet + node + miner).
  During install you choose a **CPU edition** (software rendering, runs on any PC
  or VM) or a **GPU edition** (hardware rendering + OpenCL GPU mining).
- `ThoCoin-win64.zip` — portable binaries (no installer).

Verify your download:
```
certutil -hashfile ThoCoin-Setup.exe SHA256
```
Compare the result against `SHA256SUMS.txt` in the same release.

> All participants must run the **same release**. Older builds use different
> consensus rules and will fork off the network.

---

## Network parameters

| Parameter | Value |
|---|---|
| Algorithm | Proof-of-Work, SHA-256d header |
| Signatures | ML-DSA-87 (FIPS-204), post-quantum — NIST Level 5 |
| Max supply | 198,700,000 THO |
| Initial reward | 220.2 THO / block |
| Halving | every 458,440 blocks |
| Block time | 275 s (LWMA, retargets every block) |
| Difficulty window | 90 blocks (LWMA-1) |
| Coinbase maturity | 0 blocks (spendable immediately) |
| Transaction fees | added to the coinbase by the miner |
| Max block size | 1,000,000 bytes |
| Address format | Base58Check, prefix `0x32` (addresses start with `M`) |
| Smallest unit | 1 THO = 100,000,000 base units |
| P2P port | 22221 |
| RPC port | 22222 (localhost, token-authenticated) |
| Pool stratum port | 23333 |
| Recovery phrase | BIP39, 24 words (256-bit) |
| Premine | none (genesis coinbase is unspendable) |

---

## Binaries

| Binary | Role |
|---|---|
| `thocoin-gui` | Wallet + node + solo miner + pool miner (desktop GUI) |
| `thocoind` | Headless full node (no GUI) |
| `thocoin-pool` | Mining pool server |
| `thocoin-pool-miner` | Command-line pool miner |

---

## Quick start

### Wallet & mining (most users)
1. Install and launch **ThoCoin** (`thocoin-gui.exe`).
2. The app creates a wallet automatically; back up your **24-word recovery
   phrase** from **Receive → Show recovery phrase**.
3. **Mining tab** — solo mine with your CPU or GPU.
4. **Join Pool tab** — mine in the official pool and earn rewards proportional to
   your shares (PPLNS). Rewards are paid directly to your wallet address.

### Wallet security
- Encrypt the wallet at rest by setting a passphrase before first run:
  ```
  setx THOCOIN_WALLET_PASS "your-strong-passphrase"
  ```
  Without it the app refuses to store a plaintext seed unless you explicitly set
  `THOCOIN_WALLET_ALLOW_PLAINTEXT=1`.
- Optional 2FA on RPC spends: place a Base32 TOTP secret in `wallet.totp` next to
  the wallet file.

---

## Verify our claims (no trust required)

ThoCoin is fully verifiable from source — don't take our word for it:

- **Post-quantum, no ECDSA:** the entire signing path lives in
  `src/wallet/address.rs` and uses only `fips204::ml_dsa_87`. Grep the tree for
  `ecdsa`/`secp256k1` — there is none.
- **Signature/key sizes:** any spending transaction exposes a `pubkey_len` of
  **2592** and a `sig_len` of **4627** via the RPC `gettransaction` — the exact
  ML-DSA-87 sizes (vs 33/72 bytes for ECDSA).
- **No premine:** the genesis coinbase pays to `script_p2pkh(&[0u8; 20])`, an
  address nobody holds the key to. Inspect `genesis_block()` in
  `src/core/chain.rs`.
- **Pinned genesis:** `GENESIS_HASH_HEX` in `src/core/consensus.rs` locks the
  genesis block; any node computing a different hash refuses to start.

---

## Pool mining

### Joining a pool (miner)
In the GUI open **Join Pool**, keep the official pool or enter any
`host:port`, then **Start**. The miner subscribes with your reward address,
grinds shares on CPU or GPU according to your edition, and auto-reconnects.
Rewards are paid **inside the coinbase** of every block the pool finds, split by
PPLNS share count, so they land in your wallet the moment a block is mined — no
separate payout transaction, nothing to wait for beyond the block itself.

### Hosting a pool (operator)
The pool server is a full node that must be in sync with the live chain before it
serves miners.

**Dedicated machine / VPS** (recommended):
```
thocoin-pool.exe 0.0.0.0:23333
```
It syncs from the DNS seeds, then listens for miners on port 23333.

**Same machine as a running wallet** (the wallet already holds P2P port 22221, so
give the pool its own P2P port and point it at the wallet):
```
set THOCOIN_P2P_PORT=22231
set THOCOIN_PEERS=127.0.0.1:22221
thocoin-pool.exe
```
The pool binds P2P on 22231, syncs the chain from your wallet, and only starts
accepting miners once it has caught up. Forward/open **TCP 23333** so external
miners can connect.

### Command-line miner
```
thocoin-pool-miner.exe <host:port> <THO_address> [threads]
```

---

## RPC

The node exposes a JSON-RPC endpoint on `127.0.0.1:22222`, protected by a bearer
token printed at startup (or fixed via `THOCOIN_RPC_TOKEN`).

```
curl -X POST http://127.0.0.1:22222 ^
  -H "Content-Type: application/json" ^
  -H "Authorization: Bearer <token>" ^
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getinfo\",\"params\":[]}"
```

Available methods include `getinfo`, `getbalance`, `getaddress`, `send`,
`getblockcount`, `getbestblockhash`, `getblockhash`, `getblock`,
`getblockbyheight`, `gettransaction`, `getrawmempool`, `getmempoolinfo`,
`getaddressinfo`, and `getrichlist` — enough to back a block explorer.

---

## Environment variables

| Variable | Purpose |
|---|---|
| `THOCOIN_DATA` | Override the data directory (headless node) |
| `THOCOIN_PEERS` | Comma-separated bootstrap peers, e.g. `ip:22221,ip:22221` |
| `THOCOIN_P2P_PORT` | Override the P2P listen port (run a second node on one box) |
| `THOCOIN_RPC_BIND` | RPC bind address (default `127.0.0.1`) |
| `THOCOIN_RPC_TOKEN` | Fixed RPC bearer token |
| `THOCOIN_WALLET_PASS` | Encrypt the wallet seed at rest |
| `THOCOIN_WALLET_ALLOW_PLAINTEXT` | Set to `1` to allow an unencrypted seed |

---

## Build from source

Requires Rust (stable). The GPU miner additionally needs an OpenCL runtime.
```
git clone https://github.com/thocoin-max/thocoin
cd thocoin
cargo build --release                 # CPU edition
cargo build --release --features gpu  # GPU edition (OpenCL)
cargo test
```
Binaries land in `target/release/`. On Windows you can produce a one-click
installer with `build-release.bat` followed by compiling `thocoin-installer.iss`
in Inno Setup.

---

## Consensus status

**Implemented:** post-quantum ML-DSA-87 signatures throughout (no ECDSA),
cumulative chainwork with heaviest-chain fork choice and reorg,
hardcoded checkpoints (no deep reorg past the last checkpoint), per-block LWMA
difficulty, fee-rate mempool with a minimum relay fee, witness-excluded
(non-malleable) txids, transaction fees folded into the coinbase, PPLNS pool
payouts paid in-coinbase, and a P2P layer with INV relay, peer discovery and ban
scoring.

**Known limitations:** the UTXO set is held in RAM (persisted to disk but not
paged); an independent security audit is pending before a hardened mainnet.

---

## Community

- Website: https://thocoin.org
- Explorer: https://thocoin.org/explorer
- Telegram: https://t.me/thocoin

---

## License

See [LICENSE](LICENSE).
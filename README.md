# ThoCoin

Post-quantum Proof-of-Work UTXO cryptocurrency written in Rust. Uses ML-DSA-44 (FIPS-204) signatures, LWMA per-block difficulty, and heaviest-chain consensus with checkpoints.

## Download (end users)

Get the latest build from the [**Releases**](../../releases) page:
- `ThoCoin-Setup-x.y.z.exe` — Windows installer (GUI wallet + node)
- or `ThoCoin-x.y.z-win64.zip` — portable binaries

Verify your download:
```
certutil -hashfile ThoCoin-x.y.z-win64.zip SHA256
```
Compare the result with `SHA256SUMS.txt` published in the same release.

> All participants must run the **same release**. Older builds use different consensus rules and will fork off the network.

## Parameters

| | |
|---|---|
| Algorithm | Proof-of-Work, SHA-256d header, ML-DSA-44 (FIPS-204) signatures |
| Max supply | 198,700,000 THO |
| Initial reward | 220.2 THO/block |
| Halving | every 458,440 blocks |
| Block time | 275 s (LWMA, retargets every block) |
| Difficulty window | 90 blocks (LWMA-1) |
| Coinbase maturity | 100 blocks |
| Max block size | 1,000,000 bytes |
| Address prefix | 0x32 (Base58Check) |
| P2P port | 22221 |
| RPC port | 22222 |

## Binaries

- `thocoin-gui` — wallet + node + miner (GUI)
- `thocoind` — headless node
- `thocoin-pool` — mining pool server
- `thocoin-pool-miner` — pool miner client

## Run

GUI wallet + miner:
```
thocoin-gui.exe
```
Headless node:
```
thocoind.exe
```
Bootstrap to peers (until DNS seeds are configured):
```
setx THOCOIN_PEERS "ip1:22221,ip2:22221"
```

## RPC

```
curl -X POST http://127.0.0.1:22222 -H "Content-Type: application/json" ^
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getinfo\",\"params\":[]}"
```

## Build from source

Requires Rust (stable) and an OpenCL runtime (for the GPU miner).
```
cargo build --release
cargo test
```
Output binaries in `target/release/`.

## Consensus status

Implemented: cumulative chainwork + heaviest-chain fork choice with reorg, hardcoded checkpoints (no deep reorg past the last checkpoint), per-block LWMA difficulty, fee-rate mempool with minimum relay fee, witness-excluded txid (non-malleable), basic P2P with INV relay, peer discovery and ban scoring.

Known limitations: UTXO set is kept in RAM (persisted to disk but not paged); the miner does not yet add transaction fees to the coinbase; independent security audit pending before mainnet.

## License

See [LICENSE](LICENSE).

use std::sync::Arc;
use jsonrpc_core::{IoHandler, Params, Value};
use jsonrpc_http_server::{ServerBuilder, hyper, RequestMiddleware, RequestMiddlewareAction};
use crate::core::chain::ChainState;
use crate::wallet::Wallet;
use crate::core::consensus::RPC_PORT;
use crate::core::hash::hash_to_hex;

fn rpc_token() -> String {
    if let Ok(t) = std::env::var("THOCOIN_RPC_TOKEN") {
        if !t.is_empty() { return t; }
    }
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    let tok = hex::encode(b);
    eprintln!("[RPC] auto-generated token (set THOCOIN_RPC_TOKEN to fix it): {tok}");
    tok
}

struct AuthMiddleware { token: String }

impl RequestMiddleware for AuthMiddleware {
    fn on_request(&self, request: hyper::Request<hyper::Body>) -> RequestMiddlewareAction {
        let ok = request.headers()
            .get(hyper::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                let expected = format!("Bearer {}", self.token);
                use subtle::ConstantTimeEq;
                v.as_bytes().ct_eq(expected.as_bytes()).into()
            })
            .unwrap_or(false);
        if ok {
            request.into()
        } else {
            hyper::Response::builder()
                .status(hyper::StatusCode::UNAUTHORIZED)
                .body(hyper::Body::from("unauthorized"))
                .unwrap()
                .into()
        }
    }
}

pub fn start_rpc(chain: Arc<ChainState>, wallet: Arc<Wallet>, mempool: crate::core::mempool::Mempool) -> jsonrpc_http_server::Server {
    let mut io = IoHandler::new();

    let c = chain.clone();
    io.add_sync_method("getinfo", move |_| {
        Ok(serde_json::json!({
            "height": *c.height.read(),
            "supply": *c.supply.read(),
            "tip": hash_to_hex(&*c.tip.read()),
        }))
    });

    let w = wallet.clone();
    io.add_sync_method("getaddress", move |_| Ok(Value::String(w.address())));

    let c2 = chain.clone();
    let w2 = wallet.clone();
    io.add_sync_method("getbalance", move |_| {
        Ok(Value::String(w2.balance(&c2).to_string()))
    });

    let c3 = chain.clone();
    let w3 = wallet.clone();
    let mp = mempool.clone();
    io.add_sync_method("send", move |p: Params| {
        let arr: Vec<Value> = p.parse().map_err(|_| jsonrpc_core::Error::invalid_params("need [to, amount]"))?;
        if arr.len() < 2 {
            return Err(jsonrpc_core::Error::invalid_params("need [to, amount]"));
        }
        let to = arr[0].as_str().unwrap_or("").to_string();
        let amount = arr[1].as_u64().unwrap_or(0);

        if let Some(secret) = crate::wallet::totp::load_secret_beside(&w3.path) {
            let code = arr.get(2).and_then(|v| v.as_str()).unwrap_or("");
            if !crate::wallet::totp::verify(&secret, code) {
                return Err(jsonrpc_core::Error::invalid_params(
                    "2FA enabled: a valid TOTP code is required as the 3rd param [to, amount, code]"));
            }
        }

        match w3.send(&c3, &to, amount, 1000) {
            Ok(tx) => {
                let txid = hash_to_hex(&tx.txid());
                // Insert into the mempool so the P2P announce loop relays it.
                mp.accept(&c3, tx)
                    .map_err(|e| jsonrpc_core::Error::invalid_params(format!("mempool rejected: {e}")))?;
                Ok(Value::String(txid))
            }
            Err(e) => Err(jsonrpc_core::Error::invalid_params(e.to_string())),
        }
    });

    // ---- Explorer read-only methods ----
    let c = chain.clone();
    io.add_sync_method("getblockcount", move |_| {
        Ok(Value::from(*c.height.read()))
    });

    let c = chain.clone();
    io.add_sync_method("getbestblockhash", move |_| {
        Ok(Value::String(hash_to_hex(&c.tip_hash())))
    });

    let c = chain.clone();
    io.add_sync_method("getblockhash", move |p: Params| {
        let (height,): (u64,) = p.parse()
            .map_err(|_| jsonrpc_core::Error::invalid_params("need [height]"))?;
        match c.block_at(height) {
            Some(b) => Ok(Value::String(hash_to_hex(&b.hash()))),
            None => Err(jsonrpc_core::Error::invalid_params("height not found")),
        }
    });

    let c = chain.clone();
    io.add_sync_method("getblockbyheight", move |p: Params| {
        let (height,): (u64,) = p.parse()
            .map_err(|_| jsonrpc_core::Error::invalid_params("need [height]"))?;
        match c.block_at(height) {
            Some(b) => Ok(block_to_json(&b, &c)),
            None => Err(jsonrpc_core::Error::invalid_params("height not found")),
        }
    });

    let c = chain.clone();
    io.add_sync_method("getblock", move |p: Params| {
        let (hash_hex,): (String,) = p.parse()
            .map_err(|_| jsonrpc_core::Error::invalid_params("need [block_hash]"))?;
        let hash = hex_to_hash(&hash_hex)
            .ok_or_else(|| jsonrpc_core::Error::invalid_params("bad block hash"))?;
        match c.get_block(&hash) {
            Some(b) => Ok(block_to_json(&b, &c)),
            None => Err(jsonrpc_core::Error::invalid_params("block not found")),
        }
    });

    let c = chain.clone();
    io.add_sync_method("gettransaction", move |p: Params| {
        let (txid_hex,): (String,) = p.parse()
            .map_err(|_| jsonrpc_core::Error::invalid_params("need [txid]"))?;
        let target = hex_to_hash(&txid_hex)
            .ok_or_else(|| jsonrpc_core::Error::invalid_params("bad txid"))?;
        // Scan the active chain for the tx. Explorers index this; the node just answers by scan.
        let tip = *c.height.read();
        for h in (0..=tip).rev() {
            if let Some(b) = c.block_at(h) {
                for tx in &b.transactions {
                    if tx.txid() == target {
                        return Ok(tx_to_json(tx, Some(h), tip));
                    }
                }
            }
        }
        Err(jsonrpc_core::Error::invalid_params("tx not found in active chain"))
    });

    let c = chain.clone();
    let mp_r = mempool.clone();
    io.add_sync_method("getrawmempool", move |_| {
        let _ = &c;
        let ids: Vec<Value> = mp_r.entries.read().keys()
            .map(|h| Value::String(hash_to_hex(h))).collect();
        Ok(Value::Array(ids))
    });

    let mp_r = mempool.clone();
    io.add_sync_method("getmempoolinfo", move |_| {
        let entries = mp_r.entries.read();
        let count = entries.len();
        let bytes: usize = entries.values().map(|e| e.size).sum();
        let fees: u64 = entries.values().map(|e| e.fee).sum();
        Ok(serde_json::json!({ "count": count, "bytes": bytes, "total_fee": fees }))
    });

    let c = chain.clone();
    io.add_sync_method("getaddressinfo", move |p: Params| {
        let (addr,): (String,) = p.parse()
            .map_err(|_| jsonrpc_core::Error::invalid_params("need [address]"))?;
        let h20 = crate::wallet::address::decode_address(&addr)
            .map_err(|_| jsonrpc_core::Error::invalid_params("bad address"))?;
        let script = crate::wallet::address::script_p2pkh(&h20);
        let utxo = c.utxo.read();
        let mut balance = 0u64;
        let mut utxos = Vec::new();
        for (op, out) in utxo.iter() {
            if out.script_pubkey == script {
                balance = balance.saturating_add(out.value);
                utxos.push(serde_json::json!({
                    "txid": hash_to_hex(&op.txid),
                    "vout": op.vout,
                    "value": out.value,
                }));
            }
        }
        Ok(serde_json::json!({
            "address": addr,
            "balance": balance,
            "utxo_count": utxos.len(),
            "utxos": utxos,
        }))
    });

    let c = chain.clone();
    io.add_sync_method("getrichlist", move |p: Params| {
        let limit = p.parse::<(u64,)>().map(|(n,)| n as usize).unwrap_or(100).min(1000);
        use std::collections::HashMap;
        let utxo = c.utxo.read();
        let mut by_script: HashMap<Vec<u8>, u64> = HashMap::new();
        for out in utxo.values() {
            if out.script_pubkey.first() == Some(&0x6a) { continue; }
            *by_script.entry(out.script_pubkey.clone()).or_insert(0) += out.value;
        }
        let mut rows: Vec<(Vec<u8>, u64)> = by_script.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        rows.truncate(limit);
        let supply = *c.supply.read();
        let list: Vec<Value> = rows.iter().map(|(script, bal)| {
            serde_json::json!({
                "address": script_to_address(script),
                "balance": bal,
                "share": if supply > 0 { *bal as f64 / supply as f64 } else { 0.0 },
            })
        }).collect();
        Ok(Value::Array(list))
    });

    let token = rpc_token();
    ServerBuilder::new(io)
        .threads(2)
        .request_middleware(AuthMiddleware { token })
        .start_http(&{
            let bind = std::env::var("THOCOIN_RPC_BIND").unwrap_or_else(|_| "127.0.0.1".into());
            format!("{}:{}", bind, RPC_PORT).parse().unwrap()
        })
        .expect("RPC start failed")
}

fn hex_to_hash(s: &str) -> Option<crate::core::hash::Hash> {
    let mut bytes = hex::decode(s).ok()?;
    if bytes.len() != 32 { return None; }
    bytes.reverse();
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    Some(h)
}

fn script_to_address(script: &[u8]) -> String {
    // P2PKH: OP_DUP OP_HASH160 <20> OP_EQUALVERIFY OP_CHECKSIG
    if script.len() == 25 && script[0] == 0x76 && script[1] == 0xa9 && script[2] == 0x14 {
        let mut h = [0u8; 20];
        h.copy_from_slice(&script[3..23]);
        crate::wallet::address::encode_address(&h)
    } else {
        format!("script:{}", hex::encode(script))
    }
}

fn tx_to_json(tx: &crate::core::tx::Transaction, height: Option<u64>, tip: u64) -> Value {
    use crate::core::hash::hash_to_hex;
    let confirmations = height.map(|h| tip.saturating_sub(h) + 1).unwrap_or(0);
    let vin: Vec<Value> = tx.inputs.iter().map(|i| {
        serde_json::json!({
            "txid": hash_to_hex(&i.prev.txid),
            "vout": i.prev.vout,
            "pubkey_len": i.pubkey.len(),
            "sig_len": i.signature.len(),
        })
    }).collect();
    let vout: Vec<Value> = tx.outputs.iter().enumerate().map(|(n, o)| {
        serde_json::json!({
            "n": n,
            "value": o.value,
            "address": script_to_address(&o.script_pubkey),
            "script_hex": hex::encode(&o.script_pubkey),
        })
    }).collect();
    let total_out: u64 = tx.outputs.iter().map(|o| o.value).sum();
    serde_json::json!({
        "txid": hash_to_hex(&tx.txid()),
        "wtxid": hash_to_hex(&tx.wtxid()),
        "version": tx.version,
        "lock_time": tx.lock_time,
        "size": tx.size(),
        "is_coinbase": tx.is_coinbase(),
        "block_height": height,
        "confirmations": confirmations,
        "total_out": total_out,
        "vin": vin,
        "vout": vout,
    })
}

fn block_to_json(b: &crate::core::block::Block, chain: &ChainState) -> Value {
    use crate::core::hash::hash_to_hex;
    let hash = b.hash();
    let height = chain.headers.read().get(&hash).map(|(_, h)| *h);
    let tip = *chain.height.read();
    let confirmations = height.map(|h| tip.saturating_sub(h) + 1).unwrap_or(0);
    let size = bincode::serialize(b).map(|v| v.len()).unwrap_or(0);
    let reward = b.transactions.first()
        .and_then(|cb| cb.outputs.first()).map(|o| o.value).unwrap_or(0);
    let miner = b.transactions.first()
        .and_then(|cb| cb.outputs.first())
        .map(|o| script_to_address(&o.script_pubkey)).unwrap_or_default();
    let txs: Vec<Value> = b.transactions.iter()
        .map(|t| tx_to_json(t, height, tip)).collect();
    serde_json::json!({
        "hash": hash_to_hex(&hash),
        "height": height,
        "confirmations": confirmations,
        "version": b.header.version,
        "prev_hash": hash_to_hex(&b.header.prev_hash),
        "merkle_root": hash_to_hex(&b.header.merkle_root),
        "timestamp": b.header.timestamp,
        "bits": format!("{:08x}", b.header.bits),
        "nonce": b.header.nonce,
        "size": size,
        "tx_count": b.transactions.len(),
        "reward": reward,
        "miner": miner,
        "tx": txs,
    })
}

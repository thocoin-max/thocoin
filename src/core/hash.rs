use sha2::{Sha256, Digest};
use ripemd::Ripemd160;

pub type Hash = [u8; 32];

pub fn sha256d(data: &[u8]) -> Hash {
    let h1 = Sha256::digest(data);
    let h2 = Sha256::digest(&h1);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h2);
    out
}

pub fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let rip = Ripemd160::digest(&sha);
    let mut out = [0u8; 20];
    out.copy_from_slice(&rip);
    out
}

pub fn hash_to_hex(h: &Hash) -> String {
    let mut rev = *h;
    rev.reverse();
    hex::encode(rev)
}

pub fn bits_to_target(bits: u32) -> [u8; 32] {
    let exp = (bits >> 24) as usize;
    let mant = bits & 0x007fffff;
    let mut target = [0u8; 32];
    if exp <= 3 {
        let m = mant >> (8 * (3 - exp));
        target[29] = (m >> 16) as u8;
        target[30] = (m >> 8) as u8;
        target[31] = m as u8;
    } else if exp <= 32 {
        let off = 32 - exp;
        if off + 2 < 32 {
            target[off] = (mant >> 16) as u8;
            target[off + 1] = (mant >> 8) as u8;
            target[off + 2] = mant as u8;
        }
    }
    target
}

pub fn target_to_bits(target: &[u8; 32]) -> u32 {
    let mut i = 0;
    while i < 32 && target[i] == 0 { i += 1; }
    if i == 32 { return 0; }
    let size = (32 - i) as u32;
    let b0 = target[i] as u32;
    let b1 = if i + 1 < 32 { target[i + 1] as u32 } else { 0 };
    let b2 = if i + 2 < 32 { target[i + 2] as u32 } else { 0 };
    let mut mant = (b0 << 16) | (b1 << 8) | b2;
    let mut exp = size;
    if mant & 0x0080_0000 != 0 { mant >>= 8; exp += 1; }
    (exp << 24) | (mant & 0x007f_ffff)
}

pub fn target_cmp(a: &[u8; 32], b: &[u8; 32]) -> std::cmp::Ordering {
    for i in 0..32 {
        if a[i] != b[i] { return a[i].cmp(&b[i]); }
    }
    std::cmp::Ordering::Equal
}

pub fn target_add(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry: u16 = 0;
    for i in (0..32).rev() {
        let s = a[i] as u16 + b[i] as u16 + carry;
        out[i] = s as u8;
        carry = s >> 8;
    }
    if carry != 0 { return [0xFFu8; 32]; }
    out
}

pub fn target_mul_div(target: &[u8; 32], num: u64, den: u64) -> [u8; 32] {
    if den == 0 { return [0xFFu8; 32]; }
    let mut wide = [0u8; 40];
    let mut carry: u128 = 0;
    for i in (0..32).rev() {
        let prod = target[i] as u128 * num as u128 + carry;
        wide[i + 8] = (prod & 0xff) as u8;
        carry = prod >> 8;
    }
    for j in (0..8).rev() {
        wide[j] = (carry & 0xff) as u8;
        carry >>= 8;
    }
    let mut rem: u128 = 0;
    let mut quot = [0u8; 40];
    for i in 0..40 {
        let cur = (rem << 8) | wide[i] as u128;
        quot[i] = (cur / den as u128) as u8;
        rem = cur % den as u128;
    }
    if quot[..8].iter().any(|&b| b != 0) { return [0xFFu8; 32]; }
    let mut out = [0u8; 32];
    out.copy_from_slice(&quot[8..40]);
    out
}

fn u256_not(a: &[u8; 32]) -> [u8; 32] {
    let mut o = [0u8; 32];
    for i in 0..32 { o[i] = !a[i]; }
    o
}

fn u256_inc(a: &[u8; 32]) -> [u8; 32] {
    let mut o = *a;
    for i in (0..32).rev() {
        let (v, c) = o[i].overflowing_add(1);
        o[i] = v;
        if !c { return o; }
    }
    o
}

fn u256_shl1(a: &mut [u8; 32]) {
    let mut carry = 0u8;
    for i in (0..32).rev() {
        let nc = a[i] >> 7;
        a[i] = (a[i] << 1) | carry;
        carry = nc;
    }
}

fn u256_sub(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut o = [0u8; 32];
    let mut borrow: i16 = 0;
    for i in (0..32).rev() {
        let d = a[i] as i16 - b[i] as i16 - borrow;
        if d < 0 { o[i] = (d + 256) as u8; borrow = 1; }
        else { o[i] = d as u8; borrow = 0; }
    }
    o
}

fn u256_div(num: &[u8; 32], den: &[u8; 32]) -> [u8; 32] {
    if target_cmp(den, &[0u8; 32]) == std::cmp::Ordering::Equal {
        return [0xFFu8; 32];
    }
    let mut quot = [0u8; 32];
    let mut rem = [0u8; 32];
    for bit in 0..256 {
        u256_shl1(&mut rem);
        let byte = bit / 8;
        let mask = 0x80u8 >> (bit % 8);
        if num[byte] & mask != 0 { rem[31] |= 1; }
        if target_cmp(&rem, den) != std::cmp::Ordering::Less {
            rem = u256_sub(&rem, den);
            quot[byte] |= mask;
        }
    }
    quot
}

pub fn work_from_bits(bits: u32) -> [u8; 32] {
    let target = bits_to_target(bits);
    if target_cmp(&target, &[0u8; 32]) == std::cmp::Ordering::Equal {
        return [0xFFu8; 32];
    }
    let denom = u256_inc(&target);
    let num = u256_not(&target);
    let q = u256_div(&num, &denom);
    u256_inc(&q)
}

pub fn target_add_bits(work: &[u8; 32], bits: u32) -> [u8; 32] {
    target_add(work, &work_from_bits(bits))
}

pub fn hash_meets_target(hash: &Hash, bits: u32) -> bool {
    let target = bits_to_target(bits);
    let mut rev = *hash;
    rev.reverse();
    for i in 0..32 {
        if rev[i] < target[i] { return true; }
        if rev[i] > target[i] { return false; }
    }
    true
}

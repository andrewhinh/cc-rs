//! Open-addressing string hash table. `hashmap_put` / `hashmap_put2` store the
//! key pointer; callers must keep key storage alive for the map's lifetime.
//! Keys are byte slices (chibicc `tok->loc` + `tok->len`), not Rust char
//! indices.

use std::ffi::c_void;
use std::io::{self, Write};
use std::ptr;

const INIT_SIZE: i32 = 16;
const HIGH_WATERMARK: i32 = 70;
const LOW_WATERMARK: i32 = 50;
const TOMBSTONE: *const u8 = usize::MAX as *const u8;

#[derive(Debug, Clone)]
pub struct HashEntry {
    pub key: *const u8,
    pub keylen: i32,
    pub val: *mut c_void,
}

impl Default for HashEntry {
    fn default() -> Self {
        Self {
            key: ptr::null(),
            keylen: 0,
            val: ptr::null_mut(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HashMap {
    pub buckets: Vec<HashEntry>,
    pub capacity: i32,
    pub used: i32,
}

fn fnv_hash(key: *const u8, keylen: i32) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for i in 0..keylen {
        hash = hash.wrapping_mul(0x100000001b3);
        hash ^= unsafe { *key.add(i as usize) } as u64;
    }
    hash
}

fn occupied(key: *const u8) -> bool {
    !key.is_null() && key != TOMBSTONE
}

fn match_entry(ent: &HashEntry, key: *const u8, keylen: i32) -> bool {
    occupied(ent.key)
        && ent.keylen == keylen
        && unsafe { std::slice::from_raw_parts(key, keylen as usize) }
            == unsafe { std::slice::from_raw_parts(ent.key, keylen as usize) }
}

fn rehash(map: &mut HashMap) {
    let mut nkeys = 0;
    for ent in &map.buckets {
        if occupied(ent.key) {
            nkeys += 1;
        }
    }

    let mut cap = map.capacity;
    while (nkeys * 100) / cap >= LOW_WATERMARK {
        cap *= 2;
    }
    assert!(cap > 0);

    let mut map2 = HashMap {
        buckets: vec![HashEntry::default(); cap as usize],
        capacity: cap,
        used: 0,
    };

    for ent in map.buckets.drain(..) {
        if occupied(ent.key) {
            hashmap_put2(&mut map2, ent.key, ent.keylen, ent.val);
        }
    }

    assert_eq!(map2.used, nkeys);
    *map = map2;
}

fn probe_index(hash: u64, i: i32, capacity: i32) -> usize {
    (hash.wrapping_add(i as u64) % capacity as u64) as usize
}

fn get_entry(map: &HashMap, key: *const u8, keylen: i32) -> Option<usize> {
    if map.buckets.is_empty() {
        return None;
    }

    let hash = fnv_hash(key, keylen);
    for i in 0..map.capacity {
        let idx = probe_index(hash, i, map.capacity);
        let ent = &map.buckets[idx];
        if match_entry(ent, key, keylen) {
            return Some(idx);
        }
        if ent.key.is_null() {
            return None;
        }
    }
    unreachable!();
}

fn get_or_insert_entry(map: &mut HashMap, key: *const u8, keylen: i32) -> usize {
    if map.buckets.is_empty() {
        map.buckets = vec![HashEntry::default(); INIT_SIZE as usize];
        map.capacity = INIT_SIZE;
    } else if (map.used * 100) / map.capacity >= HIGH_WATERMARK {
        rehash(map);
    }

    let hash = fnv_hash(key, keylen);
    for i in 0..map.capacity {
        let idx = probe_index(hash, i, map.capacity);
        let ent = &map.buckets[idx];

        if match_entry(ent, key, keylen) {
            return idx;
        }

        if ent.key == TOMBSTONE {
            map.buckets[idx].key = key;
            map.buckets[idx].keylen = keylen;
            return idx;
        }

        if ent.key.is_null() {
            map.buckets[idx].key = key;
            map.buckets[idx].keylen = keylen;
            map.used += 1;
            return idx;
        }
    }
    unreachable!();
}

pub fn hashmap_get2(map: &HashMap, key: *const u8, keylen: i32) -> *mut c_void {
    get_entry(map, key, keylen)
        .map(|idx| map.buckets[idx].val)
        .unwrap_or(ptr::null_mut())
}

pub fn hashmap_get_bytes(map: &HashMap, key: &[u8]) -> *mut c_void {
    hashmap_get2(map, key.as_ptr(), key.len() as i32)
}

pub fn hashmap_get(map: &HashMap, key: &str) -> *mut c_void {
    hashmap_get_bytes(map, key.as_bytes())
}

pub fn hashmap_put2(map: &mut HashMap, key: *const u8, keylen: i32, val: *mut c_void) {
    let idx = get_or_insert_entry(map, key, keylen);
    map.buckets[idx].val = val;
}

pub fn hashmap_put_bytes(map: &mut HashMap, key: &[u8], val: *mut c_void) {
    hashmap_put2(map, key.as_ptr(), key.len() as i32, val);
}

pub fn hashmap_put(map: &mut HashMap, key: &str, val: *mut c_void) {
    hashmap_put_bytes(map, key.as_bytes(), val);
}

pub fn populate_keywords(map: &mut HashMap, keywords: &[&str]) {
    if map.capacity != 0 {
        return;
    }
    for kw in keywords {
        hashmap_put(map, kw, std::ptr::dangling_mut::<c_void>());
    }
}

pub fn hashmap_contains_bytes(map: &HashMap, key: &[u8]) -> bool {
    !hashmap_get_bytes(map, key).is_null()
}

pub fn hashmap_delete2(map: &mut HashMap, key: *const u8, keylen: i32) {
    if let Some(idx) = get_entry(map, key, keylen) {
        map.buckets[idx].key = TOMBSTONE;
    }
}

pub fn hashmap_delete_bytes(map: &mut HashMap, key: &[u8]) {
    hashmap_delete2(map, key.as_ptr(), key.len() as i32);
}

pub fn hashmap_delete(map: &mut HashMap, key: &str) {
    hashmap_delete_bytes(map, key.as_bytes());
}

fn leak_key(s: String) -> (*const u8, i32) {
    let bytes = s.into_bytes();
    let keylen = bytes.len() as i32;
    let ptr = Box::leak(bytes.into_boxed_slice()).as_ptr();
    (ptr, keylen)
}

fn run_hashmap_stress() {
    let mut map = HashMap::default();

    for i in 0..5000 {
        let (key, keylen) = leak_key(format!("key {i}"));
        hashmap_put2(&mut map, key, keylen, i as *mut c_void);
    }
    for i in 1000..2000 {
        hashmap_delete(&mut map, &format!("key {i}"));
    }
    for i in 1500..1600 {
        let (key, keylen) = leak_key(format!("key {i}"));
        hashmap_put2(&mut map, key, keylen, i as *mut c_void);
    }
    for i in 6000..7000 {
        let (key, keylen) = leak_key(format!("key {i}"));
        hashmap_put2(&mut map, key, keylen, i as *mut c_void);
    }

    for i in 0..1000 {
        assert_eq!(hashmap_get(&map, &format!("key {i}")) as usize, i);
    }
    for _ in 1000..1500 {
        assert!(hashmap_get(&map, "no such key").is_null());
    }
    for i in 1500..1600 {
        assert_eq!(hashmap_get(&map, &format!("key {i}")) as usize, i);
    }
    for _ in 1600..2000 {
        assert!(hashmap_get(&map, "no such key").is_null());
    }
    for i in 2000..5000 {
        assert_eq!(hashmap_get(&map, &format!("key {i}")) as usize, i);
    }
    for _ in 5000..6000 {
        assert!(hashmap_get(&map, "no such key").is_null());
    }
    for i in 6000..7000 {
        let (key, keylen) = leak_key(format!("key {i}"));
        hashmap_put2(&mut map, key, keylen, i as *mut c_void);
    }

    assert!(hashmap_get(&map, "no such key").is_null());
}

pub fn hashmap_test() {
    run_hashmap_stress();
    io::stdout().write_all(b"OK\n").unwrap();
}

#[cfg(test)]
mod tests {
    use super::run_hashmap_stress;

    #[test]
    fn hashmap_stress() {
        run_hashmap_stress();
    }
}

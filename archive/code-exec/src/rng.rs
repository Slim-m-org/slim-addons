//! A deterministic, import-free `getrandom` backend.
//!
//! The host ABI forbids any wasm import, including the ones the `js`/`wasm_js`
//! getrandom backends would pull in. `boa_engine` only needs randomness for
//! `Math.random`, which does not need to be cryptographically secure here, so
//! a splitmix64 counter seeded from a fixed constant is enough. This is wired
//! in via `.cargo/config.toml` setting `--cfg getrandom_backend="custom"`.

use core::sync::atomic::{AtomicU64, Ordering};

use getrandom::Error;

static STATE: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);

fn next_u64() -> u64 {
    let mut z = STATE
        .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[no_mangle]
extern "Rust" fn __getrandom_v03_custom(dest: *mut u8, len: usize) -> Result<(), Error> {
    let buf = unsafe { core::slice::from_raw_parts_mut(dest, len) };
    let mut filled = 0;
    while filled < buf.len() {
        let bytes = next_u64().to_le_bytes();
        let take = core::cmp::min(8, buf.len() - filled);
        buf[filled..filled + take].copy_from_slice(&bytes[..take]);
        filled += take;
    }
    Ok(())
}

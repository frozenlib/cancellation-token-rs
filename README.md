# cancellation-token-rs

[![Crates.io](https://img.shields.io/crates/v/cancellation-token.svg)](https://crates.io/crates/cancellation-token)
[![Docs.rs](https://docs.rs/cancellation-token/badge.svg)](https://docs.rs/cancellation-token/)
[![Actions Status](https://github.com/frozenlib/cancellation-token-rs/workflows/CI/badge.svg)](https://github.com/frozenlib/cancellation-token-rs/actions)

A Rust implementation of C#'s CancellationToken API.

## Example

```rust
use cancellation_token::{CancellationToken, CancellationTokenSource, MayBeCanceled};

fn cancelable_function(ct: &CancellationToken) -> MayBeCanceled<u32> {
    for _ in 0..100 {
        ct.canceled()?; // Return from this function if canceled
        heavy_work();
    }
    Ok(100)
}
fn heavy_work() { }

fn main() {
    let cts = CancellationTokenSource::new();
    std::thread::scope(|s| {
        s.spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            cts.cancel();
        });
        if cancelable_function(&cts.token()).is_err() {
            println!("canceled");
        }
    });
}
```
 
## License

This project is dual licensed under Apache-2.0/MIT. See the two LICENSE-* files for details.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

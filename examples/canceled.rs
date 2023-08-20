use cancellation_token::{CancellationToken, CancellationTokenSource, MayBeCanceled};

fn cancelable_function(ct: &CancellationToken) -> MayBeCanceled<u32> {
    for _ in 0..100 {
        ct.canceled()?; // Return from this function if canceled
        heavy_work();
    }
    Ok(100)
}
fn heavy_work() {}

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
